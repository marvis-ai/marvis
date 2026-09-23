//! Listen orchestration: channel-aware turn assembly, persistence, and summaries.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::audio::{AudioSource, MicSource, PcmChunk, SystemAudioSource};
use crate::config::Config;
use crate::keystore::Keystore;
use crate::llm::{ChatMessage, ContentPart, Role};
use crate::prompts::listen_summary_prompt;
use crate::storage::{Db, Transcript};
use crate::stt::{make_stt_provider, Finality, SpeakerChannel, TranscriptEvent};

const SILENCE: Duration = Duration::from_millis(1500);
const SUMMARY_EVERY: usize = 5;
const HISTORY_LIMIT: usize = 20;
const WORKER_TICK: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClosedTurn {
    #[serde(serialize_with = "serialize_speaker")]
    pub speaker: SpeakerChannel,
    pub text: String,
    pub ts: i64,
}

#[derive(Debug, Clone, Default)]
struct Pending {
    text: String,
    last_final: Option<Instant>,
    /// A final replaces this interim result; later finals are new segments.
    has_interim: bool,
}

/// Deterministic state machine for provisional and final STT results.
#[derive(Debug, Clone)]
pub struct TurnAssembler {
    me: Pending,
    them: Pending,
    active: Option<SpeakerChannel>,
    closed: usize,
}

impl Default for TurnAssembler {
    fn default() -> Self {
        Self {
            me: Pending::default(),
            them: Pending::default(),
            active: None,
            closed: 0,
        }
    }
}

