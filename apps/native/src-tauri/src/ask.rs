//! `ask` — question + latest screen frame → streaming LLM → persisted.
//!
//! Pipeline (spec §Ask): [`AskService::send`] shows the ask panel,
//! resolves the FAILOVER CHAIN (`providers.order` minus disabled/unusable
//! — see `provider_candidates` in lib.rs), grabs the newest
//! [`RingBuffer`] frame (`None` → text-only), then tries each candidate
//! in turn: a failed provider logs and hands off to the next; only when
//! every candidate fails does `ask:error` fire. Both sides of the
//! exchange land in the `ask` session's `ai_messages` rows (the user row
//! persists once, before the first attempt). A provider
//! `MultimodalUnsupported` rejection retries once without the image
//! (per attempt); [`AskService::close`] aborts any in-flight stream via
//! a [`CancellationToken`].
//!
//! Event protocol (emitted to the `ask` window via `app.emit_to`):
//! - `ask:state` `{"state": "loading"|"streaming"|"idle"}` — `streaming`
//!   fires on each attempt's FIRST token; every `loading` carries
//!   `"question"` so the panel resets its buffer + header per run AND
//!   per failover retry (pre-flight errors emit `loading` → `error` →
//!   `idle` too).
//! - `ask:chunk` `{"text": token}` per token.
//! - `ask:done` `{"full": full_reply, "provider": id, "model": id}` on
//!   success — the pair that actually answered, for the panel's chip.
//! - `ask:error` `{"message": ..., "needs_setup": bool?}` on failure —
//!   `needs_setup` when the chain was empty (no usable provider at all).
//!
//! Broadcast (all windows): `capture:permission-needed` when a frame
//! exists but screen permission was revoked mid-session — the ring's
//! stale frame is dropped and the ask continues text-only.
//!
//! Wiring note for Task 14: `send`/`send_screen_only`/`close` take a
//! [`Deps`] bundle of `AppState` fields so this module never names the
//! not-yet-existing `AppState` type. `deps.db` must be an [`Arc`] — the
//! stream runs on a spawned task that outlives the command call — and
//! `AppState.ask` must be an `Arc<AskService>` (all fields are
//! interior-mutable; the spawned task keeps a share). After synchronous
//! pre-flight, real work runs inside `tauri::async_runtime::spawn` so the
//! invoking command handler returns immediately instead of blocking on
//! an LLM stream.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use crate::capture::{Frame, RingBuffer};
use crate::config::Config;
use crate::keystore::Keystore;
use crate::llm::{ChatMessage, LlmError, Provider, Role};
use crate::prompts::system_prompt;
use crate::storage::Db;
use crate::windows::{Panel, WindowPool};
use crate::ProviderCandidate;

/// Event names — part of the webview contract; change together with
/// `src/lib/events.ts`.
const EV_STATE: &str = "ask:state";
const EV_CHUNK: &str = "ask:chunk";
const EV_DONE: &str = "ask:done";
const EV_ERROR: &str = "ask:error";

/// `send_screen_only`'s fixed question (spec §Hotkeys `Cmd+Shift+S`).
const SCREEN_ONLY_PROMPT: &str = "Describe what is on my screen and how you can help.";

/// Lifecycle of one ask run. Mirrors the `ask:state` event so Rust-side
/// status reads see exactly what the webview sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskState {
    Idle,
    Loading,
    Streaming,
}

/// The `AppState` fields the ask pipeline needs, bundled so this module
/// compiles before `AppState` exists. `db` is an owned `Arc` (the spawned
/// stream task outlives the call); the rest are locked only during
/// synchronous pre-flight, so plain `&Mutex` borrows suffice.
pub struct Deps<'a> {
    pub db: Arc<Db>,
    pub ring: &'a Mutex<RingBuffer>,
    pub keystore: &'a Mutex<Keystore>,
    pub config: &'a Mutex<Config>,
    pub pool: &'a Mutex<WindowPool>,
}

