//! sherpa-onnx local STT provider: one app-wide recognizer engine fed by
//! per-provider Silero VAD workers.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use parking_lot::Mutex;
use sherpa_onnx::{
    OfflinePunctuation, OfflinePunctuationConfig, OfflineRecognizer, OfflineRecognizerConfig,
    OfflineSenseVoiceModelConfig, OnlinePunctuation, OnlinePunctuationConfig, SileroVadModelConfig,
    VadModelConfig, VoiceActivityDetector,
};

use crate::audio::PcmChunk;
use crate::sherpa_models::PunctuationPaths;

use super::{Finality, SpeakerChannel, SttProvider, TranscriptEvent};

const SAMPLE_RATE: i32 = 16_000;
const WORKER_TICK: Duration = Duration::from_millis(100);
const VAD_BUFFER_SECONDS: f32 = 30.0;
const INIT_TIMEOUT: Duration = Duration::from_secs(30);
const DECODE_TIMEOUT: Duration = Duration::from_secs(30);

struct DecodeJob {
    samples: Vec<f32>,
    reply: mpsc::Sender<Result<String, String>>,
}

/// One recognizer for the whole app — Listen runs two providers (me + them)
/// and a per-channel model would double the ~450MB working set.
pub struct SherpaEngine {
    jobs: mpsc::Sender<DecodeJob>,
    /// Kept only for `alive()` — never joined: the engine outlives providers.
    thread: JoinHandle<()>,
}

static ENGINE: Mutex<Option<(PathBuf, Arc<SherpaEngine>)>> = Mutex::new(None);

fn engine_for(model_dir: &Path) -> Result<Arc<SherpaEngine>, String> {
    engine_for_with(model_dir, SherpaEngine::spawn)
}

/// `spawn` is a seam so tests can stand up engines without model files.
fn engine_for_with(
    model_dir: &Path,
    spawn: impl FnOnce(&Path) -> Result<SherpaEngine, String>,
) -> Result<Arc<SherpaEngine>, String> {
    let mut slot = ENGINE.lock();
    if let Some((dir, engine)) = &*slot {
        // A cached engine whose decode thread died can never decode again —
        // fall through and respawn rather than hand out the dead Arc forever.
        if dir == model_dir && engine.alive() {
            return Ok(engine.clone());
        }
    }
    let engine = Arc::new(spawn(model_dir)?);
    *slot = Some((model_dir.to_path_buf(), engine.clone()));
    Ok(engine)
}

impl SherpaEngine {
    /// Spawn the engine thread. Recognizer creation runs on that thread and
    /// reports `Result<(), String>` back over an init channel so a missing or
    /// corrupt model fails `spawn` synchronously instead of poisoning jobs.
    fn spawn(model_dir: &Path) -> Result<Self, String> {
        let (jobs, rx) = mpsc::channel::<DecodeJob>();
        let (init, ready) = mpsc::channel::<Result<(), String>>();
        let dir = model_dir.to_path_buf();
        let thread = thread::spawn(move || {
            let recognizer = match create_recognizer(&dir) {
                Ok(recognizer) => {
                    let _ = init.send(Ok(()));
                    recognizer
                }
                Err(error) => {
                    let _ = init.send(Err(error));
                    return;
                }
            };
            while let Ok(job) = rx.recv() {
                let stream = recognizer.create_stream();
                stream.accept_waveform(SAMPLE_RATE, &job.samples);
                recognizer.decode(&stream);
                // A null/unparseable result means the engine is broken —
                // surface it as a decode error rather than silent text loss.
                let text = stream
                    .get_result()
                    .map(|result| result.text)
                    .ok_or_else(|| "the speech engine returned no result".to_string());
                let _ = job.reply.send(text);
            }
        });
        // Bounded wait: `engine_for` holds the ENGINE lock across spawn, so a
        // hung native load must not wedge every later start. On timeout the
        // thread exits on its own once creation returns (jobs sender dropped).
        match ready.recv_timeout(INIT_TIMEOUT) {
            Ok(result) => result.map(|()| Self { jobs, thread }),
            Err(_) => Err("the speech model could not be loaded".to_string()),
        }
    }

    /// `false` once the decode thread has exited — its job receiver is gone,
    /// so the engine can never decode again.
    fn alive(&self) -> bool {
        !self.thread.is_finished()
    }

