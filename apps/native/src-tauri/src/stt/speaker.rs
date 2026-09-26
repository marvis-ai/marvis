//! Live speaker diarization: per-segment speaker embeddings clustered online.
//!
//! Each provider worker owns one [`SpeakerTracker`] for its channel, so
//! clustering spaces are channel-scoped — a mic voice and a system-audio
//! voice can never merge into one label. When the embedding model is not
//! installed, [`tracker_if_installed`] returns `None` and events simply
//! carry no `speaker_idx`.

use std::path::Path;

use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};

use super::SpeakerChannel;
use crate::paths;

const SAMPLE_RATE: i32 = 16_000;
/// Cosine-similarity gate for assigning a segment to a known speaker.
/// Calibrated on the 3dspeaker-campplus model: clean studio speech scores
/// same-speaker ≈ 0.94, but real 3-second mic windows are far noisier —
/// a gate near 0.7 fragments one person into many speakers, while
/// distinct voices rarely exceed ~0.4. Operate low and let the
/// candidate-confirmation rule absorb drift.
const MATCH_THRESHOLD: f32 = 0.45;
/// Confident matches re-enroll into the cluster's reference set so the
/// label stays stable as a voice drifts across a long meeting. Above the
/// match floor — borderline matches attach but must not pull a cluster's
/// references toward a similar-but-different voice.
const ENROLL_THRESHOLD: f32 = 0.6;
/// Per-speaker reference cap — bounds memory and stale-vector accumulation.
const MAX_REFERENCES: usize = 8;
/// Unmatched embeddings awaiting confirmation — capped, oldest evicted.
const MAX_CANDIDATES: usize = 4;
/// Segments shorter than this give unreliable embeddings — left unlabeled
/// rather than mis-assigned.
const MIN_SEGMENT_SECONDS: f32 = 0.6;
/// Per-sample amplitude floor for the voiced-fraction estimate (f32
/// domain, ≈ 26dB below full scale).
const VOICED_SAMPLE_FLOOR: f32 = 0.02;
/// A segment whose samples are mostly below [`VOICED_SAMPLE_FLOOR`] is a
/// pause with a word in it — its embedding is noise, so it stays
/// unlabeled. Whisper's fixed windows can be mostly silence even when
/// they still transcribe a fragment.
const MIN_VOICED_FRACTION: f32 = 0.3;

/// Create a tracker when the speaker-embedding model is installed.
/// On the `Me` channel an enrolled voiceprint pre-seeds cluster 0, so
/// "You" is a verified identity rather than just the first voice heard.
pub fn tracker_if_installed(channel: SpeakerChannel) -> Option<SpeakerTracker> {
    let path = crate::sherpa_models::speaker_embedding_model_path(&paths::sherpa_models_dir())?;
    let mut tracker = SpeakerTracker::create(&path);
    if tracker.is_none() {
        log::warn!("speaker embedding model failed to load; diarization disabled");
    }
    if channel == SpeakerChannel::Me {
        if let (Some(t), Some(voiceprint)) = (tracker.as_mut(), crate::voiceprint::load()) {
            t.seed(voiceprint);
        }
    }
    tracker
}

/// Assigns embeddings to speaker clusters by max cosine similarity against
/// each speaker's reference set. Kept free of FFI so the clustering rule is
/// fully unit-testable.
///
/// A first-time unmatched embedding never spawns a cluster — it is held as
/// a candidate and the segment reports unlabeled. Only a second segment
/// similar to a held candidate confirms the new speaker. One-off noisy
/// windows (pauses, laughter, half-silent whisper chunks) therefore can't
/// create single-use "speakers", while a real new voice is confirmed one
/// segment later.
pub struct SpeakerClusters {
    references: Vec<Vec<Vec<f32>>>,
    candidates: Vec<Vec<f32>>,
    /// Index of the voiceprint-seeded cluster (`Some(0)` when enrolled).
    /// A seeded cluster re-enrolls every match — the voiceprint anchors
    /// the user's identity, so borderline matches are language/noise
    /// drift to learn from, not strangers to keep out. Measured on this
    /// model: the same voice across zh→en scores ≈ 0.56 — above MATCH but
    /// below ENROLL — so without this, an English-enrolled user speaking
    /// Chinese (or vice versa) would keep hitting the candidate gate.
    seeded: Option<usize>,
}