impl TurnAssembler {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, event: TranscriptEvent) -> Vec<ClosedTurn> {
        self.push_at(event, Instant::now())
    }

    pub fn push_at(&mut self, event: TranscriptEvent, now: Instant) -> Vec<ClosedTurn> {
        let text = event.text.trim().to_string();
        if text.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if self.active != Some(event.channel) {
            if let Some(previous) = self.active {
                out.extend(self.close(previous));
            }
            self.active = Some(event.channel);
        }
        let pending = self.pending_mut(event.channel);
        match event.finality {
            Finality::Interim => {
                pending.text = text;
                pending.has_interim = true;
            }
            Finality::Final => {
                if pending.has_interim {
                    pending.text = text;
                    pending.has_interim = false;
                } else if pending.text.is_empty() {
                    pending.text = text;
                } else if pending.text != text {
                    pending.text.push(' ');
                    pending.text.push_str(&text);
                }
                pending.last_final = Some(now);
            }
        }
        out.extend(self.flush_due(now));
        out
    }

    pub fn flush(&mut self) -> Vec<ClosedTurn> {
        let mut out = Vec::new();
        for channel in [SpeakerChannel::Me, SpeakerChannel::Them] {
            out.extend(self.close(channel));
        }
        self.active = None;
        out
    }
    pub fn flush_at(&mut self, now: Instant) -> Vec<ClosedTurn> {
        self.flush_due(now)
    }

    fn flush_due(&mut self, now: Instant) -> Vec<ClosedTurn> {
        let mut out = Vec::new();
        for channel in [SpeakerChannel::Me, SpeakerChannel::Them] {
            let due = self
                .pending(channel)
                .last_final
                .map(|at| now.duration_since(at) >= SILENCE)
                .unwrap_or(false);
            if due {
                out.extend(self.close(channel));
                if self.active == Some(channel) {
                    self.active = None;
                }
            }
        }
        out
    }
    fn close(&mut self, channel: SpeakerChannel) -> Vec<ClosedTurn> {
        let pending = self.pending_mut(channel);
        let text = std::mem::take(&mut pending.text);
        pending.last_final = None;
        pending.has_interim = false;
        if text.trim().is_empty() {
            return Vec::new();
        }
        self.closed += 1;
        vec![ClosedTurn {
            speaker: channel,
            text: text.trim().to_string(),
            ts: now_unix(),
        }]
    }
    fn pending(&self, channel: SpeakerChannel) -> &Pending {
        match channel {
            SpeakerChannel::Me => &self.me,
            SpeakerChannel::Them => &self.them,
        }
    }
    fn pending_mut(&mut self, channel: SpeakerChannel) -> &mut Pending {
        match channel {
            SpeakerChannel::Me => &mut self.me,
            SpeakerChannel::Them => &mut self.them,
        }
    }
    pub fn summary_boundary(&self) -> bool {
        self.closed > 0 && self.closed % SUMMARY_EVERY == 0
    }
    pub fn closed_count(&self) -> usize {
        self.closed
    }
}

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
fn serialize_speaker<S: serde::Serializer>(
    channel: &SpeakerChannel,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(speaker_name(*channel))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListenSummary {
    pub tldr: String,
    pub bullets: Vec<String>,
    pub follow_ups: Vec<String>,
    pub topic: Option<String>,
}

pub fn parse_summary(raw: &str) -> anyhow::Result<ListenSummary> {
    let value: serde_json::Value = serde_json::from_str(raw.trim())?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("summary is not an object"))?;
    let tldr = object
        .get("tldr")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if tldr.is_empty() {
        anyhow::bail!("summary has no tldr");
    }
    let strings = |key: &str, max: usize| -> Vec<String> {
        object
            .get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .take(max)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let topic = object
        .get("topic")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    Ok(ListenSummary {
        tldr,
        bullets: strings("bullets", 5),
        follow_ups: strings("follow_ups", 3),
        topic,
    })
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ListenStatus {
    pub state: String,
    pub provider: Option<String>,
    pub session_id: Option<i64>,
    pub turns: usize,
    pub mic: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "payload")]
pub enum ListenEvent {
    Turn(ClosedTurn),
    Summary(ListenSummary),
    Error { message: String, needs_setup: bool },
}

struct SessionContext {
    db: Arc<Db>,
    history: Arc<Mutex<Vec<Transcript>>>,
    emit: Arc<dyn Fn(ListenEvent) + Send + Sync>,
    session_id: i64,
    config: Config,
    keystore: Keystore,
    persisted_turns: Arc<AtomicUsize>,
    summary_workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
}
struct Running {
    cancel: Arc<AtomicBool>,
    sources: Vec<Box<dyn AudioSource>>,
    workers: Vec<JoinHandle<()>>,
    assembler: Arc<Mutex<TurnAssembler>>,
    context: Arc<SessionContext>,
    session_id: i64,
}

/// Thread-safe owner of a Listen session. AppState/commands are intentionally not coupled here.
pub struct ListenService {
    state: Mutex<ListenStatus>,
    running: Mutex<Option<Running>>,
    db: Mutex<Option<Arc<Db>>>,
    history: Arc<Mutex<Vec<Transcript>>>,
}

impl ListenService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ListenStatus {
                state: "idle".into(),
                provider: None,
                session_id: None,
                turns: 0,
                mic: false,
            }),
            running: Mutex::new(None),
            db: Mutex::new(None),
            history: Arc::new(Mutex::new(Vec::new())),
        }
    }
    pub fn status(&self) -> ListenStatus {
        self.state.lock().clone()
    }
    pub fn current_history(&self) -> Vec<Transcript> {
        self.history.lock().clone()
    }

    pub fn start(
        &self,
        db: Arc<Db>,
        keystore: &Keystore,
        config: &Config,
        mic_allowed: bool,
        emit: Arc<dyn Fn(ListenEvent) + Send + Sync>,
    ) -> anyhow::Result<()> {
        self.stop();
        self.history.lock().clear();
        let session_id = db.session_get_or_create_active("listen")?;
        let provider_name = config.models.stt_provider.clone();
        let model = config.models.stt_model.clone();
        let key = if provider_name == "deepgram" {
            keystore.key("deepgram")
        } else {
            None
        };
        if provider_name == "deepgram" && key.is_none() {
            let _ = db.session_end(session_id);
            emit(ListenEvent::Error {
                message: "Speech-to-text provider is not configured".into(),
                needs_setup: true,
            });
            return Ok(());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let assembler = Arc::new(Mutex::new(TurnAssembler::new()));
        let context = Arc::new(SessionContext {
            db: db.clone(),
            history: self.history.clone(),
            emit: emit.clone(),
            session_id,
            config: config.clone(),
            keystore: keystore.clone(),
            persisted_turns: Arc::new(AtomicUsize::new(0)),
            summary_workers: Arc::new(Mutex::new(Vec::new())),
        });
        let mut sources = Vec::new();
        let mut workers = Vec::new();
        let mut add = |channel: SpeakerChannel,
                       mut source: Box<dyn AudioSource>|
         -> anyhow::Result<()> {
            let (tx, rx) = mpsc::channel::<PcmChunk>();
            source.start(tx)?;
            let mut stt = make_stt_provider(&provider_name, key.clone(), model.clone(), channel)?;
            let callback_assembler = assembler.clone();
            let callback_context = context.clone();
            stt.start(
                Box::new(move |event| {
                    let turns = callback_assembler.lock().push(event);
                    for turn in turns {
                        persist_turn(&callback_context, turn);
                    }
                }),
                Box::new({
                    let emit = emit.clone();
                    move |message| {
                        emit(ListenEvent::Error {
                            message,
                            needs_setup: false,
                        })
                    }
                }),
            )?;
            let cancel_rx = cancel.clone();
            let worker_assembler = assembler.clone();
            let worker_context = context.clone();
            workers.push(std::thread::spawn(move || {
                while !cancel_rx.load(Ordering::Acquire) {
                    match rx.recv_timeout(WORKER_TICK) {
                        Ok(chunk) => {
                            let _ = stt.enqueue(chunk);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            let turns = worker_assembler.lock().flush_at(Instant::now());
                            for turn in turns {
                                persist_turn(&worker_context, turn);
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                stt.stop();
            }));
            sources.push(source);
            Ok(())
        };
        let mut mic_started = false;
        if mic_allowed {
            if add(SpeakerChannel::Me, Box::new(MicSource::new())).is_ok() {
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
            anyhow::bail!("no audio source available");
        }
        *self.db.lock() = Some(db);
        *self.running.lock() = Some(Running {
            cancel,
            sources,
            workers,
            assembler,
            context,
            session_id,
        });
        *self.state.lock() = ListenStatus {
            state: "listening".into(),
            provider: Some(provider_name),
            session_id: Some(session_id),
            turns: 0,
            mic: mic_started,
        };
        Ok(())
    }

    pub fn stop(&self) {
        let Some(mut running) = self.running.lock().take() else {
            return;
        };
        running.cancel.store(true, Ordering::Release);
        for source in &mut running.sources {
            source.stop();
        }
        for worker in running.workers {
            let _ = worker.join();
        }
        for turn in running.assembler.lock().flush() {
            persist_turn(&running.context, turn);
        }
        for worker in running.context.summary_workers.lock().drain(..) {
            let _ = worker.join();
        }
        let _ = running.context.db.session_end(running.session_id);
        self.state.lock().state = "idle".into();
    }
}

fn append_history(history: &mut Vec<Transcript>, transcript: Transcript) {
    history.push(transcript);
    if history.len() > HISTORY_LIMIT {
        history.remove(0);
    }
}

fn persist_turn(context: &Arc<SessionContext>, turn: ClosedTurn) {
    let inserted =
        context
            .db
            .transcript_add(context.session_id, speaker_name(turn.speaker), &turn.text);
    if let Ok(id) = inserted {
        let transcript = Transcript {
            id,
            session_id: context.session_id,
            speaker: speaker_name(turn.speaker).into(),
            text: turn.text.clone(),
            ts: turn.ts,
        };
        let mut history = context.history.lock();
        append_history(&mut history, transcript);
        let count = context.persisted_turns.fetch_add(1, Ordering::AcqRel) + 1;
        (context.emit)(ListenEvent::Turn(turn));
        if count % SUMMARY_EVERY == 0 {
            let transcript = history.clone();
            let summary_context = context.clone();
            context
                .summary_workers
                .lock()
                .push(std::thread::spawn(move || {
                    let result = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .ok()
                        .and_then(|runtime| {
                            runtime
                                .block_on(generate_summary(
                                    &summary_context.db,
                                    summary_context.session_id,
                                    &summary_context.config,
                                    &summary_context.keystore,
                                    &transcript,
                                ))
                                .ok()
                        });
                    if let Some(summary) = result {
                        (summary_context.emit)(ListenEvent::Summary(summary));
                    }
                }));
        }
    } else {
        // A database failure must not hide a valid transcript event.
        (context.emit)(ListenEvent::Turn(turn));
    }
}

/// Generate and persist a bounded structured summary. Provider failures are deliberately non-fatal.
pub async fn generate_summary(
    db: &Db,
    session_id: i64,
    config: &Config,
    keystore: &Keystore,
    transcript: &[Transcript],
) -> anyhow::Result<ListenSummary> {
    let candidates = crate::provider_candidates(config, keystore);
    let history = transcript
        .iter()
        .map(|t| format!("{}: {}", t.speaker, t.text))
        .collect::<Vec<_>>()
        .join("\n");
    let messages = [ChatMessage {
        role: Role::User,
        content: vec![ContentPart::Text(listen_summary_prompt(&history))],
    }];
    for candidate in candidates {
        let mut sink = |_token: &str| {};
        match candidate.provider.stream_chat(&messages, &mut sink).await {
            Ok(raw) => match parse_summary(&raw) {
                Ok(summary) => {
                    db.summary_add(
                        session_id,
                        &summary.tldr,
                        &summary.bullets,
                        &summary.follow_ups,
                        summary.topic.as_deref(),
                    )?;
                    return Ok(summary);
                }
                Err(error) => log::warn!("listen summary parse failed: {error}"),
            },
            Err(error) => log::warn!("listen summary provider failed: {error}"),
        }
    }
    anyhow::bail!("no provider produced a valid summary")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(channel: SpeakerChannel, text: &str, finality: Finality) -> TranscriptEvent {
        TranscriptEvent {
            channel,
            text: text.into(),
            finality,
        }
    }
    #[test]
    fn interim_replaces_with_different_final() {
        let mut a = TurnAssembler::new();
        let t = Instant::now();
        a.push_at(event(SpeakerChannel::Me, "hel", Finality::Interim), t);
        a.push_at(event(SpeakerChannel::Me, "hello", Finality::Final), t);
        assert_eq!(a.flush_at(t + SILENCE).pop().unwrap().text, "hello");
    }
    #[test]
    fn interim_replaces_and_final_closes_after_silence() {
        let mut a = TurnAssembler::new();
        let t = Instant::now();
        a.push_at(event(SpeakerChannel::Me, "hello", Finality::Interim), t);
        a.push_at(event(SpeakerChannel::Me, "hello", Finality::Final), t);
        assert_eq!(a.flush_at(t + SILENCE - Duration::from_millis(1)).len(), 0);
        assert_eq!(a.flush_at(t + SILENCE).pop().unwrap().text, "hello");
    }
    #[test]
    fn channel_switch_closes_previous() {
        let mut a = TurnAssembler::new();
        let t = Instant::now();
        a.push_at(event(SpeakerChannel::Me, "one", Finality::Final), t);
        assert_eq!(
            a.push_at(event(SpeakerChannel::Them, "two", Finality::Final), t)
                .pop()
                .unwrap()
                .speaker,
            SpeakerChannel::Me
        );
    }
    #[test]
    fn empty_text_is_dropped() {
        let mut a = TurnAssembler::new();
        assert!(a
            .push(event(SpeakerChannel::Me, "  ", Finality::Final))
            .is_empty());
    }
    #[test]
    fn summary_boundary_is_exactly_every_five_turns() {
        let mut a = TurnAssembler::new();
        let t = Instant::now();
        for i in 0..10 {
            let channel = if i % 2 == 0 {
                SpeakerChannel::Me
            } else {
                SpeakerChannel::Them
            };
            let _ = a.push_at(event(channel, "x", Finality::Final), t);
        }
        a.flush();
        assert!(a.summary_boundary());
        assert_eq!(a.closed_count(), 10);
    }
    #[test]
    fn history_keeps_only_the_last_twenty_turns() {
        let mut history = Vec::new();
        for id in 0..25 {
            append_history(
                &mut history,
                Transcript {
                    id,
                    session_id: 1,
                    speaker: "me".into(),
                    text: id.to_string(),
                    ts: id,
                },
            );
        }
        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(history.first().unwrap().id, 5);
        assert_eq!(history.last().unwrap().id, 24);
    }
    #[test]
    fn summary_parser_bounds_arrays_and_rejects_invalid_without_mutation() {
        let raw = r#"{"tldr":"x","bullets":["1","2","3","4","5","6"],"follow_ups":["a","b","c","d"],"topic":"t","extra":true}"#;
        let s = parse_summary(raw).unwrap();
        assert_eq!(s.bullets.len(), 5);
        assert_eq!(s.follow_ups.len(), 3);
        let unchanged = s.clone();
        assert!(parse_summary(r#"{"bullets":["mutate"]}"#).is_err());
        assert_eq!(s, unchanged);
    }
}
