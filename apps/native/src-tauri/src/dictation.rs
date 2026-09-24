//! Ask-input dictation: a microphone-only, transient STT session feeding a
//! single draft for the `me` channel. Unlike `ListenService` it persists
//! nothing — no `sessions`, `transcripts`, or `summaries` rows — and it
//! never opens `SystemAudioSource`.

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

/// Single-speaker draft state: committed finals plus the latest interim.
/// Unlike `TurnAssembler` there is no turn closing, silence cadence, or
/// channel switching — `finish()` yields the whole draft once and resets.
/// Live snapshots are `final: false`; only `finish()` marks `final: true`.
#[derive(Debug, Clone, Default)]
pub struct DraftAssembler {
    committed: String,
    provisional: Option<String>,
}

impl DraftAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one STT event into the draft and return the live snapshot;
    /// `None` for events that cannot change it (other channels, blank text).
    pub fn push(&mut self, event: TranscriptEvent) -> Option<DictationDraft> {
        if event.channel != SpeakerChannel::Me {
            return None;
        }
        let text = event.text.trim();
        if text.is_empty() {
            return None;
        }
        match event.finality {
            Finality::Interim => self.provisional = Some(text.to_string()),
            Finality::Final => {
                self.provisional = None;
                append_segment(&mut self.committed, text);
            }
        }
        Some(self.snapshot(false))
    }

    /// The complete draft — committed + provisional — marked `final`,
    /// then resets so a repeated `finish()` returns an empty draft.
    pub fn finish(&mut self) -> DictationDraft {
        let draft = self.snapshot(true);
        self.committed.clear();
        self.provisional = None;
        draft
    }

    fn snapshot(&self, finality: bool) -> DictationDraft {
        let mut text = self.committed.clone();
        if let Some(provisional) = &self.provisional {
            append_segment(&mut text, provisional);
        }
        DictationDraft { text, finality }
    }
}