impl Default for SpeakerClusters {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeakerClusters {
    pub fn new() -> Self {
        Self {
            references: Vec::new(),
            candidates: Vec::new(),
            seeded: None,
        }
    }

    /// Born with cluster 0 already formed by an enrolled voiceprint.
    pub(crate) fn seed(&mut self, embedding: Vec<f32>) {
        if self.references.is_empty() {
            self.references.push(vec![embedding]);
            self.seeded = Some(0);
        }
    }

    /// The speaker index for `embedding` — an established cluster above
    /// [`MATCH_THRESHOLD`], a newly confirmed speaker, or `None` while the
    /// segment is an unconfirmed candidate.
    pub fn assign(&mut self, embedding: &[f32]) -> Option<u32> {
        if self.references.is_empty() {
            self.references.push(vec![embedding.to_vec()]);
            return Some(0);
        }
        let mut best: Option<(usize, f32)> = None;
        for (idx, references) in self.references.iter().enumerate() {
            let score = references
                .iter()
                .map(|reference| cosine(embedding, reference))
                .fold(0.0_f32, f32::max);
            if best.is_none_or(|(_, best_score)| score > best_score) {
                best = Some((idx, score));
            }
        }
        let (best_idx, best_score) = best.expect("non-empty references");
        if best_score >= MATCH_THRESHOLD {
            // Confident repeats enrich the reference set — a slow drift
            // keeps matching without letting marginal matches pile up.
            // The seeded cluster trusts every match (see `seeded`).
            let enroll_gate = if self.seeded == Some(best_idx) {
                MATCH_THRESHOLD
            } else {
                ENROLL_THRESHOLD
            };
            if best_score >= enroll_gate && self.references[best_idx].len() < MAX_REFERENCES {
                self.references[best_idx].push(embedding.to_vec());
            }
            return Some(best_idx as u32);
        }
        // Unmatched against every speaker — a repeat of a held candidate
        // confirms a genuinely new voice.
        if let Some(position) = self
            .candidates
            .iter()
            .position(|candidate| cosine(embedding, candidate) >= MATCH_THRESHOLD)
        {
            let seed = self.candidates.remove(position);
            self.references.push(vec![seed, embedding.to_vec()]);
            return Some((self.references.len() - 1) as u32);
        }
        if self.candidates.len() == MAX_CANDIDATES {
            self.candidates.remove(0);
        }
        self.candidates.push(embedding.to_vec());
        None
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = (na * nb).sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

/// Embedding extractor + cluster state for one audio channel.
pub struct SpeakerTracker {
    extractor: SpeakerEmbeddingExtractor,
    clusters: SpeakerClusters,
}

impl SpeakerTracker {
    /// `None` when the model can't be loaded — callers treat diarization as
    /// unavailable rather than failing the session.
    pub fn create(model: &Path) -> Option<Self> {
        let config = SpeakerEmbeddingExtractorConfig {
            model: Some(model.display().to_string()),
            num_threads: 1,
            ..SpeakerEmbeddingExtractorConfig::default()
        };
        let extractor = SpeakerEmbeddingExtractor::create(&config)?;
        Some(Self {
            extractor,
            clusters: SpeakerClusters::new(),
        })
    }

    /// The cluster index for this segment; `None` when the segment is too
    /// short, mostly silence, or embedding extraction fails.
    pub fn assign(&mut self, samples: &[f32]) -> Option<u32> {
        let embedding = self.embed(samples)?;
        self.clusters.assign(&embedding)
    }

    /// The speaker embedding for `samples`, or `None` when the audio is
    /// too short, mostly silence, or extraction fails. Shared with voice
    /// enrollment, which embeds a recording without touching clusters.
    pub fn embed(&self, samples: &[f32]) -> Option<Vec<f32>> {
        if samples.len() < (SAMPLE_RATE as f32 * MIN_SEGMENT_SECONDS) as usize {
            return None;
        }
        if voiced_fraction(samples) < MIN_VOICED_FRACTION {
            return None;
        }
        let stream = self.extractor.create_stream()?;
        stream.accept_waveform(SAMPLE_RATE, samples);
        if !self.extractor.is_ready(&stream) {
            return None;
        }
        self.extractor.compute(&stream)
    }

    /// Pre-seed cluster 0 with an enrolled voiceprint — the first real
    /// utterance then matches-or-rejects against the user's own voice.
    /// A dimension mismatch (e.g. enrolled before a model swap) is ignored:
    /// cosine over a truncated prefix would produce junk scores.
    pub fn seed(&mut self, embedding: Vec<f32>) {
        if embedding.len() != self.extractor.dim() as usize {
            log::warn!(
                "voiceprint dimension {} does not match model {}; ignoring",
                embedding.len(),
                self.extractor.dim()
            );
            return;
        }
        self.clusters.seed(embedding);
    }
}

/// The fraction of samples above the amplitude floor — a mostly-silent
/// segment embeds as noise, so both tracking and enrollment gate on it.
pub(crate) fn voiced_fraction(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let voiced = samples
        .iter()
        .filter(|sample| sample.abs() > VOICED_SAMPLE_FLOOR)
        .count();
    voiced as f32 / samples.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec_at(index: usize, dim: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; dim];
        v[index % dim] = 1.0;
        v
    }

    #[test]
    fn distinct_voices_need_confirmation_before_forming_a_cluster() {
        let mut clusters = SpeakerClusters::new();
        assert_eq!(clusters.assign(&vec_at(0, 8)), Some(0));
        // An orthogonal voice's first sighting is unlabeled — one noisy
        // segment can't mint a speaker. A second sighting confirms it.
        assert_eq!(clusters.assign(&vec_at(1, 8)), None);
        assert_eq!(clusters.assign(&vec_at(1, 8)), Some(1));
        // Established clusters keep assigning directly.
        assert_eq!(clusters.assign(&vec_at(0, 8)), Some(0));
        assert_eq!(clusters.assign(&vec_at(1, 8)), Some(1));
    }

    /// The user's real failure mode: one person, many "speakers". Noisy
    /// one-off segments must stay unlabeled instead of fragmenting.
    #[test]
    fn one_off_junk_embeddings_never_spawn_speakers() {
        let mut clusters = SpeakerClusters::new();
        assert_eq!(clusters.assign(&vec_at(0, 8)), Some(0));
        // Orthogonal outliers — pauses/noise — report unlabeled…
        assert_eq!(clusters.assign(&vec_at(1, 8)), None);
        assert_eq!(clusters.assign(&vec_at(2, 8)), None);
        assert_eq!(clusters.assign(&vec_at(3, 8)), None);
        assert_eq!(clusters.assign(&vec_at(4, 8)), None);
        // …and the original voice still owns the transcript.
        assert_eq!(clusters.assign(&vec_at(0, 8)), Some(0));
        assert_eq!(clusters.references.len(), 1);
    }

    #[test]
    fn similar_embeddings_stay_in_one_cluster() {
        let mut clusters = SpeakerClusters::new();
        let seed = vec![1.0, 0.1, 0.0, 0.0];
        let drifted = vec![0.9, 0.15, 0.1, 0.0];
        assert_eq!(clusters.assign(&seed), Some(0));
        // cos(drifted, seed) ≈ 0.96 → same speaker, and it re-enrolls.
        assert_eq!(clusters.assign(&drifted), Some(0));
        assert_eq!(clusters.references[0].len(), 2);
    }

    #[test]
    fn borderline_matches_do_not_enroll() {
        let mut clusters = SpeakerClusters::new();
        clusters.assign(&vec![1.0, 0.0]);
        // cos ≈ 0.58: above MATCH (0.45), below ENROLL (0.6).
        let near = vec![0.58, 0.81];
        assert_eq!(clusters.assign(&near), Some(0));
        assert_eq!(clusters.references[0].len(), 1);
    }

    /// The seeded "You" cluster trusts every match: a borderline 0.58
    /// score — where an unseeded cluster refuses to enroll — still joins
    /// the reference set, so cross-language drift enriches instead of
    /// spawning guests.
    #[test]
    fn seeded_cluster_reenrolls_borderline_matches() {
        let mut clusters = SpeakerClusters::new();
        clusters.seed(vec![1.0, 0.0]);
        // cos ≈ 0.58: above MATCH (0.45), below ENROLL (0.6).
        assert_eq!(clusters.assign(&vec![0.58, 0.81]), Some(0));
        assert_eq!(clusters.references[0].len(), 2);
    }

    #[test]
    fn zero_embedding_never_crashes_assignment() {
        let mut clusters = SpeakerClusters::new();
        assert_eq!(clusters.assign(&vec![0.0, 0.0]), Some(0));
        assert_eq!(clusters.assign(&vec![1.0, 0.0]), None);
    }

    // Temporary real-model smoke check — run explicitly:
    //   cargo test stt::speaker::tests::real_model_separates_voices -- --ignored
    // Needs the speaker-id model + /tmp/speaker_{a,a2,b,c}.wav f32@16k files.
    #[test]
    #[ignore]
    fn real_model_separates_voices() {
        fn wav_f32(path: &str) -> Vec<f32> {
            let bytes = std::fs::read(path).expect(path);
            let data_at = bytes
                .windows(4)
                .position(|w| w == b"data")
                .map(|p| p + 8)
                .expect("data chunk");
            bytes[data_at..]
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect()
        }
        let root = crate::paths::sherpa_models_dir();
        let model = crate::sherpa_models::speaker_embedding_model_path(&root)
            .expect("speaker-id model installed");
        let mut tracker = SpeakerTracker::create(&model).expect("extractor loads");
        // A new voice needs a second similar segment to confirm, so each
        // distinct voice is fed twice (embeddings are deterministic).
        let a = tracker.assign(&wav_f32("/tmp/speaker_a.wav"));
        let a2 = tracker.assign(&wav_f32("/tmp/speaker_a2.wav"));
        let b = tracker.assign(&wav_f32("/tmp/speaker_b.wav"));
        let b2 = tracker.assign(&wav_f32("/tmp/speaker_b.wav"));
        let c = tracker.assign(&wav_f32("/tmp/speaker_c.wav"));
        let c2 = tracker.assign(&wav_f32("/tmp/speaker_c.wav"));
        eprintln!("a={a:?} a2={a2:?} b={b:?}/{b2:?} c={c:?}/{c2:?}");
        assert_eq!(a, Some(0));
        assert_eq!(a2, Some(0));
        assert_eq!(b, None);
        assert_eq!(b2, Some(1));
        // Karen-vs-Samantha cosine is 0.69 — above MATCH, so c merges into
        // cluster 1. Documented limit: engineered-neutral TTS voices are
        // unusually close; real same-gender speakers score lower.
        assert_eq!(c, Some(1));
        assert_eq!(c2, Some(1));
    }

    // Enrollment smoke check — run explicitly:
    //   cargo test stt::speaker::tests::enrolled_seed_anchors_the_same_voice -- --ignored
    // Needs the speaker-id model + /tmp/enroll_{a,a2,b}.wav f32@16k files:
    // a/a2 are the same voice on different sentences, b is a second voice.
    #[test]
    #[ignore]
    fn enrolled_seed_anchors_the_same_voice() {
        fn wav_f32(path: &str) -> Vec<f32> {
            let bytes = std::fs::read(path).expect(path);
            let data_at = bytes
                .windows(4)
                .position(|w| w == b"data")
                .map(|p| p + 8)
                .expect("data chunk");
            bytes[data_at..]
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect()
        }
        let root = crate::paths::sherpa_models_dir();
        let model = crate::sherpa_models::speaker_embedding_model_path(&root)
            .expect("speaker-id model installed");
        let mut tracker = SpeakerTracker::create(&model).expect("extractor loads");
        // Enroll from the first clip, then verify a DIFFERENT utterance by
        // the same voice still lands on cluster 0 — that's the promise.
        let voiceprint = tracker
            .embed(&wav_f32("/tmp/enroll_a.wav"))
            .expect("embeds");
        tracker.seed(voiceprint);
        let same_voice = tracker.assign(&wav_f32("/tmp/enroll_a2.wav"));
        let other = tracker.assign(&wav_f32("/tmp/enroll_b.wav"));
        let other2 = tracker.assign(&wav_f32("/tmp/enroll_b.wav"));
        eprintln!("same={same_voice:?} other={other:?}/{other2:?}");
        assert_eq!(same_voice, Some(0));
        assert_eq!(other, None);
        assert_eq!(other2, Some(1));
    }

    // Cross-language probe: does a zh-enrolled voiceprint match the same
    // voice speaking English? Tingting reads zh + English (same identity),
    // Daniel gives the same-language different-voice control.
    //   cargo test stt::speaker::tests::cross_language_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn cross_language_probe() {
        fn wav_f32(path: &str) -> Vec<f32> {
            let bytes = std::fs::read(path).expect(path);
            let data_at = bytes
                .windows(4)
                .position(|w| w == b"data")
                .map(|p| p + 8)
                .expect("data chunk");
            bytes[data_at..]
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect()
        }
        let root = crate::paths::sherpa_models_dir();
        let model = crate::sherpa_models::speaker_embedding_model_path(&root)
            .expect("speaker-id model installed");
        let tracker = SpeakerTracker::create(&model).expect("extractor loads");
        let tt_zh = tracker.embed(&wav_f32("/tmp/tt_zh.wav")).expect("tt_zh");
        let tt_en = tracker.embed(&wav_f32("/tmp/tt_en.wav")).expect("tt_en");
        let daniel_en = tracker
            .embed(&wav_f32("/tmp/daniel_en.wav"))
            .expect("daniel_en");
        let daniel_en2 = tracker
            .embed(&wav_f32("/tmp/daniel_en2.wav"))
            .expect("daniel_en2");
        eprintln!(
            "tt_zh·tt_en      = {:.3}  (same voice, zh→en)",
            cosine(&tt_zh, &tt_en)
        );
        eprintln!(
            "tt_zh·daniel_en  = {:.3}  (diff voice, en)",
            cosine(&tt_zh, &daniel_en)
        );
        eprintln!(
            "daniel_en·daniel_en2 = {:.3}  (same voice, en→en)",
            cosine(&daniel_en, &daniel_en2)
        );
        eprintln!("MATCH={MATCH_THRESHOLD} ENROLL={ENROLL_THRESHOLD}");

        // The regression this guards: a zh-enrolled cluster must accept the
        // same voice's English at ~0.56 AND enroll it — otherwise every
        // English window rides the candidate gate toward "Guest 1".
        let mut seeded = SpeakerTracker::create(&model).expect("extractor loads");
        seeded.seed(tt_zh.clone());
        let refs_before = seeded.clusters.references[0].len();
        assert_eq!(seeded.assign(&wav_f32("/tmp/tt_en.wav")), Some(0));
        assert!(seeded.clusters.references[0].len() > refs_before);
    }
}