    /// Hand one VAD segment to the engine thread and wait for its transcript.
    /// A dead engine (send or reply failure) is terminal for the worker.
    fn decode(&self, samples: Vec<f32>) -> Result<String, String> {
        let (reply, ready) = mpsc::channel();
        self.jobs
            .send(DecodeJob { samples, reply })
            .map_err(|_| "the speech engine is not running".to_string())?;
        ready
            .recv_timeout(DECODE_TIMEOUT)
            .map_err(|_| "the speech engine stopped responding".to_string())?
    }
}

fn create_recognizer(model_dir: &Path) -> Result<OfflineRecognizer, String> {
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model_dir.join("model.int8.onnx").display().to_string()),
        language: Some("auto".into()),
        use_itn: true,
    };
    config.model_config.tokens = Some(model_dir.join("tokens.txt").display().to_string());
    config.model_config.num_threads = 2;
    OfflineRecognizer::create(&config)
        .ok_or_else(|| "the speech model could not be loaded".to_string())
}

/// Load the optional English punctuation+casing model. Any failure yields
/// `None` — transcripts then pass through unmodified, as before.
fn create_punctuator(model: &Path, vocab: &Path) -> Option<OnlinePunctuation> {
    let mut config = OnlinePunctuationConfig::default();
    config.model.cnn_bilstm = Some(model.display().to_string());
    config.model.bpe_vocab = Some(vocab.display().to_string());
    OnlinePunctuation::create(&config)
}

/// Load the optional zh-en CT-Transformer punctuator — same
/// progressive-enhancement rule: any failure yields `None` and zh text
/// passes through unpunctuated.
fn create_punctuator_zh(model: &Path) -> Option<OfflinePunctuation> {
    let mut config = OfflinePunctuationConfig::default();
    config.model.ct_transformer = Some(model.display().to_string());
    OfflinePunctuation::create(&config)
}

/// Which punctuator gets a text — decided before any model call so the
/// rule stays unit-testable without ONNX files: any CJK ideograph → zh
/// (passthrough when the zh model is absent — mixed text must never fall
/// into the en lowercase path); else ASCII letters → en; else passthrough.
#[derive(Debug, PartialEq, Eq)]
enum PunctRoute {
    Zh,
    En,
    Pass,
}

fn punct_route(has_en: bool, has_zh: bool, text: &str) -> PunctRoute {
    if text.chars().any(is_cjk_ideograph) {
        return if has_zh {
            PunctRoute::Zh
        } else {
            PunctRoute::Pass
        };
    }
    if has_en && text.bytes().any(|b| b.is_ascii_alphabetic()) {
        return PunctRoute::En;
    }
    PunctRoute::Pass
}

/// SenseVoice emits English uppercased with no punctuation — and, in the
/// pinned 2025-09-09 export, `use_itn` is a no-op (upstream
/// k2-fsa/sherpa-onnx#2742) so Chinese arrives unpunctuated too.
/// CJK-bearing text routes to the zh-en CT-Transformer; Latin-only text
/// goes through the CNN-BiLSTM's case head (lowered first — it was
/// trained on lowercase input — and the model restores word case plus
/// `, . ?`). The CT-Transformer keeps input case, so it must never touch
/// the all-caps English path; anything else passes through untouched.
fn restore_punct_and_case(
    punct_en: Option<&OnlinePunctuation>,
    punct_zh: Option<&OfflinePunctuation>,
    text: &str,
) -> String {
    match punct_route(punct_en.is_some(), punct_zh.is_some(), text) {
        PunctRoute::Zh => punct_zh
            .expect("the Zh route implies punct_zh")
            .add_punctuation(text)
            .unwrap_or_else(|| text.to_string()),
        PunctRoute::En => {
            let lowered = text.to_lowercase();
            punct_en
                .expect("the En route implies punct_en")
                .add_punctuation(&lowered)
                .unwrap_or(lowered)
        }
        PunctRoute::Pass => text.to_string(),
    }
}

/// CJK Unified Ideographs (incl. ext-A and compatibility forms) — the
/// script class the zh-en CT-Transformer punctuates. Kana and hangul are
/// deliberately out: they stay on the passthrough/en path.
fn is_cjk_ideograph(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}')
}

// The multi-field `default()` + reassignment shape mirrors the recognizer
// config above and sherpa-onnx's own examples.
#[allow(clippy::field_reassign_with_default)]
fn vad_config(model_dir: &Path) -> VadModelConfig {
    let mut config = VadModelConfig::default();
    config.silero_vad = SileroVadModelConfig {
        model: Some(model_dir.join("silero_vad.onnx").display().to_string()),
        threshold: 0.5,
        min_silence_duration: 0.3,
        min_speech_duration: 0.25,
        window_size: 512,
        max_speech_duration: 20.0,
    };
    config.sample_rate = SAMPLE_RATE;
    config.num_threads = 1;
    config
}

