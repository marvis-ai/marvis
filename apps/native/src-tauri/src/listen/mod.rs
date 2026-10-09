//! Listen orchestration: channel-aware turn assembly, persistence, and summaries.

mod echo;
mod summary;
mod turns;

use self::{echo::*, summary::*, turns::*};

#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::audio::{AudioSource, MicSource, PcmChunk, SessionRecorder, SystemAudioSource};
use crate::config::Config;
use crate::keystore::Keystore;
use crate::llm::{ChatMessage, Role};
use crate::prompts::{summary_context, summary_system_prompt_for};
use crate::storage::Db;
use crate::stt::{
    make_stt_provider, sanitize_provider_error, stt_setup_error, Finality, SpeakerChannel,
    TranscriptEvent,
};

const SILENCE: Duration = Duration::from_millis(1500);
const SUMMARY_EVERY: usize = 5;
const WORKER_TICK: Duration = Duration::from_millis(100);
const SUMMARY_TIMEOUT: Duration = Duration::from_secs(30);

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn speaker_name(channel: SpeakerChannel) -> &'static str {
    match channel {
        SpeakerChannel::Me => "me",
        SpeakerChannel::Them => "them",
    }
}
/// Recorder channel index — mic is 0, system audio 1.
fn channel_idx(channel: SpeakerChannel) -> usize {
    match channel {
        SpeakerChannel::Me => 0,
        SpeakerChannel::Them => 1,
    }
}
/// The session's retained recording: `~/.marvis/audios/recording_<started>.wav`,
/// with the session id disambiguating second-granularity collisions. Returns
/// the recorder plus its path for `sessions.audio_file`; `None` on failure
/// (recording is best-effort — capture must not fail over disk trouble).
fn create_session_recorder(
    started_at: Option<i64>,
    session_id: i64,
) -> Option<(SessionRecorder, std::path::PathBuf)> {
    let dir = crate::paths::audios_dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        log::warn!("listen: audios dir unavailable: {error}");
        return None;
    }
    let stamp = started_at.unwrap_or_else(now_unix);
    for name in [
        format!("recording_{stamp}.wav"),
        format!("recording_{stamp}_{session_id}.wav"),
    ] {
        let path = dir.join(name);
        match SessionRecorder::create(&path) {
            Ok(recorder) => return Some((recorder, path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                log::warn!("listen: session recorder create failed: {error}");
                return None;
            }
        }
    }
    None
}
fn serialize_speaker<S: serde::Serializer>(
    channel: &SpeakerChannel,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(speaker_name(*channel))
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ListenError {
    pub message: String,
    pub needs_setup: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ListenStatus {
    pub state: String,
    pub provider: Option<String>,
    pub session_id: Option<i64>,
    pub audio_file: Option<String>,
    pub turns: usize,
    pub mic: bool,
    pub error: Option<ListenError>,
    /// Session start epoch — the header timer's zero point.
    pub started_at: Option<i64>,
    /// Accumulated pause seconds; `paused_since` is Some while paused.
    pub paused_secs: i64,
    pub paused_since: Option<i64>,
}

impl ListenStatus {
    /// `true` while a Listen session owns audio sources — `dictation_start`
    /// reads this for the dictation/Listen mutual-exclusion check.
    pub fn is_listening(&self) -> bool {
        self.state == "listening" || self.state == "paused"
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "payload")]
pub enum ListenEvent {
    Turn(ListenTurn),
    Summary(ListenSummaryEvent),
    Error { message: String, needs_setup: bool },
}

struct SessionContext {
    db: Arc<Db>,
    status: Arc<Mutex<ListenStatus>>,
    emit: Arc<dyn Fn(ListenEvent) + Send + Sync>,
    cancel: Arc<AtomicBool>,
    session_id: i64,
    config: Config,
    keystore: Keystore,
    persisted_turns: Arc<AtomicUsize>,
    summary_schedule: Mutex<SummarySchedule>,
    /// The session's retained recording (`None` when the WAV couldn't be
    /// created — recording is best-effort, never fatal to capture).
    recorder: Arc<Mutex<Option<SessionRecorder>>>,
}
struct Running {
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
    assembler: Arc<Mutex<TurnAssembler>>,
    context: Arc<SessionContext>,
    session_id: i64,
}

/// Thread-safe owner of a Listen session. AppState/commands are intentionally not coupled here.
pub struct ListenService {
    state: Arc<Mutex<ListenStatus>>,
    running: Mutex<Option<Running>>,
    db: Mutex<Option<Arc<Db>>>,
}

impl ListenService {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(ListenStatus {
                state: "idle".into(),
                provider: None,
                session_id: None,
                audio_file: None,
                turns: 0,
                mic: false,
                error: None,
                started_at: None,
                paused_secs: 0,
                paused_since: None,
            })),
            running: Mutex::new(None),
            db: Mutex::new(None),
        }
    }
    pub fn status(&self) -> ListenStatus {
        self.state.lock().clone()
    }

    pub fn start(
        &self,
        db: Arc<Db>,
        keystore: &Keystore,
        config: &Config,
        mic_allowed: bool,
        bundled_whisper: Option<&std::path::Path>,
        emit: Arc<dyn Fn(ListenEvent) + Send + Sync>,
    ) -> anyhow::Result<()> {
        self.stop();
        let provider_name = config.models.stt_provider.clone();
        let model = config.models.stt_model.clone();
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
            *self.state.lock() = ListenStatus {
                state: "error".into(),
                provider: Some(provider_name),
                session_id: None,
                audio_file: None,
                turns: 0,
                mic: false,
                error: Some(ListenError {
                    message: message.to_string(),
                    needs_setup: true,
                }),
                started_at: None,
                paused_secs: 0,
                paused_since: None,
            };
            emit(ListenEvent::Error {
                message: message.to_string(),
                needs_setup: true,
            });
            anyhow::bail!(message)
        }
        // `stop()` only ends the session this service was running — a row
        // still open from a killed run or a failed start would otherwise
        // absorb the new recording, so a Listen press always mints fresh.
        if let Err(error) = db.session_end_open("listen") {
            log::warn!("listen: session_end_open before start failed: {error}");
        }
        let session_id = db.session_get_or_create_active("listen")?;
        // The engine that captured the session goes on the row now —
        // a finished doc's header reads it back off `session_list`.
        let stt_label = if model.trim().is_empty() {
            provider_name.clone()
        } else {
            format!("{provider_name} {}", model.trim())
        };
        if let Err(error) = db.session_set_stt(session_id, &stt_label) {
            log::warn!("listen: session_set_stt failed: {error}");
        }
        let existing = db.transcripts_for(session_id, None)?;
        // The recording must exist before workers stream — `audio_file`
        // links immediately so a crashed run still finds the partial WAV.
        let started_at = db.session_started_at(session_id).ok().flatten();
        let recorder = create_session_recorder(started_at, session_id);
        if let Some((_, path)) = &recorder {
            if let Err(error) = db.session_set_audio_file(session_id, &path.to_string_lossy()) {
                log::warn!("listen: session_set_audio_file failed: {error}");
            }
        }
        let audio_file = match db.session_audio_file(session_id) {
            Ok(path) => path,
            Err(error) => {
                log::warn!(
                    "listen: session_audio_file failed for session {session_id}: {error}"
                );
                None
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let assembler = Arc::new(Mutex::new(TurnAssembler::new()));
        let echo_gate = Arc::new(Mutex::new(EchoGate::default()));
        let context = Arc::new(SessionContext {
            db: db.clone(),
            status: self.state.clone(),
            emit: emit.clone(),
            cancel: cancel.clone(),
            session_id,
            config: config.clone(),
            keystore: keystore.clone(),
            persisted_turns: Arc::new(AtomicUsize::new(existing.len())),
            summary_schedule: Mutex::new(SummarySchedule::default()),
            recorder: Arc::new(Mutex::new(recorder.map(|(recorder, _)| recorder))),
        });
        let mut workers = Vec::new();
        let mut add =
            |channel: SpeakerChannel, mut source: Box<dyn AudioSource>| -> anyhow::Result<()> {
                let mut stt = make_stt_provider(
                    &provider_name,
                    key.clone(),
                    model.clone(),
                    channel,
                    bundled_whisper,
                    true,
                )?;
                let (tx, rx) = mpsc::channel::<PcmChunk>();
                if let Err(error) = source.start(tx) {
                    source.stop();
                    return Err(error);
                }
                let callback_assembler = assembler.clone();
                let callback_gate = echo_gate.clone();
                let callback_context = context.clone();
                if let Err(error) = stt.start(
                    Box::new(move |mut event| {
                        event.audio_start_ms = event.audio_start_ms.and_then(|ms| {
                            callback_context
                                .recorder
                                .lock()
                                .as_ref()
                                .and_then(|r| r.channel_start_ms(channel_idx(event.channel)))
                                .map(|start| start + ms)
                        });
                        // `them` finals seed the echo reference (interims
                        // would inflate it with text that may never land);
                        // a `me` event that re-transcribes one is speaker
                        // playback heard by the mic — dropped before it
                        // can emit a phantom "You" line or close the
                        // speaker's turn.
                        let dropped = {
                            let mut gate = callback_gate.lock();
                            match event.channel {
                                SpeakerChannel::Them
                                    if event.finality == Finality::Final =>
                                {
                                    gate.record(&event.text);
                                    false
                                }
                                SpeakerChannel::Me if gate.is_echo(&event.text) => {
                                    log::debug!(
                                        "listen: dropped mic echo {:?}",
                                        event.text
                                    );
                                    true
                                }
                                _ => false,
                            }
                        };
                        if dropped {
                            // A dropped final leaves its last interim
                            // seeded as the channel's provisional — it
                            // would still land in the next `close()`.
                            // Clear it and re-emit the interim so a live
                            // "You" bubble sheds the echo tail.
                            if event.finality == Finality::Final {
                                let (interim, label, audio_start_ms) = {
                                    let mut assembler = callback_assembler.lock();
                                    assembler.drop_provisional(SpeakerChannel::Me);
                                    (
                                        assembler.interim(SpeakerChannel::Me),
                                        assembler.interim_label(SpeakerChannel::Me),
                                        assembler.interim_audio_start_ms(SpeakerChannel::Me),
                                    )
                                };
                                if let Some(text) = interim {
                                    (callback_context.emit)(ListenEvent::Turn(ListenTurn {
                                        audio_start_ms,
                                        speaker: SpeakerChannel::Me,
                                        speaker_idx: label,
                                        text,
                                        ts: now_unix(),
                                        session_id: callback_context.session_id,
                                        finality: false,
                                    }));
                                }
                            }
                            return;
                        }
                        let channel = event.channel;
                        let speaker_idx = event.speaker_idx;
                        let (turns, interim, interim_label, audio_start_ms) = {
                            let mut assembler = callback_assembler.lock();
                            let turns = assembler.push(event);
                            let interim = assembler.interim(channel);
                            let label = assembler.interim_label(channel).or(speaker_idx);
                            (
                                turns,
                                interim,
                                label,
                                assembler.interim_audio_start_ms(channel),
                            )
                        };
                        for turn in turns {
                            persist_turn(&callback_context, turn);
                        }
                        if let Some(text) = interim {
                            (callback_context.emit)(ListenEvent::Turn(ListenTurn {
                                audio_start_ms,
                                speaker: channel,
                                speaker_idx: interim_label,
                                text,
                                ts: now_unix(),
                                session_id: callback_context.session_id,
                                finality: false,
                            }));
                        }
                    }),
                    Box::new({
                        let callback_context = context.clone();
                        move |message| {
                            report_terminal_error(&callback_context, &message);
                        }
                    }),
                ) {
                    source.stop();
                    return Err(error);
                }
                let cancel_rx = cancel.clone();
                let paused_rx = paused.clone();
                let worker_assembler = assembler.clone();
                let worker_context = context.clone();
                let source_error_message = match channel {
                    SpeakerChannel::Me => "The microphone stopped working",
                    SpeakerChannel::Them => "System audio capture stopped working",
                };
                workers.push(std::thread::spawn(move || {
                    let mut dropped_chunks = 0usize;
                    while !cancel_rx.load(Ordering::Acquire) {
                        // A fatal backend error means the stream is dead even
                        // though `is_running` still reads true — end the
                        // session rather than recording silence.
                        if let Some(message) = source.try_recv_status() {
                            log::error!("listen audio source died mid-session: {message}");
                            report_terminal_error(&worker_context, source_error_message);
                            break;
                        }
                        match rx.recv_timeout(WORKER_TICK) {
                            Ok(chunk) => {
                                // Soft-pause: drain the source but starve STT — nothing transcribes
                                // and nothing persists (or records) while paused.
                                if paused_rx.load(Ordering::Acquire) {
                                    continue;
                                }
                                if let Some(recorder) = worker_context.recorder.lock().as_mut() {
                                    if let Err(error) =
                                        recorder.push(channel_idx(channel), &chunk.samples)
                                    {
                                        log::warn!("listen recorder write failed: {error}");
                                    }
                                }
                                if !stt.enqueue(chunk) {
                                    dropped_chunks += 1;
                                    if dropped_chunks.is_multiple_of(100) {
                                        log::warn!(
                                            "listen audio enqueue dropped {dropped_chunks} chunks"
                                        );
                                    }
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                        let turns = worker_assembler.lock().flush_at(Instant::now());
                        for turn in turns {
                            persist_turn(&worker_context, turn);
                        }
                    }
                    stt.stop();
                    source.stop();
                }));
                Ok(())
            };
        let mut mic_started = false;
        if mic_allowed {
            if add(SpeakerChannel::Me, Box::<MicSource>::default()).is_ok() {
                mic_started = true;
            } else {
                log::warn!("listen microphone unavailable");
            }
        }
        if let Ok(source) = SystemAudioSource::new() {
            if let Err(error) = add(SpeakerChannel::Them, Box::new(source)) {
                log::warn!("listen system audio unavailable: {error}");
            }
        } else {
            log::warn!("listen system audio unavailable");
        }
        if workers.is_empty() {
            let _ = db.session_end(session_id);
            *self.state.lock() = ListenStatus {
                state: "error".into(),
                provider: Some(provider_name),
                session_id: None,
                audio_file: None,
                turns: 0,
                mic: false,
                error: Some(ListenError {
                    message: "no audio source available".into(),
                    needs_setup: false,
                }),
                started_at: None,
                paused_secs: 0,
                paused_since: None,
            };
            emit(ListenEvent::Error {
                message: "no audio source available".into(),
                needs_setup: false,
            });
            anyhow::bail!("no audio source available");
        }
        *self.db.lock() = Some(db);
        *self.running.lock() = Some(Running {
            cancel,
            paused,
            workers,
            assembler,
            context,
            session_id,
        });
        let mut status = self.state.lock();
        status.state = "listening".into();
        status.provider = Some(provider_name);
        status.session_id = Some(session_id);
        status.audio_file = audio_file;
        status.turns = existing.len();
        status.mic = mic_started;
        status.error = None;
        status.started_at = started_at;
        Ok(())
    }

    /// Re-check a durable setup failure against current settings — a config
    /// write, key store, or model install can resolve the cause without a
    /// new start attempt, and the error should clear (or rewrite to the
    /// still-broken reason) instead of lingering until then. Returns the
    /// updated status when it changed so the caller can emit `listen:state`;
    /// live sessions and non-setup errors are left untouched.
    pub fn revalidate_setup(
        &self,
        keystore: &Keystore,
        config: &Config,
        bundled_whisper: Option<&std::path::Path>,
        sherpa_root: &std::path::Path,
    ) -> Option<ListenStatus> {
        let mut status = self.state.lock();
        if status.state != "error" || status.error.as_ref().is_none_or(|e| !e.needs_setup) {
            return None;
        }
        let provider = config.models.stt_provider.clone();
        let key = if provider == "deepgram" {
            keystore.key("deepgram")
        } else {
            None
        };
        match stt_setup_error(
            &provider,
            key.as_deref(),
            &config.models.stt_model,
            bundled_whisper,
            sherpa_root,
        ) {
            None => {
                *status = ListenStatus {
                    state: "idle".into(),
                    provider: None,
                    session_id: None,
                    audio_file: None,
                    turns: 0,
                    mic: false,
                    error: None,
                    started_at: None,
                    paused_secs: 0,
                    paused_since: None,
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
                status.error = Some(ListenError {
                    message: message.to_string(),
                    needs_setup: true,
                });
                Some(status.clone())
            }
        }
    }

    pub fn stop(&self) {
        let Some(running) = self.running.lock().take() else {
            *self.state.lock() = ListenStatus {
                state: "idle".into(),
                provider: None,
                session_id: None,
                audio_file: None,
                turns: 0,
                mic: false,
                error: None,
                started_at: None,
                paused_secs: 0,
                paused_since: None,
            };
            return;
        };
        running.cancel.store(true, Ordering::Release);
        for worker in running.workers {
            let _ = worker.join();
        }
        for turn in running.assembler.lock().flush() {
            persist_turn(&running.context, turn);
        }
        // Summary generation can be waiting on a provider for up to
        // SUMMARY_TIMEOUT. Detach it after capture finalization so Stop
        // returns promptly; its session-tagged event can finish the viewed
        // document when the result arrives.
        let _ = running.context.db.session_end(running.session_id);
        *self.state.lock() = ListenStatus {
            state: "idle".into(),
            provider: None,
            session_id: None,
            audio_file: None,
            turns: 0,
            mic: false,
            error: None,
            started_at: None,
            paused_secs: 0,
            paused_since: None,
        };
    }

    /// Pause without tearing down sources: workers keep draining (and
    /// dropping) chunks so resume is instant. The open turn flushes first —
    /// a pause is a clean transcript boundary.
    pub fn pause(&self) -> Option<ListenStatus> {
        let running = self.running.lock();
        let running = running.as_ref()?;
        if running.paused.swap(true, Ordering::AcqRel) {
            return Some(self.status());
        }
        for turn in running.assembler.lock().flush() {
            persist_turn(&running.context, turn);
        }
        let mut status = self.state.lock();
        status.state = "paused".into();
        status.paused_since = Some(now_unix());
        Some(status.clone())
    }

    pub fn resume(&self) -> Option<ListenStatus> {
        let running = self.running.lock();
        let running = running.as_ref()?;
        if !running.paused.swap(false, Ordering::AcqRel) {
            return Some(self.status());
        }
        let mut status = self.state.lock();
        if let Some(since) = status.paused_since.take() {
            status.paused_secs += now_unix() - since;
        }
        status.state = "listening".into();
        Some(status.clone())
    }
}

