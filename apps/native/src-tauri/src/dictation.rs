//! Ask-input dictation: a microphone-only, transient STT session feeding a
//! single draft for the `me` channel. Unlike `ListenService` it persists
//! nothing — no `sessions`, `transcripts`, or `summaries` rows — and it
//! never opens `SystemAudioSource`.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;

use crate::audio::{AudioSource, MicSource, PcmChunk};
use crate::config::Config;
use crate::keystore::Keystore;
use crate::stt::{
    make_stt_provider, sanitize_provider_error, whisper_setup_error, Finality, SpeakerChannel,
    SttProvider, TranscriptEvent, WhisperProvider,
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
    running: Mutex<Option<Running>>,
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
    pub fn start(
        &self,
        keystore: &Keystore,
        config: &Config,
        mic_allowed: bool,
        bundled_whisper: Option<&Path>,
        emit: Arc<dyn Fn(DictationEvent) + Send + Sync>,
    ) -> anyhow::Result<()> {
        let _ = self.stop();
        let provider_name = config.models.stt_provider.clone();
        let model = config.models.stt_model.clone();

        if !mic_allowed {
            return Err(self.fail(
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
        if provider_name == "deepgram" && key.is_none() {
            return Err(self.fail(
                &provider_name,
                "Speech-to-text provider is not configured",
                true,
                &emit,
            ));
        }
        if provider_name == "whisper" {
            // The same bundled-aware binary/model validation Listen uses —
            // a missing binary or model is a setup error, not a panic.
            if let Some(message) = whisper_setup_error(
                &WhisperProvider::status_with_bundled(bundled_whisper),
                &model,
            ) {
                return Err(self.fail(&provider_name, message, true, &emit));
            }
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
            provider_error_callback(&self.state, &cancel, &emit),
        ) {
            source.stop();
            log::warn!("dictation STT start failed: {error}");
            return Err(self.fail(
                &provider_name,
                "Speech-to-text could not be started",
                false,
                &emit,
            ));
        }

        *self.running.lock() = Some(Running {
            worker: spawn_pump(rx, stt, source, cancel.clone()),
            cancel,
            assembler,
        });
        let mut status = self.state.lock();
        status.state = "listening".into();
        status.provider = Some(provider_name);
        status.error = None;
        Ok(())
    }

    /// Cancel + join the pump (which stops the provider and source), then
    /// finalize the draft exactly once. Idempotent: repeated calls return
    /// an empty final draft and leave state `idle`.
    pub fn stop(&self) -> DictationDraft {
        let Some(running) = self.running.lock().take() else {
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
        // the assembler after this point.
        let _ = running.worker.join();
        let draft = running.assembler.lock().finish();
        *self.state.lock() = DictationStatus {
            state: "idle".into(),
            provider: None,
            error: None,
        };
        draft
    }

    /// Record a failure in the durable status, emit it as a sanitized
    /// `dictation:error`, and return it for the invoke result.
    fn fail(
        &self,
        provider: &str,
        message: &str,
        needs_setup: bool,
        emit: &Arc<dyn Fn(DictationEvent) + Send + Sync>,
    ) -> anyhow::Error {
        let message = sanitize_provider_error(message);
        *self.state.lock() = DictationStatus {
            state: "error".into(),
            provider: Some(provider.to_string()),
            error: Some(DictationError {
                message: message.clone(),
                needs_setup,
            }),
        };
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
/// status `error`, and emit one sanitized `dictation:error`.
fn provider_error_callback(
    state: &Arc<Mutex<DictationStatus>>,
    cancel: &Arc<AtomicBool>,
    emit: &Arc<dyn Fn(DictationEvent) + Send + Sync>,
) -> Box<dyn Fn(String) + Send + Sync> {
    let state = state.clone();
    let cancel = cancel.clone();
    let emit = emit.clone();
    Box::new(move |message| {
        cancel.store(true, Ordering::Release);
        let error = DictationError {
            message: sanitize_provider_error(&message),
            needs_setup: false,
        };
        {
            let mut status = state.lock();
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
        let (events, emit) = event_log();
        let callback = provider_error_callback(&state, &cancel, &emit);

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
}
