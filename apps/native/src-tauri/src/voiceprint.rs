//! Voice enrollment — a stored speaker embedding ("voiceprint") that pins
//! the microphone channel's speaker 0 to the user, so "You" is a verified
//! identity rather than whoever happened to speak first.
//!
//! Recording follows the dictation pump shape: a worker owns the cpal
//! stream (it is `Send`, never `Sync`, so it must not sit in `AppState`)
//! and appends normalized 16 kHz mono samples to a shared buffer until
//! cancelled, then stops the device itself.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;

use crate::audio::{AudioSource, MicSource};
use crate::paths;
use crate::stt::speaker::{self, SpeakerTracker};

/// Sample rate of the normalized `PcmChunk`s the mic pump emits.
const SAMPLE_RATE: usize = 16_000;
/// Minimum usable recording — embeddings under this are too noisy to
/// verify against.
const MIN_SECONDS: f32 = 3.0;
/// Hard buffer cap (~30s) so a stuck UI can't grow memory unbounded.
const MAX_SAMPLES: usize = SAMPLE_RATE * 30;
/// Same junk gate as live tracking: a recording that is mostly silence
/// embeds as noise and would seed a wrong cluster 0.
const MIN_VOICED: f32 = 0.3;

/// What `voiceprint_status` reports to the UI.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct VoiceprintStatus {
    /// An enrolled voiceprint exists on disk.
    pub enrolled: bool,
    /// A recording session is live right now.
    pub recording: bool,
}

/// What `voice_enroll_stop` returns on success.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct VoiceEnrollResult {
    pub seconds: f32,
}

/// Live session: `buffer` is the shared sink (16 kHz mono i16, the
/// `PcmChunk` wire format); `cancel` asks the worker to stop the mic and
/// exit; `worker` is joined by `stop`/`cancel`. `source_error` carries a
/// fatal mic failure so `stop` can report it instead of a misleading
/// "too short" when the stream died mid-take.
struct LiveEnrollment {
    cancel: Arc<AtomicBool>,
    buffer: Arc<Mutex<Vec<i16>>>,
    source_error: Arc<Mutex<Option<String>>>,
    worker: JoinHandle<()>,
}

/// Owns the enrollment recording lifecycle — one live session at most.
pub struct VoiceEnroll {
    live: Mutex<Option<LiveEnrollment>>,
}

impl Default for VoiceEnroll {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceEnroll {
    pub fn new() -> Self {
        Self {
            live: Mutex::new(None),
        }
    }