/// A Silero-VAD-segmented, final-only sherpa-onnx provider.
pub struct SherpaProvider {
    model_dir: PathBuf,
    channel: SpeakerChannel,
    diarize: bool,
    queue: Option<mpsc::Sender<PcmChunk>>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl SherpaProvider {
    /// Resolve the model dir eagerly. An unknown model value lands on a
    /// nonexistent dir under the models root so `start` fails cleanly with
    /// the same load error an uninstalled model reports.
    pub fn new(model: &str, channel: SpeakerChannel, diarize: bool) -> Self {
        let root = crate::paths::sherpa_models_dir();
        let model_dir = crate::sherpa_models::stt_entry_for_value(model)
            .map(|entry| crate::sherpa_models::entry_dir(&root, entry))
            .unwrap_or_else(|| root.join("unknown-model"));
        Self {
            model_dir,
            channel,
            diarize,
            queue: None,
            cancel: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }
}

impl SttProvider for SherpaProvider {
    fn start(
        &mut self,
        callback: Box<dyn Fn(TranscriptEvent) + Send + Sync>,
        error_callback: Box<dyn Fn(String) + Send + Sync>,
    ) -> anyhow::Result<()> {
        if self.worker.is_some() {
            anyhow::bail!("sherpa provider is already running");
        }
        let engine = engine_for(&self.model_dir).map_err(anyhow::Error::msg)?;
        let vad = VoiceActivityDetector::create(&vad_config(&self.model_dir), VAD_BUFFER_SECONDS)
            .ok_or_else(|| anyhow::anyhow!("the speech model could not be loaded"))?;
        // Unbounded on purpose: decodes block the worker for up to whole
        // seconds per segment, so a bounded queue overflows during speech
        // and every dropped chunk is a permanent hole in the transcript —
        // a backlog is only lag and drains in the next silence.
        let (sender, receiver) = mpsc::channel();
        self.queue = Some(sender);
        self.cancel.store(false, Ordering::Release);
        let cancel = Arc::clone(&self.cancel);
        let channel = self.channel;
        let diarize = self.diarize;
        let sherpa_root = crate::paths::sherpa_models_dir();
        let punct_paths = crate::sherpa_models::punctuation_paths(&sherpa_root);
        self.worker = Some(thread::spawn(move || {
            run_worker(
                receiver,
                vad,
                engine,
                channel,
                diarize,
                punct_paths,
                cancel,
                callback,
                error_callback,
            )
        }));
        Ok(())
    }

    fn enqueue(&self, chunk: PcmChunk) -> bool {
        // `false` now means the worker is gone — a terminal failure the
        // error callback already reported — never a full queue.
        self.queue
            .as_ref()
            .is_some_and(|sender| sender.send(chunk).is_ok())
    }

    fn stop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.queue.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for SherpaProvider {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The optional punctuation add-ons loaded for a worker — each `None`
/// side passes its script's text through unmodified.
struct Punctuators {
    en: Option<OnlinePunctuation>,
    zh: Option<OfflinePunctuation>,
}

impl Punctuators {
    fn load(paths: &PunctuationPaths) -> Self {
        Self {
            en: paths
                .en
                .as_ref()
                .and_then(|(model, vocab)| create_punctuator(model, vocab)),
            zh: paths.zh.as_deref().and_then(create_punctuator_zh),
        }
    }
}

/// Feed PCM chunks to the VAD and submit each completed speech segment to
/// the shared engine. `flush` + one last drain on exit decodes trailing
/// speech. A decode failure is terminal: report once and exit.
#[allow(clippy::too_many_arguments)]
fn run_worker(
    receiver: mpsc::Receiver<PcmChunk>,
    vad: VoiceActivityDetector,
    engine: Arc<SherpaEngine>,
    channel: SpeakerChannel,
    diarize: bool,
    punct_paths: PunctuationPaths,
    cancel: Arc<AtomicBool>,
    callback: Box<dyn Fn(TranscriptEvent) + Send + Sync>,
    error_callback: Box<dyn Fn(String) + Send + Sync>,
) {
    // Punctuation is progressive enhancement like diarization: absent or
    // unloadable model files leave the raw transcript unchanged.
    let punct = Punctuators::load(&punct_paths);
    let decode = |samples: &[f32]| {
        engine
            .decode(samples.to_vec())
            .map(|text| restore_punct_and_case(punct.en.as_ref(), punct.zh.as_ref(), &text))
    };
    // Diarization is progressive enhancement: a missing/unloadable
    // embedding model leaves `speaker_idx` unset rather than failing STT.
    let mut tracker = diarize
        .then(|| crate::stt::speaker::tracker_if_installed(channel))
        .flatten();
    // Same terminal-error semantics as deepgram: a failure surfaced during a
    // user-requested stop is not an error worth an event.
    let report = |message: String| {
        if !cancel.load(Ordering::Acquire) {
            error_callback(message);
        }
    };
    while !cancel.load(Ordering::Acquire) {
        let chunk = match receiver.recv_timeout(WORKER_TICK) {
            Ok(chunk) => chunk,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let samples: Vec<f32> = chunk
            .samples
            .iter()
            .map(|sample| f32::from(*sample) / 32_768.0)
            .collect();
        vad.accept_waveform(&samples);
        if !drain_segments(&vad, &decode, channel, &mut tracker, &callback, &report) {
            return;
        }
    }
    vad.flush();
    drain_segments(&vad, &decode, channel, &mut tracker, &callback, &report);
}

/// Decode every queued speech segment. `false` means a terminal decode
/// failure — the error was reported once and the worker must exit.
fn drain_segments(
    vad: &VoiceActivityDetector,
    decode: &dyn Fn(&[f32]) -> Result<String, String>,
    channel: SpeakerChannel,
    tracker: &mut Option<crate::stt::SpeakerTracker>,
    callback: &(dyn Fn(TranscriptEvent) + Send + Sync),
    error_callback: &(dyn Fn(String) + Send + Sync),
) -> bool {
    while !vad.is_empty() {
        let Some(segment) = vad.front() else {
            break;
        };
        let samples = segment.samples().to_vec();
        let audio_start_ms = segment.start().max(0) as u64 * 1000 / 16_000;
        vad.pop();
        let timed_callback = |mut event: TranscriptEvent| {
            event.audio_start_ms = Some(audio_start_ms);
            callback(event);
        };
        if !emit_segment(
            &samples,
            decode,
            channel,
            tracker,
            &timed_callback,
            error_callback,
        ) {
            return false;
        }
    }
    true
}

/// Decode one segment and map it to a final transcript event. Blank text is
/// skipped silently; a decode error is reported once and ends the worker.
fn emit_segment(
    samples: &[f32],
    decode: impl Fn(&[f32]) -> Result<String, String>,
    channel: SpeakerChannel,
    tracker: &mut Option<crate::stt::SpeakerTracker>,
    callback: &(dyn Fn(TranscriptEvent) + Send + Sync),
    error_callback: &(dyn Fn(String) + Send + Sync),
) -> bool {
    match decode(samples) {
        Ok(text) => {
            let text = text.trim();
            if !text.is_empty() {
                let speaker_idx = tracker.as_mut().and_then(|t| t.assign(samples));
                callback(TranscriptEvent {
                    audio_start_ms: None,
                    channel,
                    text: text.to_string(),
                    finality: Finality::Final,
                    speaker_idx,
                });
            }
            true
        }
        Err(message) => {
            error_callback(message);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A live engine stand-in: the thread parks on the job queue until every
    /// sender drops, like the real decode loop.
    fn test_engine() -> SherpaEngine {
        let (jobs, rx) = mpsc::channel::<DecodeJob>();
        let thread = thread::spawn(move || while rx.recv().is_ok() {});
        SherpaEngine { jobs, thread }
    }

    /// An engine whose decode thread has already exited — the state a
    /// panicked job loop leaves behind.
    fn dead_engine() -> SherpaEngine {
        let (jobs, rx) = mpsc::channel::<DecodeJob>();
        drop(rx); // sends fail, same as a thread that died mid-loop
        let thread = thread::spawn(|| {});
        while !thread.is_finished() {
            thread::yield_now();
        }
        SherpaEngine { jobs, thread }
    }

    #[test]
    fn engine_for_respawns_after_the_engine_thread_dies() {
        let dir = Path::new("dead-engine-test-dir");
        // Seed the registry the way a dead decode thread leaves it: the
        // cached Arc is still valid but can never decode again.
        let dead = Arc::new(dead_engine());
        assert!(!dead.alive());
        *ENGINE.lock() = Some((dir.to_path_buf(), dead.clone()));
        let next = engine_for_with(dir, |_| Ok(test_engine())).unwrap();
        // A fresh engine replaces the dead Arc — not the poisoned slot.
        assert!(next.alive());
        assert!(!Arc::ptr_eq(&dead, &next));
        // The live replacement is cached and reused on the next call.
        let again = engine_for_with(dir, |_| Ok(test_engine())).unwrap();
        assert!(Arc::ptr_eq(&next, &again));
        *ENGINE.lock() = None;
    }

    #[test]
    fn emit_segment_maps_clean_text_to_a_final_event() {
        let events = std::sync::Mutex::new(Vec::new());
        let errors = std::sync::Mutex::new(Vec::new());
        // Decode seam: emit_segment takes a decode closure so the VAD→event
        // mapping is testable without model files.
        let ok = emit_segment(
            &[0.0f32; 4],
            |_| Ok("  你好 world  ".to_string()),
            SpeakerChannel::Me,
            &mut None,
            &|e| events.lock().unwrap().push(e),
            &|m| errors.lock().unwrap().push(m),
        );
        assert!(ok);
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].text, "你好 world");
        assert_eq!(events[0].finality, Finality::Final);
        assert_eq!(events[0].channel, SpeakerChannel::Me);
    }

    #[test]
    fn emit_segment_skips_blank_text_and_surfaces_decode_failure() {
        let events = std::sync::Mutex::new(Vec::new());
        let errors = std::sync::Mutex::new(Vec::new());
        // Blank decode text → ok, but no event.
        assert!(emit_segment(
            &[0.0f32; 4],
            |_| Ok("   ".to_string()),
            SpeakerChannel::Me,
            &mut None,
            &|e| events.lock().unwrap().push(e),
            &|m| errors.lock().unwrap().push(m),
        ));
        assert!(events.lock().unwrap().is_empty());
        // Decode failure → one error_callback call, false (terminal).
        assert!(!emit_segment(
            &[0.0f32; 4],
            |_| Err("decode failed".to_string()),
            SpeakerChannel::Me,
            &mut None,
            &|e| events.lock().unwrap().push(e),
            &|m| errors.lock().unwrap().push(m),
        ));
        assert_eq!(errors.lock().unwrap().len(), 1);
    }

    /// Without the punctuation models installed every byte passes through
    /// unchanged — the all-caps behavior is then upstream's, not ours.
    #[test]
    fn restore_punct_and_case_passes_through_without_a_model() {
        assert_eq!(
            restore_punct_and_case(None, None, "HELLO 你好"),
            "HELLO 你好"
        );
        assert_eq!(restore_punct_and_case(None, None, ""), "");
    }

    /// The zh/en routing hinge: ideographs route to the CT-Transformer,
    /// Latin and punctuation alone do not.
    #[test]
    fn is_cjk_ideograph_flags_only_ideographs() {
        assert!(is_cjk_ideograph('你'));
        assert!(is_cjk_ideograph('\u{f901}')); // compatibility ideograph
        assert!(!is_cjk_ideograph('a'));
        assert!(!is_cjk_ideograph('。'));
        assert!(!is_cjk_ideograph('あ')); // kana alone stays off the zh model
    }

    /// The routing rule, decided without models: CJK → zh when installed
    /// and passthrough when not — mixed CJK+Latin text must never fall
    /// into the en lowercase+CNN-BiLSTM path. ASCII letters → en;
    /// anything else passes through.
    #[test]
    fn punct_route_keeps_cjk_off_the_en_path() {
        use PunctRoute::*;
        assert_eq!(punct_route(true, true, "你好 WORLD"), Zh);
        // The arm that once leaked through: zh model absent → passthrough.
        assert_eq!(punct_route(true, false, "你好 WORLD"), Pass);
        assert_eq!(punct_route(false, false, "你好"), Pass);
        assert_eq!(punct_route(true, true, "HELLO WORLD"), En);
        assert_eq!(punct_route(true, false, "HELLO"), En);
        assert_eq!(punct_route(false, true, "HELLO"), Pass);
        assert_eq!(punct_route(true, true, "123 ?!?"), Pass);
    }

    #[test]
    fn provider_reports_not_enqueued_before_start_and_stop_is_idempotent() {
        let mut p = SherpaProvider::new("sense-voice", SpeakerChannel::Me, false);
        assert!(!p.enqueue(PcmChunk {
            samples: vec![1],
            sample_rate: 16_000,
            channels: 1
        }));
        p.stop(); // safe before start
    }

    #[test]
    fn start_fails_when_the_model_is_missing() {
        // A non-catalog name resolves to a models-dir child that can never
        // exist, so this stays deterministic on hosts where the real
        // sense-voice model is installed.
        let mut p = SherpaProvider::new("no-such-model", SpeakerChannel::Me, false);
        // No model files on disk → engine init fails synchronously.
        assert!(p.start(Box::new(|_| {}), Box::new(|_| {})).is_err());
    }
}
