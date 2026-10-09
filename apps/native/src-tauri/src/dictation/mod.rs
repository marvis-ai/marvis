//! Ask-input dictation: a microphone-only, transient STT session feeding a
//! single draft for the `me` channel. Unlike `ListenService` it persists
//! nothing — no `sessions`, `transcripts`, or `summaries` rows — and it
//! never opens `SystemAudioSource`.

mod assembler;
mod worker;

use self::{assembler::*, worker::*};

#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;

use crate::audio::{AudioSource, MicSource, PcmChunk};
use crate::config::Config;
use crate::keystore::Keystore;
use crate::stt::{
    make_stt_provider, sanitize_provider_error, stt_setup_error, Finality, SpeakerChannel,
    SttProvider, TranscriptEvent,
};

const WORKER_TICK: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DictationStatus {
    pub state: String, // "idle" | "listening" | "error"
    pub provider: Option<String>,
    pub error: Option<DictationError>,
}

impl DictationStatus {
    /// `true` while dictation owns the microphone — `listen_start` reads
    /// this for the dictation/Listen mutual-exclusion check.
    pub fn is_listening(&self) -> bool {
        self.state == "listening"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DictationError {
    pub message: String,
    pub needs_setup: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DictationDraft {
    pub text: String,
    #[serde(rename = "final")]
    pub finality: bool,
}

/// Emitted to the `bar` window as `dictation:draft` / `dictation:error`;
/// state transitions are emitted separately as `dictation:state`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "payload")]
pub enum DictationEvent {
    Draft(DictationDraft),
    Error { message: String, needs_setup: bool },
}

/// Live session state: the pump worker owns the provider and source and
/// stops both before it exits; `assembler` is shared with the transcript
/// callback so `stop()` can finalize the draft once.
struct Running {
    cancel: Arc<AtomicBool>,
    worker: JoinHandle<()>,
    assembler: Arc<Mutex<DraftAssembler>>,
}

/// Thread-safe owner of a dictation session. Commands (`dictation_*`) are
/// intentionally not coupled here — the service takes plain borrows and an
/// emit closure, exactly like `ListenService`.
pub struct DictationService {
    state: Arc<Mutex<DictationStatus>>,
    /// Held across a `stop()`'s whole teardown (including the worker
    /// join) and across a `commit()`'s store+status write, so the two
    /// can never interleave into an untracked worker or a stale
    /// `listening`/`error` overwrite.
    running: Mutex<Option<Running>>,
    /// Start/stop generation: `stop()` bumps it before touching
    /// `running`, and `start()` snapshots it after its own reset — a
    /// stop (or newer start) that lands while a start is still building
    /// makes that start's `commit`/`fail`/error-callback writes no-op
    /// instead of overwriting the winner's status.
    epoch: Arc<AtomicU64>,
}

impl DictationService {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(DictationStatus {
                state: "idle".into(),
                provider: None,
                error: None,
            })),
            running: Mutex::new(None),
            epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn status(&self) -> DictationStatus {
        self.state.lock().clone()
    }

    /// Start mic-only dictation through the configured STT provider.
    ///
    /// Every failure is reported twice — once as a sanitized
    /// [`DictationEvent::Error`] for the webview and once as the `Err` for
    /// the invoke — and both carry curated/sanitized text only (no paths,
    /// keys, URLs, or process output). `listening` is set only after the
    /// source AND provider are both live.
    ///
    /// The epoch snapshot is taken after the reset: a `stop()` (or a
    /// newer start's reset) landing anywhere later bumps the epoch, which
    /// turns this start's `commit`/`fail` writes into no-ops and aborts
    /// the commit — a racing stop can never be overwritten by a late
    /// `listening`/`error` write or leave an untracked mic worker.
    pub fn start(
        &self,
        keystore: &Keystore,
        config: &Config,
        mic_allowed: bool,
        bundled_whisper: Option<&Path>,
        emit: Arc<dyn Fn(DictationEvent) + Send + Sync>,
    ) -> anyhow::Result<()> {
        let _ = self.stop();
        let epoch = self.epoch.load(Ordering::Acquire);
        let provider_name = config.models.stt_provider.clone();
        let model = config.models.stt_model.clone();

        if !mic_allowed {
            return Err(self.fail(
                epoch,
                &provider_name,
                "Microphone permission is required to dictate",
                true,
                &emit,
            ));
        }
        let key = if provider_name == "deepgram" {
            keystore.key("deepgram")
        } else {
            None
        };
        if let Some(message) = stt_setup_error(
            &provider_name,
            key.as_deref(),
            &model,
            bundled_whisper,
            &crate::paths::sherpa_models_dir(),
        ) {
            return Err(self.fail(epoch, &provider_name, message, true, &emit));
        }

        // Build the provider before touching the microphone so a bad
        // provider/model configuration fails without opening the device.
        let mut stt = match make_stt_provider(
            &provider_name,
            key,
            model,
            SpeakerChannel::Me,
            bundled_whisper,
            false,
        ) {
            Ok(stt) => stt,
            Err(error) => {
                log::warn!("dictation STT provider init failed: {error}");
                return Err(self.fail(
                    epoch,
                    &provider_name,
                    &sanitize_provider_error(&error.to_string()),
                    true,
                    &emit,
                ));
            }
        };
        let (tx, rx) = mpsc::channel::<PcmChunk>();
        let mut source = MicSource::new();
        if let Err(error) = source.start(tx) {
            // Raw source errors can name devices — log them, emit a curated one.
            log::warn!("dictation microphone start failed: {error}");
            return Err(self.fail(
                epoch,
                &provider_name,
                "Microphone could not be started",
                false,
                &emit,
            ));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let assembler = Arc::new(Mutex::new(DraftAssembler::new()));
        if let Err(error) = stt.start(
            transcript_callback(&assembler, &emit),
            provider_error_callback(&self.state, &cancel, &self.epoch, epoch, &emit),
        ) {
            source.stop();
            log::warn!("dictation STT start failed: {error}");
            return Err(self.fail(
                epoch,
                &provider_name,
                "Speech-to-text could not be started",
                false,
                &emit,
            ));
        }

        self.commit(
            epoch,
            provider_name,
            Running {
                worker: spawn_pump(
                    rx,
                    stt,
                    source,
                    cancel.clone(),
                    provider_error_callback(&self.state, &cancel, &self.epoch, epoch, &emit),
                ),
                cancel,
                assembler,
            },
        )
    }

    /// Commit phase of [`start`]: the epoch re-check, the `running`
    /// store, and the `listening` write are one critical section — a
    /// `stop()` holds the same lock for its whole teardown, so the two
    /// cannot interleave. If a stop/newer start already bumped the
    /// epoch, or the provider already died (its error callback sets
    /// `cancel`), this start lost the race: cancel + join the pump it
    /// just spawned — outside the lock — and leave the winner's status
    /// untouched.
    fn commit(&self, epoch: u64, provider_name: String, running: Running) -> anyhow::Result<()> {
        let mut slot = self.running.lock();
        if self.epoch.load(Ordering::Acquire) != epoch || running.cancel.load(Ordering::Acquire) {
            let Running { cancel, worker, .. } = running;
            cancel.store(true, Ordering::Release);
            drop(slot);
            let _ = worker.join();
            return Err(anyhow::anyhow!("dictation start was interrupted"));
        }
        *slot = Some(running);
        let mut status = self.state.lock();
        if self.epoch.load(Ordering::Acquire) != epoch
            || slot
                .as_ref()
                .is_some_and(|running| running.cancel.load(Ordering::Acquire))
        {
            let running = slot.take().expect("just stored");
            running.cancel.store(true, Ordering::Release);
            drop(status);
            drop(slot);
            let _ = running.worker.join();
            return Err(anyhow::anyhow!("dictation start was interrupted"));
        }
        status.state = "listening".into();
        status.provider = Some(provider_name);
        status.error = None;
        Ok(())
    }

    /// Cancel + join the pump (which stops the provider and source), then
    /// finalize the draft exactly once. Idempotent: repeated calls return
    /// an empty final draft and leave state `idle`. The epoch bump lands
    /// before the `running` lock, so an in-flight `start()` always sees
    /// it — the losing start cleans itself up at commit.
    pub fn stop(&self) -> DictationDraft {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        let mut slot = self.running.lock();
        let Some(running) = slot.take() else {
            *self.state.lock() = DictationStatus {
                state: "idle".into(),
                provider: None,
                error: None,
            };
            return DictationDraft {
                text: String::new(),
                finality: true,
            };
        };
        running.cancel.store(true, Ordering::Release);
        // The worker stops the provider and source before exiting, so the
        // join doubles as the teardown boundary — no callback can mutate
        // the assembler after this point. `slot` stays held across the
        // join so a racing `commit` serializes behind the whole stop.
        let _ = running.worker.join();
        let draft = running.assembler.lock().finish();
        *self.state.lock() = DictationStatus {
            state: "idle".into(),
            provider: None,
            error: None,
        };
        draft
    }

    /// Re-check a durable setup failure against current settings — same
    /// contract as `ListenService::revalidate_setup`: a fixed cause clears
    /// the error to `idle`, a still-broken setup rewrites the message.
    /// `mic_allowed` mirrors `start()`'s gate order (permission precedes
    /// provider checks); the caller re-reads it rather than prompting.
    /// Returns the updated status when it changed so the caller can emit
    /// `dictation:state`.
    pub fn revalidate_setup(
        &self,
        keystore: &Keystore,
        config: &Config,
        bundled_whisper: Option<&Path>,
        sherpa_root: &Path,
        mic_allowed: bool,
    ) -> Option<DictationStatus> {
        let mut status = self.state.lock();
        if status.state != "error" || status.error.as_ref().is_none_or(|e| !e.needs_setup) {
            return None;
        }
        let provider = config.models.stt_provider.clone();
        let message = if !mic_allowed {
            Some("Microphone permission is required to dictate")
        } else {
            let key = if provider == "deepgram" {
                keystore.key("deepgram")
            } else {
                None
            };
            stt_setup_error(
                &provider,
                key.as_deref(),
                &config.models.stt_model,
                bundled_whisper,
                sherpa_root,
            )
        };
        match message {
            None => {
                *status = DictationStatus {
                    state: "idle".into(),
                    provider: None,
                    error: None,
                };
                Some(status.clone())
            }
            Some(message) => {
                if status
                    .error
                    .as_ref()
                    .is_some_and(|error| error.message == message)
                {
                    return None;
                }
                status.provider = Some(provider);
                status.error = Some(DictationError {
                    message: message.to_string(),
                    needs_setup: true,
                });
                Some(status.clone())
            }
        }
    }

    /// Record a failure in the durable status, emit it as a sanitized
    /// `dictation:error`, and return it for the invoke result. When the
    /// epoch has moved on (a `stop()`/newer start landed mid-build) the
    /// status write and emit are skipped — a stale start must not
    /// overwrite the winner's `idle`/`listening` — but the `Err` still
    /// rejects the invoke.
    fn fail(
        &self,
        epoch: u64,
        provider: &str,
        message: &str,
        needs_setup: bool,
        emit: &Arc<dyn Fn(DictationEvent) + Send + Sync>,
    ) -> anyhow::Error {
        let message = sanitize_provider_error(message);
        // The epoch check and the status write share the `state` lock so
        // a `stop()`'s `idle` write can never land between them.
        let mut status = self.state.lock();
        if self.epoch.load(Ordering::Acquire) != epoch {
            return anyhow::anyhow!(message);
        }
        *status = DictationStatus {
            state: "error".into(),
            provider: Some(provider.to_string()),
            error: Some(DictationError {
                message: message.clone(),
                needs_setup,
            }),
        };
        drop(status);
        (emit)(DictationEvent::Error {
            message: message.clone(),
            needs_setup,
        });
        anyhow::anyhow!(message)
    }
}