/// One ask at a time; the Task-14 `AppState` share.
pub struct AskService {
    state: Mutex<AskState>,
    /// Cancel handle for the in-flight stream; replaced per run/close.
    cancel: Mutex<CancellationToken>,
    /// Last completed assistant reply (scroll/status consumers).
    current_response: Mutex<String>,
    /// Last submitted user text.
    current_question: Mutex<String>,
    /// Bumped per `send`: a stale (cancelled) task's trailing events are
    /// dropped instead of clobbering a newer run's state/UI.
    generation: AtomicU64,
}

impl AskService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(AskState::Idle),
            cancel: Mutex::new(CancellationToken::new()),
            current_response: Mutex::new(String::new()),
            current_question: Mutex::new(String::new()),
            generation: AtomicU64::new(0),
        }
    }

    #[allow(dead_code)] // status consumers land with the webview tasks
    pub fn state(&self) -> AskState {
        *self.state.lock()
    }

    #[allow(dead_code)] // status consumers land with the webview tasks
    pub fn current_response(&self) -> String {
        self.current_response.lock().clone()
    }

    #[allow(dead_code)] // status consumers land with the webview tasks
    pub fn current_question(&self) -> String {
        self.current_question.lock().clone()
    }

    /// Ask a free-text question. A send while `Loading`/`Streaming` is
    /// ignored (warn-logged) rather than cancel-then-send — the in-flight
    /// stream keeps running.
    ///
    /// Returns after synchronous pre-flight; the stream itself runs on a
    /// `tauri::async_runtime::spawn` task so the calling command handler
    /// never blocks on the LLM.
    pub fn send(self: &Arc<Self>, app: &AppHandle, deps: &Deps<'_>, text: &str) {
        self.kick(app, deps, text, false);
    }

    /// `Cmd+Shift+S` / empty-input ask: fixed prompt, frame REQUIRED.
    pub fn send_screen_only(self: &Arc<Self>, app: &AppHandle, deps: &Deps<'_>) {
        self.kick(app, deps, SCREEN_ONLY_PROMPT, true);
    }

    /// `ask_close`: cancel the in-flight stream (its `select!` arm emits
    /// the final `ask:state{idle}`), mint a fresh token for the next run,
    /// reset state, hide the panel. Emits nothing itself.
    ///
    /// Order matters: replace the token BEFORE flipping state to Idle —
    /// a `send` gated on `Idle` must never clone the cancelled token.
    pub fn close(&self, pool: &Mutex<WindowPool>) {
        self.cancel.lock().cancel();
        *self.cancel.lock() = CancellationToken::new();
        *self.state.lock() = AskState::Idle;
        pool.lock().hide(Panel::Ask);
    }

    /// Shared pre-flight + spawn behind `send`/`send_screen_only`.
    /// `frame_required` is the screen-only variant's no-frame→error rule.
    fn kick(self: &Arc<Self>, app: &AppHandle, deps: &Deps<'_>, text: &str, frame_required: bool) {
        let gen = {
            let mut state = self.state.lock();
            if *state != AskState::Idle {
                log::warn!("ask::send: busy ({state:?}); ignoring new send");
                return;
            }
            // State flip and generation bump share one critical section —
            // the emit closure's guarded fold can then never interleave
            // (a stale emit lands before this lock or sees the new gen).
            *state = AskState::Loading;
            self.generation.fetch_add(1, Ordering::SeqCst) + 1
        };
        *self.current_question.lock() = text.to_string();
        // Show first so any pre-flight error still renders in the panel.
        deps.pool.lock().show(Panel::Ask);

        // The failover chain: `providers.order` minus disabled/unusable.
        // An empty chain is the "no usable provider" error — nothing to
        // fall back TO, so the panel links straight to settings.
        let candidates = {
            let cfg = deps.config.lock();
            let ks = deps.keystore.lock();
            crate::provider_candidates(&cfg, &ks)
        };
        if candidates.is_empty() {
            return self.pre_spawn_error(
                app,
                text,
                json!({
                    "message": "No AI provider is configured — add a key in Settings → Providers",
                    "needs_setup": true,
                }),
            );
        }
        let mut frame = deps.ring.lock().latest();
        // Mid-session screen-permission revocation: SCStream stops
        // delivering but the ring keeps serving its last frames — a
        // stale screenshot silently shipped is worse than no image.
        // Emit the spec'd signal and answer text-only instead.
        if frame.is_some() && !crate::permissions::screen_status() {
            let _ = app.emit(
                "capture:permission-needed",
                json!({ "permission": "screen" }),
            );
            frame = None;
        }
        if frame_required && frame.is_none() {
            return self.pre_spawn_error(
                app,
                text,
                json!({"message": "No frame captured — check screen permission"}),
            );
        }

        let cancel = self.cancel.lock().clone();
        let svc = Arc::clone(self);
        let app = app.clone();
        let db = Arc::clone(&deps.db);
        let text = text.to_string();
        tauri::async_runtime::spawn(async move {
            // Outgoing events also fold into Rust-side state — but only
            // while this run is the current generation; a cancelled
            // task's trailing emits must not clobber a newer send.
            let emit = move |name: &str, payload: serde_json::Value| {
                {
                    // The generation check and the `ask:state` fold share
                    // this critical section: a stale task either lands
                    // its emit BEFORE `kick`'s atomic flip+bump, or sees
                    // the new generation and drops. Check-then-act across
                    // separate locks would let a cancelled task clobber a
                    // newer send's state.
                    let mut state = svc.state.lock();
                    if svc.generation.load(Ordering::SeqCst) != gen {
                        return;
                    }
                    if name == EV_STATE {
                        *state = match payload["state"].as_str() {
                            Some("loading") => AskState::Loading,
                            Some("streaming") => AskState::Streaming,
                            _ => AskState::Idle,
                        };
                    }
                }
                svc.observe(name, &payload);
                let _ = app.emit_to("ask", name, payload);
            };
            let _ = send_chain(
                candidates,
                db.as_ref(),
                &emit,
                &text,
                frame.as_ref(),
                &cancel,
            )
            .await;
        });
    }

    /// Early-exit error (empty provider chain, no frame):
    /// the busy check already flipped state to Loading — emit the same
    /// `ask:state{loading}` → `ask:error` → `ask:state{idle}` sequence
    /// `send_with`'s failure path uses (the `loading` carries the
    /// question so the panel's run-reset/header work here too), then
    /// reset Rust-side state to match.
    fn pre_spawn_error(&self, app: &AppHandle, text: &str, payload: serde_json::Value) {
        let _ = app.emit_to(
            "ask",
            EV_STATE,
            json!({"state": "loading", "question": text}),
        );
        let _ = app.emit_to("ask", EV_ERROR, payload);
        *self.state.lock() = AskState::Idle;
        let _ = app.emit_to("ask", EV_STATE, json!({"state": "idle"}));
    }

    /// Fold one outgoing event into Rust-side state: `ask:state` is
    /// folded by the emit closure under its generation-guarded lock (the
    /// busy-check source of truth); `ask:done` records the reply here.
    fn observe(&self, name: &str, payload: &serde_json::Value) {
        if name == EV_DONE {
            if let Some(full) = payload["full"].as_str() {
                *self.current_response.lock() = full.to_string();
            }
        }
    }
}