fn append_segment(committed: &mut String, segment: &str) {
    if !committed.is_empty() {
        committed.push(' ');
    }
    committed.push_str(segment);
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
                worker: spawn_pump(rx, stt, source, cancel.clone()),
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

/// Transcript callback: fold each `me` event into the draft and emit the
/// live snapshot; events that can't change the draft emit nothing.
fn transcript_callback(
    assembler: &Arc<Mutex<DraftAssembler>>,
    emit: &Arc<dyn Fn(DictationEvent) + Send + Sync>,
) -> Box<dyn Fn(TranscriptEvent) + Send + Sync> {
    let assembler = assembler.clone();
    let emit = emit.clone();
    Box::new(move |event| {
        let draft = assembler.lock().push(event);
        if let Some(draft) = draft {
            (emit)(DictationEvent::Draft(draft));
        }
    })
}

/// Terminal provider-error callback: cancel the pump, mark the durable
/// status `error`, and emit one sanitized `dictation:error`. The cancel
/// always lands (it also makes `commit` abort a start whose provider
/// died mid-build), but the status write and emit are skipped once the
/// epoch moved on — a late error from a session a `stop()` already tore
/// down must not clobber the winner's status. The epoch check runs
/// while holding `state` (same pattern as `fail()`): `stop()` bumps the
/// epoch before its own `idle` write, so checking under the lock keeps
/// that write from slipping between the check and the `error` write.
fn provider_error_callback(
    state: &Arc<Mutex<DictationStatus>>,
    cancel: &Arc<AtomicBool>,
    epoch: &Arc<AtomicU64>,
    start_epoch: u64,
    emit: &Arc<dyn Fn(DictationEvent) + Send + Sync>,
) -> Box<dyn Fn(String) + Send + Sync> {
    let state = state.clone();
    let cancel = cancel.clone();
    let epoch = epoch.clone();
    let emit = emit.clone();
    Box::new(move |message| {
        cancel.store(true, Ordering::Release);
        let error = DictationError {
            message: sanitize_provider_error(&message),
            needs_setup: false,
        };
        {
            let mut status = state.lock();
            if epoch.load(Ordering::Acquire) != start_epoch {
                return;
            }
            status.state = "error".into();
            status.error = Some(error.clone());
        }
        (emit)(DictationEvent::Error {
            message: error.message,
            needs_setup: error.needs_setup,
        });
    })
}

/// Pump mic PCM into the provider until cancelled or the source
/// disconnects, then stop provider and source. Mirrors the Listen pump
/// minus turn flushing — dictation has no silence cadence.
fn spawn_pump(
    rx: mpsc::Receiver<PcmChunk>,
    mut stt: Box<dyn SttProvider>,
    mut source: MicSource,
    cancel: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut dropped_chunks = 0usize;
        while !cancel.load(Ordering::Acquire) {
            match rx.recv_timeout(WORKER_TICK) {
                Ok(chunk) => {
                    if !stt.enqueue(chunk) {
                        dropped_chunks += 1;
                        if dropped_chunks.is_multiple_of(100) {
                            log::warn!("dictation audio enqueue dropped {dropped_chunks} chunks");
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        stt.stop();
        source.stop();
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stt::{Finality, SpeakerChannel, TranscriptEvent};

    fn event(channel: SpeakerChannel, text: &str, finality: Finality) -> TranscriptEvent {
        TranscriptEvent {
            channel,
            text: text.into(),
            finality,
        }
    }

    #[test]
    fn interim_replaces_previous_provisional() {
        let mut a = DraftAssembler::new();
        let first = a
            .push(event(SpeakerChannel::Me, "hel", Finality::Interim))
            .unwrap();
        assert_eq!(first.text, "hel");
        assert!(!first.finality);
        let next = a
            .push(event(SpeakerChannel::Me, "hello", Finality::Interim))
            .unwrap();
        assert_eq!(next.text, "hello");
    }

    #[test]
    fn final_appends_to_committed_and_clears_provisional() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "hel", Finality::Interim));
        let draft = a
            .push(event(SpeakerChannel::Me, "hello", Finality::Final))
            .unwrap();
        assert_eq!(draft.text, "hello");
        assert!(!draft.finality);
        let draft = a
            .push(event(SpeakerChannel::Me, "wor", Finality::Interim))
            .unwrap();
        assert_eq!(draft.text, "hello wor");
    }

    #[test]
    fn whisper_style_final_chunks_accumulate_in_order() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "chunk one", Finality::Final));
        let draft = a
            .push(event(SpeakerChannel::Me, "chunk two", Finality::Final))
            .unwrap();
        assert_eq!(draft.text, "chunk one chunk two");
    }

    #[test]
    fn repeated_identical_words_are_kept() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "yes", Finality::Final));
        let draft = a
            .push(event(SpeakerChannel::Me, "yes", Finality::Final))
            .unwrap();
        assert_eq!(draft.text, "yes yes");
    }

    #[test]
    fn empty_and_whitespace_text_is_ignored() {
        let mut a = DraftAssembler::new();
        assert!(a
            .push(event(SpeakerChannel::Me, "  ", Finality::Interim))
            .is_none());
        assert!(a
            .push(event(SpeakerChannel::Me, "", Finality::Final))
            .is_none());
        assert_eq!(a.finish().text, "");
    }

    #[test]
    fn draft_text_is_trimmed() {
        let mut a = DraftAssembler::new();
        let draft = a
            .push(event(SpeakerChannel::Me, "  hello  ", Finality::Interim))
            .unwrap();
        assert_eq!(draft.text, "hello");
    }

    #[test]
    fn other_channels_do_not_change_the_draft() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "mine", Finality::Final));
        assert!(a
            .push(event(SpeakerChannel::Them, "theirs", Finality::Final))
            .is_none());
        assert!(a
            .push(event(SpeakerChannel::Them, "noise", Finality::Interim))
            .is_none());
        assert_eq!(a.finish().text, "mine");
    }

    #[test]
    fn finish_returns_complete_draft_once_and_resets() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "one", Finality::Final));
        a.push(event(SpeakerChannel::Me, "two", Finality::Interim));
        let draft = a.finish();
        assert_eq!(draft.text, "one two");
        assert!(draft.finality);
        let again = a.finish();
        assert_eq!(again.text, "");
        assert!(again.finality);
        let next = a
            .push(event(SpeakerChannel::Me, "three", Finality::Interim))
            .unwrap();
        assert_eq!(next.text, "three");
    }

    #[test]
    fn draft_serializes_final_field_name() {
        let payload = serde_json::to_value(DictationDraft {
            text: "hi".into(),
            finality: true,
        })
        .unwrap();
        assert_eq!(payload, serde_json::json!({ "text": "hi", "final": true }));
    }

    #[test]
    fn status_serializes_documented_wire_fields() {
        let status = DictationStatus {
            state: "error".into(),
            provider: Some("whisper".into()),
            error: Some(DictationError {
                message: "setup".into(),
                needs_setup: true,
            }),
        };
        let payload = serde_json::to_value(status).unwrap();
        assert_eq!(
            payload,
            serde_json::json!({
                "state": "error",
                "provider": "whisper",
                "error": { "message": "setup", "needs_setup": true },
            })
        );
    }

    #[test]
    fn is_listening_only_while_state_is_listening() {
        for (state, expected) in [("idle", false), ("error", false), ("listening", true)] {
            let status = DictationStatus {
                state: state.into(),
                provider: None,
                error: None,
            };
            assert_eq!(status.is_listening(), expected);
        }
    }

    fn service_root() -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "marvis-dictation-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    type EventLog = Arc<std::sync::Mutex<Vec<DictationEvent>>>;

    fn event_log() -> (EventLog, Arc<dyn Fn(DictationEvent) + Send + Sync>) {
        let events: EventLog = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = events.clone();
        (
            events,
            Arc::new(move |event| captured.lock().unwrap().push(event)),
        )
    }

    /// Mic denial is a setup error reported once — before any audio
    /// hardware or provider is touched.
    #[test]
    fn start_rejects_missing_mic_permission() {
        let root = service_root();
        let keystore = Keystore::at(root.join("keys.json"));
        let config = Config::default();
        let (events, emit) = event_log();
        let service = DictationService::new();

        let result = service.start(&keystore, &config, false, None, emit);

        assert!(result.is_err());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            DictationEvent::Error {
                needs_setup: true,
                ..
            }
        ));
        let status = service.status();
        assert_eq!(status.state, "error");
        assert!(!status.is_listening());
        assert_eq!(status.provider.as_deref(), Some("deepgram"));
        assert_eq!(
            status.error,
            Some(DictationError {
                message: "Microphone permission is required to dictate".into(),
                needs_setup: true,
            })
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// The Deepgram key is required only when that provider is configured,
    /// and the failure mirrors Listen's single `needs_setup` event.
    #[test]
    fn start_rejects_missing_deepgram_key() {
        let root = service_root();
        let keystore = Keystore::at(root.join("keys.json"));
        let config = Config::default();
        let (events, emit) = event_log();
        let service = DictationService::new();

        let result = service.start(&keystore, &config, true, None, emit);

        assert!(result.is_err());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            DictationEvent::Error {
                needs_setup: true,
                ..
            }
        ));
        let status = service.status();
        assert_eq!(status.state, "error");
        assert_eq!(
            status.error,
            Some(DictationError {
                message: "Speech-to-text provider is not configured".into(),
                needs_setup: true,
            })
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// Revalidation walks `start()`'s gate order: a denied mic still
    /// reports the mic error, then the next blocker (here the missing
    /// Deepgram key), and only a fully fixed setup clears to `idle`.
    #[test]
    fn revalidate_setup_walks_mic_then_provider_and_clears_when_fixed() {
        let root = service_root();
        let mut keystore = Keystore::at(root.join("keys.json"));
        let config = Config::default();
        let (_events, emit) = event_log();
        let service = DictationService::new();
        let sherpa_root = root.join("sherpa");

        assert!(service
            .start(&keystore, &config, false, None, emit)
            .is_err());
        assert_eq!(service.status().state, "error");

        // Mic still denied → same error, nothing to report.
        assert!(service
            .revalidate_setup(&keystore, &config, None, &sherpa_root, false)
            .is_none());

        // Mic granted → the next gate (missing key) becomes the reason.
        let next = service
            .revalidate_setup(&keystore, &config, None, &sherpa_root, true)
            .expect("a different broken reason must yield a new status");
        assert_eq!(next.state, "error");
        assert_eq!(
            next.error,
            Some(DictationError {
                message: "Speech-to-text provider is not configured".into(),
                needs_setup: true,
            })
        );

        // Key stored → the durable error clears to idle.
        keystore.set_key("deepgram", "dg-key").unwrap();
        let next = service
            .revalidate_setup(&keystore, &config, None, &sherpa_root, true)
            .expect("resolved error must yield a new status");
        assert_eq!(next.state, "idle");
        assert_eq!(next.error, None);
        assert_eq!(service.status().state, "idle");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Revalidation only touches setup failures — a runtime
    /// (`needs_setup: false`) error stays put.
    #[test]
    fn revalidate_setup_ignores_runtime_errors() {
        let root = service_root();
        let keystore = Keystore::at(root.join("keys.json"));
        let config = Config::default();
        let service = DictationService::new();
        *service.state.lock() = DictationStatus {
            state: "error".into(),
            provider: Some("deepgram".into()),
            error: Some(DictationError {
                message: "provider went away".into(),
                needs_setup: false,
            }),
        };
        assert!(service
            .revalidate_setup(&keystore, &config, None, &root.join("sherpa"), true)
            .is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    /// A provider name outside the catalog must fail before the microphone
    /// is opened — the provider is constructed first on purpose.
    #[test]
    fn start_rejects_unknown_stt_provider_before_mic() {
        let root = service_root();
        let keystore = Keystore::at(root.join("keys.json"));
        let mut config = Config::default();
        config.models.stt_provider = "not-a-provider".into();
        let (events, emit) = event_log();
        let service = DictationService::new();

        let result = service.start(&keystore, &config, true, None, emit);

        assert!(result.is_err());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            DictationEvent::Error {
                message,
                needs_setup,
            } => {
                assert!(needs_setup);
                assert_eq!(message, "unsupported STT provider: not-a-provider");
            }
            other => panic!("expected error event, got {other:?}"),
        }
        assert_eq!(service.status().state, "error");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Stop is safe on an idle service and returns an empty final draft.
    #[test]
    fn stop_is_idempotent_and_returns_empty_draft_when_idle() {
        let service = DictationService::new();
        for _ in 0..2 {
            let draft = service.stop();
            assert_eq!(draft.text, "");
            assert!(draft.finality);
            assert_eq!(service.status().state, "idle");
        }
    }

    /// Each `push` that yields a snapshot is forwarded as one draft event.
    #[test]
    fn transcript_callback_emits_only_draft_changing_events() {
        let assembler = Arc::new(Mutex::new(DraftAssembler::new()));
        let (events, emit) = event_log();
        let callback = transcript_callback(&assembler, &emit);

        callback(event(SpeakerChannel::Me, "hel", Finality::Interim));
        callback(event(SpeakerChannel::Them, "theirs", Finality::Final));
        callback(event(SpeakerChannel::Me, "  ", Finality::Interim));
        callback(event(SpeakerChannel::Me, "hello", Finality::Final));

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        match &events[0] {
            DictationEvent::Draft(draft) => {
                assert_eq!(draft.text, "hel");
                assert!(!draft.finality);
            }
            other => panic!("expected draft event, got {other:?}"),
        }
        match &events[1] {
            DictationEvent::Draft(draft) => assert_eq!(draft.text, "hello"),
            other => panic!("expected draft event, got {other:?}"),
        }
    }

    /// Provider failures cancel the pump, mark the status `error`, and the
    /// emitted message is flattened/capped — newlines can never reach the
    /// webview payload.
    #[test]
    fn provider_error_callback_cancels_and_sanitizes() {
        let state = Arc::new(Mutex::new(DictationStatus {
            state: "listening".into(),
            provider: Some("deepgram".into()),
            error: None,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let epoch = Arc::new(AtomicU64::new(7));
        let (events, emit) = event_log();
        let callback = provider_error_callback(&state, &cancel, &epoch, 7, &emit);

        callback("first line\nsecond line with detail".to_string());

        assert!(cancel.load(Ordering::Acquire));
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            DictationEvent::Error {
                message,
                needs_setup,
            } => {
                assert_eq!(message, "first line second line with detail");
                assert!(!needs_setup);
            }
            other => panic!("expected error event, got {other:?}"),
        }
        let status = state.lock().clone();
        assert_eq!(status.state, "error");
        assert_eq!(status.provider.as_deref(), Some("deepgram"));
        assert_eq!(
            status.error,
            Some(DictationError {
                message: "first line second line with detail".into(),
                needs_setup: false,
            })
        );
    }

    /// Once the epoch moved on — a `stop()` or a newer start won the
    /// race — a late provider error still cancels the pump but must not
    /// write status or emit `dictation:error` for a dead session.
    #[test]
    fn provider_error_callback_is_suppressed_after_epoch_bump() {
        let state = Arc::new(Mutex::new(DictationStatus {
            state: "idle".into(),
            provider: None,
            error: None,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let epoch = Arc::new(AtomicU64::new(3));
        let (events, emit) = event_log();
        let callback = provider_error_callback(&state, &cancel, &epoch, 3, &emit);

        epoch.fetch_add(1, Ordering::AcqRel);
        callback("late failure".to_string());

        assert!(cancel.load(Ordering::Acquire));
        assert!(events.lock().unwrap().is_empty());
        assert_eq!(state.lock().state, "idle");
        assert_eq!(state.lock().error, None);
    }

    /// A `Running` whose pump is just a cancellable sleep loop — enough
    /// to exercise commit/stop teardown without audio hardware or a
    /// real STT process.
    fn dummy_running() -> (Running, Arc<AtomicBool>, Arc<Mutex<DraftAssembler>>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let assembler = Arc::new(Mutex::new(DraftAssembler::new()));
        let worker = {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                while !cancel.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        };
        (
            Running {
                cancel: cancel.clone(),
                worker,
                assembler: assembler.clone(),
            },
            cancel,
            assembler,
        )
    }

    /// An uninterrupted commit stores the session and reports
    /// `listening`; the stop that follows still recovers the draft and
    /// leaves `idle`.
    #[test]
    fn commit_marks_listening_and_stop_recovers_the_draft() {
        let service = DictationService::new();
        let epoch = service.epoch.load(Ordering::Acquire);
        let (running, _cancel, assembler) = dummy_running();
        assembler
            .lock()
            .push(event(SpeakerChannel::Me, "hello", Finality::Final));

        service
            .commit(epoch, "deepgram".into(), running)
            .expect("uncontested commit");

        assert!(service.status().is_listening());
        assert!(service.running.lock().is_some());
        let draft = service.stop();
        assert_eq!(draft.text, "hello");
        assert!(draft.finality);
        assert_eq!(service.status().state, "idle");
        assert!(service.running.lock().is_none());
    }

    /// A `stop()` landing between a start's reset and its commit wins:
    /// the losing start cancels and joins the pump it just spawned
    /// instead of storing an untracked worker or overwriting `idle`.
    #[test]
    fn stop_during_start_commit_wins_and_leaves_no_worker() {
        let service = DictationService::new();
        // The snapshot a real `start()` takes after its internal reset.
        let epoch = service.epoch.load(Ordering::Acquire);
        // The racing stop lands while the start is still building.
        let _ = service.stop();

        let (running, cancel, _assembler) = dummy_running();
        let result = service.commit(epoch, "deepgram".into(), running);

        assert!(result.is_err());
        // The aborted start's pump was cancelled, not orphaned.
        assert!(cancel.load(Ordering::Acquire));
        assert_eq!(service.status().state, "idle");
        assert!(service.running.lock().is_none());
    }

    /// A provider error that already fired mid-build (cancel set, same
    /// epoch) also aborts the commit — no `listening` overwrite on a
    /// dead session.
    #[test]
    fn commit_aborts_when_provider_already_failed() {
        let service = DictationService::new();
        let epoch = service.epoch.load(Ordering::Acquire);
        let (running, cancel, _assembler) = dummy_running();
        cancel.store(true, Ordering::Release);

        let result = service.commit(epoch, "deepgram".into(), running);

        assert!(result.is_err());
        assert!(service.running.lock().is_none());
        assert!(!service.status().is_listening());
    }

    /// Hammering stop from several threads while a commit races in ends
    /// one way: `idle`, nothing running, every stop returning a final
    /// draft — no stale `listening`, no untracked worker.
    #[test]
    fn concurrent_stops_and_a_racing_commit_never_leave_stale_status() {
        let service = Arc::new(DictationService::new());
        let epoch = service.epoch.load(Ordering::Acquire);
        let (running, _cancel, _assembler) = dummy_running();

        let commit = {
            let service = service.clone();
            std::thread::spawn(move || service.commit(epoch, "deepgram".into(), running))
        };
        let stops: Vec<_> = (0..4)
            .map(|_| {
                let service = service.clone();
                std::thread::spawn(move || service.stop())
            })
            .collect();

        let _ = commit.join().expect("commit thread");
        for stop in stops {
            let draft = stop.join().expect("stop thread");
            assert!(draft.finality);
        }
        assert_eq!(service.status().state, "idle");
        assert!(service.running.lock().is_none());
    }

    /// Repeated stops are idempotent even when interleaved with an
    /// aborted start — the second and later stops return empty final
    /// drafts and the status stays `idle`.
    #[test]
    fn repeated_stops_after_aborted_start_stay_idle() {
        let service = DictationService::new();
        let epoch = service.epoch.load(Ordering::Acquire);
        let _ = service.stop();
        let (running, _cancel, _assembler) = dummy_running();
        assert!(service.commit(epoch, "deepgram".into(), running).is_err());
        for _ in 0..3 {
            let draft = service.stop();
            assert_eq!(draft.text, "");
            assert!(draft.finality);
            assert_eq!(service.status().state, "idle");
        }
    }
}