    /// Begin capturing the default microphone. Fails before opening the
    /// device when the speaker model needed to embed the result is absent.
    pub fn start(&self) -> Result<(), String> {
        let mut slot = self.live.lock();
        if slot.is_some() {
            return Err("a recording is already in progress".to_string());
        }
        if speaker_model_path().is_none() {
            return Err("download the speaker diarization model first".to_string());
        }
        let (tx, rx) = mpsc::channel();
        let mut source = MicSource::new();
        if let Err(error) = source.start(tx) {
            log::warn!("voiceprint: microphone start failed: {error}");
            return Err("the microphone could not be started".to_string());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let buffer: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));
        let source_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let worker_cancel = Arc::clone(&cancel);
        let worker_buffer = Arc::clone(&buffer);
        let worker_error = Arc::clone(&source_error);
        let worker = thread::spawn(move || {
            loop {
                if worker_cancel.load(Ordering::Acquire) {
                    break;
                }
                if let Some(message) = source.try_recv_status() {
                    log::warn!("voiceprint: microphone died mid-recording: {message}");
                    *worker_error.lock() = Some("the microphone stopped working".to_string());
                    break;
                }
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(chunk) => {
                        let mut samples = worker_buffer.lock();
                        if samples.len() < MAX_SAMPLES {
                            samples.extend(chunk.samples);
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            source.stop();
        });
        *slot = Some(LiveEnrollment {
            cancel,
            buffer,
            source_error,
            worker,
        });
        Ok(())
    }

    /// Stop recording, embed the captured audio, and persist the
    /// voiceprint. Validation failures leave any previous voiceprint in
    /// place — a bad take never clobbers a good one.
    pub fn stop(&self) -> Result<VoiceEnrollResult, String> {
        let Some(live) = self.live.lock().take() else {
            return Err("no recording is in progress".to_string());
        };
        live.cancel.store(true, Ordering::Release);
        let _ = live.worker.join();
        let samples: Vec<f32> = live
            .buffer
            .lock()
            .iter()
            .map(|sample| f32::from(*sample) / 32_768.0)
            .collect();
        let seconds = samples.len() as f32 / SAMPLE_RATE as f32;
        if seconds < MIN_SECONDS {
            return Err(live.source_error.lock().clone().unwrap_or_else(|| {
                "the recording was too short — speak for a few seconds".into()
            }));
        }
        if speaker::voiced_fraction(&samples) < MIN_VOICED {
            return Err("no speech was detected — try again in a quieter spot".to_string());
        }
        let model = speaker_model_path()
            .ok_or_else(|| "download the speaker diarization model first".to_string())?;
        let tracker = SpeakerTracker::create(&model)
            .ok_or_else(|| "the voice sample could not be analyzed".to_string())?;
        let embedding = tracker
            .embed(&samples)
            .ok_or_else(|| "the voice sample could not be analyzed".to_string())?;
        save(&embedding).map_err(|_| "the voiceprint could not be saved".to_string())?;
        Ok(VoiceEnrollResult { seconds })
    }

    /// Abandon a live recording without saving — also called when the
    /// preferences window unmounts mid-take.
    pub fn cancel(&self) {
        if let Some(live) = self.live.lock().take() {
            live.cancel.store(true, Ordering::Release);
            let _ = live.worker.join();
        }
    }

    pub fn status(&self) -> VoiceprintStatus {
        VoiceprintStatus {
            enrolled: paths::voiceprint_file().is_file(),
            recording: self.live.lock().is_some(),
        }
    }

    /// Delete the stored voiceprint. Removing nothing is not an error —
    /// the UI's Remove button can race a status refresh.
    pub fn remove(&self) -> Result<(), String> {
        match std::fs::remove_file(paths::voiceprint_file()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("the voiceprint could not be removed".to_string()),
        }
    }
}

/// The installed speaker-embedding model, or `None` — enrollment cannot
/// embed without it.
fn speaker_model_path() -> Option<std::path::PathBuf> {
    crate::sherpa_models::speaker_embedding_model_path(&paths::sherpa_models_dir())
}

/// Load the enrolled embedding — `None` when absent or unreadable (a
/// truncated write must not seed clusters with garbage).
pub(crate) fn load() -> Option<Vec<f32>> {
    load_at(&paths::voiceprint_file())
}

fn load_at(path: &std::path::Path) -> Option<Vec<f32>> {
    let bytes = std::fs::read(path).ok()?;
    let embedding: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect();
    (!embedding.is_empty()).then_some(embedding)
}

fn save(embedding: &[f32]) -> std::io::Result<()> {
    save_at(&paths::voiceprint_file(), embedding)
}

fn save_at(path: &std::path::Path, embedding: &[f32]) -> std::io::Result<()> {
    let bytes: Vec<u8> = embedding
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seed that makes "You" verified: an enrolled embedding forms
    /// cluster 0, a matching voice keeps it, and a second voice confirmed
    /// twice lands on cluster 1 ("Guest 1").
    #[test]
    fn enrolled_voiceprint_claims_cluster_zero() {
        let mut clusters = speaker::SpeakerClusters::new();
        clusters.seed(vec![1.0, 0.0, 0.0]);
        assert_eq!(clusters.assign(&[0.98, 0.2, 0.0]), Some(0));
        // A genuinely different voice still needs two sightings.
        assert_eq!(clusters.assign(&[0.0, 1.0, 0.0]), None);
        assert_eq!(clusters.assign(&[0.0, 0.99, 0.1]), Some(1));
    }

    /// Unique scratch file per test — these run in parallel, so the real
    /// `voiceprint.bin` is never touched.
    fn scratch(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!("marvis-voiceprint-{tag}-{nanos}.bin"))
    }

    /// Garbage files must not seed the mic cluster: truncated or empty
    /// payloads round-trip to `None`, never a partial embedding.
    #[test]
    fn load_rejects_empty_and_garbage_files() {
        let path = scratch("garbage");
        assert_eq!(load_at(&path), None);
        std::fs::write(&path, b"").unwrap();
        assert_eq!(load_at(&path), None);
        // Not a multiple of 4 — dangling bytes drop; a wrong-dimension
        // vector still loads here but `SpeakerTracker::seed` rejects it
        // against the extractor's `dim()`.
        std::fs::write(&path, &[0u8; 6]).unwrap();
        assert_eq!(load_at(&path).map(|v| v.len()), Some(1));
        let _ = std::fs::remove_file(&path);
    }

    /// The round-trip is byte-exact — cosine math depends on it.
    #[test]
    fn save_then_load_preserves_the_embedding() {
        let path = scratch("roundtrip");
        let embedding = vec![0.25f32, -1.5, 2.75, 0.0];
        save_at(&path, &embedding).unwrap();
        assert_eq!(load_at(&path), Some(embedding));
        let _ = std::fs::remove_file(&path);
    }

    /// Idle state is neither enrolled nor recording — the card renders
    /// "Record" from this.
    #[test]
    fn fresh_service_reports_idle() {
        let service = VoiceEnroll::new();
        let status = service.status();
        assert!(!status.recording);
        // `enrolled` reflects the real on-disk file — only assert it agrees
        // with the filesystem rather than a value, so the test is hermetic.
        assert_eq!(status.enrolled, paths::voiceprint_file().is_file());
        assert!(service.stop().is_err());
        service.cancel();
    }
}