impl Default for AskService {
    fn default() -> Self {
        Self::new()
    }
}

/// The testable core: persist → walk the failover chain → persist,
/// emitting the `ask:*` protocol through `emit`. No `AppHandle`/keystore/
/// pool inside — [`AskService::send`] gathers those deps and delegates.
///
/// Order of operations:
/// 1. Persist the user message FIRST — it was already sent, so it must
///    be recorded even when every candidate later errors or is cancelled.
/// 2. `ask:state{loading}` once (the chain is one run).
/// 3. Each [`ProviderCandidate`] streams via [`stream_candidate`]:
///    `Done` → persist assistant + `ask:done{full, provider, model}` +
///    `ask:state{idle}`; `Failed` → warn-log, re-emit `loading` (the
///    panel resets its buffer — a dead provider's partial chunks must
///    not bleed into the next attempt), and try the NEXT candidate;
///    `Cancelled` → `ask:state{idle}` and stop immediately — the user
///    asked to stop, so no failover may start a new request.
/// 4. Every candidate failed → `ask:error{message}` (the LAST failure's
///    message — the most actionable one) + `ask:state{idle}`.
///
/// Db failures are `log::warn`ed and ignored — a storage hiccup must
/// never block the stream.
///
/// Resolves to the full assistant text. Cancel resolves to a
/// `status:0`/`"cancelled"` [`LlmError::Http`] sentinel — the events, not
/// the return value, drive the UI.
pub(crate) async fn send_chain(
    candidates: Vec<ProviderCandidate>,
    db: &Db,
    emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync),
    text: &str,
    frame: Option<&Frame>,
    cancel: &CancellationToken,
) -> Result<String, LlmError> {
    let session_id = persist_user_message(db, text);
    emit(EV_STATE, json!({"state": "loading", "question": text}));

    let mut last_err: Option<LlmError> = None;
    for (i, cand) in candidates.iter().enumerate() {
        // A failover hand-off re-announces `loading` so the panel drops
        // the failed attempt's partial chunks before the next stream.
        if i > 0 {
            emit(EV_STATE, json!({"state": "loading", "question": text}));
        }
        match stream_candidate(&*cand.provider, emit, text, frame, cancel).await {
            CandidateOutcome::Done(full) => {
                persist_assistant_message(db, session_id, &full);
                emit(
                    EV_DONE,
                    json!({"full": full, "provider": cand.id, "model": cand.model}),
                );
                emit(EV_STATE, json!({"state": "idle"}));
                return Ok(full);
            }
            // User row stays — it was already sent. Never fall over on
            // cancel: the user asked to stop, so the chain stops here.
            CandidateOutcome::Cancelled => {
                emit(EV_STATE, json!({"state": "idle"}));
                return Err(LlmError::Http {
                    status: 0,
                    message: "cancelled".to_string(),
                });
            }
            CandidateOutcome::Failed(e) => {
                log::warn!("ask: provider {} failed ({e}); trying next", cand.id);
                last_err = Some(e);
            }
        }
    }
    // Chain exhausted — surface the last failure (most actionable).
    let e = last_err.unwrap_or(LlmError::NoModel);
    emit(EV_ERROR, json!({"message": e.to_string()}));
    emit(EV_STATE, json!({"state": "idle"}));
    Err(e)
}

/// One candidate's full attempt: stream, and on a
/// `MultimodalUnsupported` rejection retry ONCE text-only. Emits
/// `ask:chunk`/`ask:state{streaming}` but never `done`/`error`/`idle` —
/// the chain owns the run's protocol; this owns one provider's messages.
async fn stream_candidate(
    provider: &dyn Provider,
    emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync),
    text: &str,
    frame: Option<&Frame>,
    cancel: &CancellationToken,
) -> CandidateOutcome {
    let mut streaming = false;
    let mut msgs = build_messages(text, frame);
    let mut retried = false;
    loop {
        match stream_once(provider, &msgs, emit, cancel, &mut streaming).await {
            StreamOutcome::Done(full) => return CandidateOutcome::Done(full),
            StreamOutcome::Cancelled => return CandidateOutcome::Cancelled,
            StreamOutcome::Failed(e) => {
                // Vision-incapable model gets ONE retry without the frame.
                if !retried && frame.is_some() && e.is_multimodal() {
                    retried = true;
                    msgs = build_messages(text, None);
                    continue;
                }
                return CandidateOutcome::Failed(e);
            }
        }
    }
}

/// One provider's result within the chain — `Failed` hands off to the
/// next candidate; `Cancelled`/`Done` end the run.
enum CandidateOutcome {
    Done(String),
    Cancelled,
    Failed(LlmError),
}

/// One `stream_chat` attempt's result.
enum StreamOutcome {
    Done(String),
    /// `cancel` fired — the request future was dropped mid-flight.
    Cancelled,
    Failed(LlmError),
}

/// Race one `stream_chat` against cancellation. Emits
/// `ask:state{streaming}` on the FIRST token (`streaming` persists across
/// the multimodal retry so it fires at most once per send) and
/// `ask:chunk` per token. `on_token` re-checks `is_cancelled` itself —
/// a token racing the cancellation must never reach the webview.
async fn stream_once(
    provider: &dyn Provider,
    msgs: &[ChatMessage],
    emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync),
    cancel: &CancellationToken,
    streaming: &mut bool,
) -> StreamOutcome {
    let mut on_token = |token: &str| {
        if cancel.is_cancelled() {
            return;
        }
        if !*streaming {
            *streaming = true;
            emit(EV_STATE, json!({"state": "streaming"}));
        }
        emit(EV_CHUNK, json!({"text": token}));
    };
    tokio::select! {
        _ = cancel.cancelled() => StreamOutcome::Cancelled,
        r = provider.stream_chat(msgs, &mut on_token) => match r {
            Ok(full) => StreamOutcome::Done(full),
            Err(e) => StreamOutcome::Failed(e),
        },
    }
}

/// `[system, user]` — the user message pairs `text` with the frame's
/// JPEG when one was captured; text-only otherwise (and on the retry).
fn build_messages(text: &str, frame: Option<&Frame>) -> Vec<ChatMessage> {
    vec![
        ChatMessage::text(Role::System, system_prompt("")),
        match frame {
            Some(f) => ChatMessage::user_with_image(text, f.jpeg.clone()),
            None => ChatMessage::text(Role::User, text),
        },
    ]
}

/// Session row + the user message. `None` when the session lookup itself
/// failed — the assistant row then has nowhere to go either.
fn persist_user_message(db: &Db, text: &str) -> Option<i64> {
    let sid = match db.session_get_or_create_active("ask") {
        Ok(sid) => sid,
        Err(e) => {
            log::warn!("ask: session_get_or_create_active failed: {e}");
            return None;
        }
    };
    if let Err(e) = db.ai_message_add(sid, "user", text) {
        log::warn!("ask: failed to persist user message: {e}");
    }
    Some(sid)
}

/// The completed assistant reply, next to its user row.
fn persist_assistant_message(db: &Db, session_id: Option<i64>, full: &str) {
    let Some(sid) = session_id else {
        return;
    };
    if let Err(e) = db.ai_message_add(sid, "assistant", full) {
        log::warn!("ask: failed to persist assistant message: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ContentPart;
    use std::collections::VecDeque;
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::atomic::AtomicU32;
    use std::time::Duration;

    /// Scripted provider: each `stream_chat` call pops the next behaviour
    /// and records the messages it was given. Fields are `Arc`-shared so
    /// a test keeps its assertion handle after the provider is boxed
    /// into a [`ProviderCandidate`].
    enum Behavior {
        /// Emit these tokens, then resolve `Ok(concat)`.
        Tokens(Vec<String>),
        /// Resolve with this error immediately.
        Fail(LlmError),
        /// Never resolve — exercises cancellation.
        Hang,
    }

    struct MockProvider {
        script: Arc<Mutex<VecDeque<Behavior>>>,
        calls: Arc<Mutex<Vec<Vec<ChatMessage>>>>,
    }

    impl MockProvider {
        fn new(script: Vec<Behavior>) -> Self {
            Self {
                script: Arc::new(Mutex::new(script.into())),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn calls(&self) -> Arc<Mutex<Vec<Vec<ChatMessage>>>> {
            Arc::clone(&self.calls)
        }
    }

    /// Wrap a mock in the chain's candidate shape — the id/model ride
    /// into `ask:done` so tests can assert which provider answered.
    fn candidate(id: &str, provider: MockProvider) -> ProviderCandidate {
        ProviderCandidate {
            id: id.to_string(),
            model: "mock-model".to_string(),
            provider: Box::new(provider),
        }
    }

    impl Provider for MockProvider {
        fn stream_chat<'a>(
            &'a self,
            msgs: &'a [ChatMessage],
            on_token: &'a mut (dyn FnMut(&str) + Send),
        ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send + 'a>> {
            self.calls.lock().push(msgs.to_vec());
            let behavior = self
                .script
                .lock()
                .pop_front()
                .unwrap_or(Behavior::Tokens(Vec::new()));
            Box::pin(async move {
                match behavior {
                    Behavior::Tokens(tokens) => {
                        let mut full = String::new();
                        for t in &tokens {
                            on_token(t);
                            full.push_str(t);
                        }
                        Ok(full)
                    }
                    Behavior::Fail(e) => Err(e),
                    Behavior::Hang => std::future::pending().await,
                }
            })
        }

        fn validate<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = Result<(), LlmError>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    type Events = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

    /// Recording emitter — the `send_chain` seam stands in for
    /// `app.emit_to`.
    fn recorder() -> (Events, impl Fn(&str, serde_json::Value) + Send + Sync) {
        let events: Events = Arc::new(Mutex::new(Vec::new()));
        let ev = Arc::clone(&events);
        let emit = move |name: &str, payload: serde_json::Value| {
            ev.lock().push((name.to_string(), payload));
        };
        (events, emit)
    }

    fn ev(name: &str, payload: serde_json::Value) -> (String, serde_json::Value) {
        (name.to_string(), payload)
    }

    /// Unique temp dir per test; `Db::at` creates it.
    fn tmp_dir() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "marvis-ask-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn fake_frame() -> Frame {
        Frame {
            jpeg: vec![0xff, 0xd8, 0xff, 0xe0],
            width: 4,
            height: 4,
            ts: 0,
            hash: 1,
        }
    }

    fn ask_messages(db: &Db) -> Vec<crate::storage::AiMessage> {
        let sid = db.session_get_or_create_active("ask").unwrap();
        db.ai_messages_for(sid).unwrap()
    }

    fn has_image(msg: &ChatMessage) -> bool {
        msg.content
            .iter()
            .any(|p| matches!(p, ContentPart::ImageJpeg(_)))
    }

    #[tokio::test]
    async fn send_chain_streams_ordered_chunks_and_persists_both_messages() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let provider = MockProvider::new(vec![Behavior::Tokens(vec![
            "Hello".into(),
            " ".into(),
            "world".into(),
        ])]);
        let calls = provider.calls();
        let (events, emit) = recorder();
        let frame = fake_frame();
        let cancel = CancellationToken::new();

        let full = send_chain(
            vec![candidate("openai", provider)],
            &db,
            &emit,
            "what is this?",
            Some(&frame),
            &cancel,
        )
        .await
        .unwrap();
        assert_eq!(full, "Hello world");

        // Protocol order: loading → streaming-on-first-token → ordered
        // chunks → done{full, provider, model} → idle.
        let got = events.lock().clone();
        assert_eq!(
            got,
            vec![
                ev(
                    EV_STATE,
                    json!({"state": "loading", "question": "what is this?"})
                ),
                ev(EV_STATE, json!({"state": "streaming"})),
                ev(EV_CHUNK, json!({"text": "Hello"})),
                ev(EV_CHUNK, json!({"text": " "})),
                ev(EV_CHUNK, json!({"text": "world"})),
                ev(
                    EV_DONE,
                    json!({"full": "Hello world", "provider": "openai", "model": "mock-model"})
                ),
                ev(EV_STATE, json!({"state": "idle"})),
            ]
        );

        // Both rows landed in the 'ask' session, user first.
        let msgs = ask_messages(&db);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "what is this?");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "Hello world");

        // One provider call, and its user message carried the image.
        let calls = calls.lock();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][1].role, Role::User);
        assert!(has_image(&calls[0][1]));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn send_chain_fails_over_to_the_next_provider() {
        // The whole point of the chain: a dead provider hands off.
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let first = MockProvider::new(vec![Behavior::Fail(LlmError::Auth)]);
        let second = MockProvider::new(vec![Behavior::Tokens(vec!["ok".into()])]);
        let first_calls = first.calls();
        let second_calls = second.calls();
        let (events, emit) = recorder();
        let cancel = CancellationToken::new();

        let full = send_chain(
            vec![candidate("openai", first), candidate("gemini", second)],
            &db,
            &emit,
            "q",
            None,
            &cancel,
        )
        .await
        .unwrap();
        assert_eq!(full, "ok");
        assert_eq!(first_calls.lock().len(), 1);
        assert_eq!(second_calls.lock().len(), 1);

        // loading → (fail) → loading reset → streaming → done naming the
        // SECOND provider — no ask:error between the attempts.
        let got = events.lock().clone();
        assert_eq!(
            got,
            vec![
                ev(EV_STATE, json!({"state": "loading", "question": "q"})),
                ev(EV_STATE, json!({"state": "loading", "question": "q"})),
                ev(EV_STATE, json!({"state": "streaming"})),
                ev(EV_CHUNK, json!({"text": "ok"})),
                ev(
                    EV_DONE,
                    json!({"full": "ok", "provider": "gemini", "model": "mock-model"})
                ),
                ev(EV_STATE, json!({"state": "idle"})),
            ]
        );
        // One assistant row, from the provider that answered.
        let msgs = ask_messages(&db);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].content, "ok");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn send_chain_all_candidates_failing_emits_the_last_error() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let first = MockProvider::new(vec![Behavior::Fail(LlmError::Auth)]);
        let second = MockProvider::new(vec![Behavior::Fail(LlmError::Http {
            status: 500,
            message: "boom".into(),
        })]);
        let (events, emit) = recorder();
        let cancel = CancellationToken::new();

        let err = send_chain(
            vec![candidate("openai", first), candidate("gemini", second)],
            &db,
            &emit,
            "q",
            None,
            &cancel,
        )
        .await
        .unwrap_err();
        // The LAST failure's message surfaces — it's the most actionable.
        assert_eq!(err.to_string(), "http 500: boom");
        assert_eq!(
            events.lock().clone(),
            vec![
                ev(EV_STATE, json!({"state": "loading", "question": "q"})),
                ev(EV_STATE, json!({"state": "loading", "question": "q"})),
                ev(EV_ERROR, json!({"message": "http 500: boom"})),
                ev(EV_STATE, json!({"state": "idle"})),
            ]
        );
        assert_eq!(ask_messages(&db).len(), 1); // user row only
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn send_chain_cancel_never_falls_over() {
        // Cancel mid-first-candidate → the chain stops; candidate two is
        // never even called.
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let first = MockProvider::new(vec![Behavior::Hang]);
        let second = MockProvider::new(vec![Behavior::Tokens(vec!["nope".into()])]);
        let second_calls = second.calls();
        let (events, emit) = recorder();
        let cancel = CancellationToken::new();

        let c2 = cancel.clone();
        let cancels = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            c2.cancel();
        });
        let err = send_chain(
            vec![candidate("openai", first), candidate("gemini", second)],
            &db,
            &emit,
            "q",
            None,
            &cancel,
        )
        .await
        .unwrap_err();
        cancels.await.unwrap();
        assert_eq!(err.to_string(), "http 0: cancelled");
        assert_eq!(second_calls.lock().len(), 0);
        assert!(events.lock().iter().all(|(n, _)| n != EV_ERROR));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn send_chain_multimodal_error_retries_text_only_once() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let provider = MockProvider::new(vec![
            Behavior::Fail(LlmError::MultimodalUnsupported),
            Behavior::Tokens(vec!["answer".into()]),
        ]);
        let calls = provider.calls();
        let (events, emit) = recorder();
        let frame = fake_frame();
        let cancel = CancellationToken::new();

        let full = send_chain(
            vec![candidate("openai", provider)],
            &db,
            &emit,
            "q",
            Some(&frame),
            &cancel,
        )
        .await
        .unwrap();
        assert_eq!(full, "answer");

        // Call 1 carried the image; the retry must be text-only.
        let calls = calls.lock();
        assert_eq!(calls.len(), 2);
        assert!(has_image(&calls[0][1]));
        assert!(!has_image(&calls[1][1]));
        assert_eq!(
            calls[1][1].content,
            vec![ContentPart::Text("q".to_string())]
        );

        // Successful retry: no ask:error; done still emitted.
        assert!(events.lock().iter().all(|(n, _)| n != EV_ERROR));
        assert!(events.lock().iter().any(|(n, _)| n == EV_DONE));

        let msgs = ask_messages(&db);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "answer");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn send_chain_second_multimodal_error_surfaces_normally() {
        // Retry fails multimodal too → one retry only, then ask:error.
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let provider = MockProvider::new(vec![
            Behavior::Fail(LlmError::MultimodalUnsupported),
            Behavior::Fail(LlmError::MultimodalUnsupported),
        ]);
        let calls = provider.calls();
        let (events, emit) = recorder();
        let frame = fake_frame();
        let cancel = CancellationToken::new();

        let err = send_chain(
            vec![candidate("openai", provider)],
            &db,
            &emit,
            "q",
            Some(&frame),
            &cancel,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, LlmError::MultimodalUnsupported));
        assert_eq!(calls.lock().len(), 2);

        let got = events.lock().clone();
        assert_eq!(
            got,
            vec![
                ev(EV_STATE, json!({"state": "loading", "question": "q"})),
                ev(
                    EV_ERROR,
                    json!({"message": LlmError::MultimodalUnsupported.to_string()})
                ),
                ev(EV_STATE, json!({"state": "idle"})),
            ]
        );
        assert_eq!(ask_messages(&db).len(), 1); // user row only

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn send_chain_cancel_mid_stream_persists_user_only() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let provider = MockProvider::new(vec![Behavior::Hang]);
        let (events, emit) = recorder();
        let frame = fake_frame();
        let cancel = CancellationToken::new();

        // Cancel while the (never-resolving) stream is in-flight — the
        // select! arm drops the request future mid-flight.
        let c2 = cancel.clone();
        let cancels = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            c2.cancel();
        });
        let err = send_chain(
            vec![candidate("openai", provider)],
            &db,
            &emit,
            "q",
            Some(&frame),
            &cancel,
        )
        .await
        .unwrap_err();
        cancels.await.unwrap();
        assert_eq!(err.to_string(), "http 0: cancelled");

        // loading → idle only: no streaming/chunk/done/error.
        let got = events.lock().clone();
        assert_eq!(
            got,
            vec![
                ev(EV_STATE, json!({"state": "loading", "question": "q"})),
                ev(EV_STATE, json!({"state": "idle"})),
            ]
        );

        // The user row persists (already sent); no assistant row.
        let msgs = ask_messages(&db);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "q");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn send_chain_no_frame_sends_single_text_part() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let provider = MockProvider::new(vec![Behavior::Tokens(vec!["ok".into()])]);
        let calls = provider.calls();
        let (_events, emit) = recorder();
        let cancel = CancellationToken::new();

        send_chain(
            vec![candidate("openai", provider)],
            &db,
            &emit,
            "q",
            None,
            &cancel,
        )
        .await
        .unwrap();

        let calls = calls.lock();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0][1].content,
            vec![ContentPart::Text("q".to_string())]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn send_chain_exhausted_single_candidate_emits_error_and_idle() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let provider = MockProvider::new(vec![Behavior::Fail(LlmError::Auth)]);
        let calls = provider.calls();
        let (events, emit) = recorder();
        let frame = fake_frame();
        let cancel = CancellationToken::new();

        let err = send_chain(
            vec![candidate("openai", provider)],
            &db,
            &emit,
            "q",
            Some(&frame),
            &cancel,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, LlmError::Auth));

        // Auth is not multimodal — no retry even with a frame attached.
        assert_eq!(calls.lock().len(), 1);
        let got = events.lock().clone();
        assert_eq!(
            got,
            vec![
                ev(EV_STATE, json!({"state": "loading", "question": "q"})),
                ev(EV_ERROR, json!({"message": LlmError::Auth.to_string()})),
                ev(EV_STATE, json!({"state": "idle"})),
            ]
        );
        assert_eq!(ask_messages(&db).len(), 1); // user row only

        let _ = std::fs::remove_dir_all(&dir);
    }
}
