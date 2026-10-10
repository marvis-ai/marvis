//! `ask` — question + optional screen material → streaming LLM → persisted.
//!
//! Pipeline (spec §Ask): [`AskService::send`] expands the card,
//! resolves the FAILOVER CHAIN (`providers.order` minus disabled/unusable
//! — see `provider_candidates` in lib.rs), then [`resolve_screen`] picks
//! the run's screen material by the truth table: recording ON → the
//! reader's cached description (with `[vision]` — see `vision_candidate`
//! in lib.rs) or the freshest [`RingBuffer`] frame; recording OFF with
//! screen intent (`with_screen` flag, `looks_like_screen_intent`, or the
//! screen-only button) → a one-shot screenshot, described inline when a
//! reader is configured else attached raw; OFF without intent →
//! text-only. Each candidate then streams in turn: a failed provider
//! logs and hands off to the next; only when every candidate fails does
//! `ask:error` fire. A text description rides a `<screen_context>`
//! block, so chat providers never need image support.
//! Both sides of the exchange land in the `ask` session's `messages`
//! rows (the user row persists once, before the first attempt). The
//! first answered send on a still-untitled session also names it — the
//! answering provider condenses the question into `sessions.title` via
//! a detached sidecar spawned after `ask:state{idle}` (history shows
//! the first-question fallback until it lands, then `sessions:changed`
//! refreshes it; a `stop` can't recall it, so an ended session still
//! gets named). A provider `MultimodalUnsupported` rejection retries
//! once text-only (per attempt): a screen frame is simply dropped,
//! while the user's own attachments are first described by the
//! `[vision]` reader into an `<attached_images>` block — never
//! silently discarded.
//!
//! Runs are PER-SESSION and concurrent: [`AskService::send`] resolves
//! the run's session up-front, registers it in the `runs` table, and
//! refuses only a send aimed at a session that is ALREADY running —
//! every other session streams in parallel, each bound to its own
//! rows and `session_id`. [`AskService::close`] collapses the card
//! and nothing else — leaving a view never kills a run: the
//! composer's `ask_stop` cancels ONE session's run via its
//! [`CancellationToken`], and `retire_all` (the `leave_main`
//! teardown) cancels them all.
//!
//! Event protocol (emitted to the `bar` window via `app.emit_to`):
//! - `ask:state` `{"state": "loading"|"streaming"|"idle"}` — `streaming`
//!   fires on each attempt's FIRST token; every `loading` carries
//!   `"question"` so the card resets its buffer + header per run AND
//!   per failover retry (pre-flight errors emit `loading` → `error` →
//!   `idle` too). The run-boundary fields tell the fold WHAT the emit
//!   is without text-matching: `"attempt"` is the 0-based failover
//!   index (>0 marks a retry of the same run, never a new turn) and
//!   `"regenerate"` marks `ask_retry`'s re-ask — a same-text re-send
//!   is attempt 0 of a fresh run, so it appends a second pair just
//!   like the persisted history shows. `send_chain`'s `loading`s also
//!   carry `"preset"` — the armed `instruct` preset id, `null` for a
//!   plain send or an id that didn't resolve.
//! - `ask:chunk` `{"text": token}` per token.
//! - `ask:done` `{"full": full_reply, "provider": id, "model": id,
//!   "usage": {"input": n?, "output": n?} | null}` on success — the pair
//!   that actually answered plus the run's reported token spend.
//! - `ask:error` `{"message": ..., "needs_setup": bool?}` on failure —
//!   `needs_setup` when the chain was empty (no usable provider at all).
//!
//! Every `ask:*` payload carries `"run"` — the run's `generation` — so
//! the webview can drop packets a stopped/superseded run emitted
//! before the cancel landed (an emit already in the IPC pipe can't be
//! recalled),
//! and `"session_id"` — the session the run writes into — so a run that
//! outlives its view (New Chat / resume don't kill it) never paints
//! into a conversation that isn't showing that session.
//!
//! Broadcast (to the `bar` window, the toast's only consumer):
//! `capture:permission-needed` when a ring frame exists but screen
//! permission was revoked mid-session — the stale frame is dropped and
//! the ask continues text-only.
//!
//! Wiring note for Task 14: `send`/`send_screen_only`/`close` take a
//! [`Deps`] bundle of `AppState` fields so this module never names the
//! not-yet-existing `AppState` type. `deps.db`/`deps.ring`/`deps.reader`
//! must be [`Arc`]s — the stream runs on a spawned task that outlives the
//! command call — and `AppState.ask` must be an `Arc<AskService>` (all
//! fields are interior-mutable; the spawned task keeps a share). After
//! synchronous pre-flight, real work runs inside
//! `tauri::async_runtime::spawn` so the invoking command handler returns
//! immediately instead of blocking on an LLM stream.

mod attachments;
mod compact;
mod history;
mod pipeline;
mod screen;
mod stream;
mod title;

pub(crate) use self::compact::CompactService;
use self::compact::{compaction_plan, CompactHook, CompactionPlan};
use self::{pipeline::*, screen::*, title::TitleSidecar};

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use crate::capture::{Frame, RingBuffer};
use crate::config::Config;
use crate::keystore::Keystore;
use crate::llm::{ChatMessage, ContentPart, LlmError, Provider, Role, StreamReply, TokenUsage};
use crate::memory::{MemoryHook, MemoryService};
use crate::prompts::{live_system_prompt_with_profile, live_user_prompt};
use crate::screen_read;
use crate::storage::{Db, Message, MessageAttachment, MessageMeta, NewAttachment, Transcript};
use crate::windows::{WindowPool, BAR_LABEL};
use crate::ProviderCandidate;

/// Event names — part of the webview contract; change together with
/// `src/lib/events.ts`.
const EV_STATE: &str = "ask:state";
const EV_CHUNK: &str = "ask:chunk";
const EV_DONE: &str = "ask:done";
const EV_ERROR: &str = "ask:error";
/// A generated session title landed — the history list re-reads to swap
/// its first-question fallback.
const EV_SESSIONS_CHANGED: &str = "sessions:changed";

/// `send_screen_only`'s fixed question — the camera button's ask.
const SCREEN_ONLY_PROMPT: &str = "Describe what is on my screen and how you can help.";

/// The sidecar call that names a still-untitled session. The title's
/// only evidence is the question itself, so it gets the raw text, not
/// the context-wrapped wire prompt.
const TITLE_PROMPT: &str = "Write a very short title for the conversation this question starts — 6 words or fewer, in the question's language. Reply with the title only: no quotes, no trailing punctuation.";
/// A pasted giant first question still titles from its head — bound the
/// sidecar's input instead of riding the request whole.
const TITLE_QUESTION_CAP: usize = 1000;
/// `stream_chat` bounds only the connect — a hung title call would leak
/// the run's task, so the sidecar gets a total deadline too.
const TITLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Context window: only the trailing N persisted `messages` ride
/// along with each ask (spec: last 20, text-only).
const HISTORY_TAIL: usize = 20;

const COMPACT_BATCH: usize = 10;
const MAX_COMPACT_CHARS: usize = 2_000;
const MAX_COMPACT_ROW_CHARS: usize = 1_000;
const MAX_COMPACT_INPUT_BYTES: usize = 32_000;
const COMPACT_TIMEOUT: Duration = Duration::from_secs(30);

/// Composer images per send — matches the webview cap in
/// `image-attachments.ts`; a crafted invoke beyond it is rejected
/// before the user row persists.
const MAX_ATTACHMENTS: usize = 4;

/// Decoded-payload bound — the webview's 20 MiB source limit can only
/// shrink under JPEG re-encode at 2048px, so this is a generous
/// malformed-input ceiling, not a real budget.
const MAX_ATTACHMENT_BYTES: usize = 24 * 1024 * 1024;

/// One normalized image attached to an `ask_send` invoke — `name` is
/// the original filename, display metadata only (never a storage
/// path); `jpeg_base64` is the webview-normalized JPEG payload with no
/// `data:` prefix.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAttachmentInput {
    pub name: String,
    pub jpeg_base64: String,
}

/// Lifecycle of one ask run. Mirrors the `ask:state` event so Rust-side
/// status reads see exactly what the webview sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskState {
    Idle,
    Loading,
    Streaming,
}

impl AskState {
    /// The `ask:state` / `ask_current` wire value.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Loading => "loading",
            Self::Streaming => "streaming",
        }
    }
}

/// The `AppState` fields the ask pipeline needs, bundled so this module
/// compiles before `AppState` exists. `db`/`ring`/`reader`/`memory`/
/// `config` are owned `Arc`s (the spawned stream task outlives the
/// call — `config`'s share is how the scheduled extraction re-reads
/// `[memory].enabled`); the rest are locked only during synchronous
/// pre-flight, so plain `&Mutex` borrows suffice.
pub struct Deps<'a> {
    pub db: Arc<Db>,
    pub ring: Arc<Mutex<RingBuffer>>,
    /// The ambient screen reader — `resolve_screen` serves its cached
    /// context while recording runs.
    pub reader: Arc<screen_read::ScreenReader>,
    /// `state.capture` is live — ring frames are fresh.
    pub capture_running: bool,
    /// The consent-gated extraction service — Arc'd because the
    /// scheduled `MemoryHook` outlives this borrow.
    pub memory: Arc<MemoryService>,
    /// The live config share — `kick` snapshots it for the chain AND
    /// hands the memory hook a closure re-reading `[memory].enabled`
    /// at extraction time.
    pub config: Arc<Mutex<Config>>,
    pub keystore: &'a Mutex<Keystore>,
    pub pool: &'a Mutex<WindowPool>,
}

/// One send's inputs — what `send`/`send_screen_only`/`retry` hand to
/// `kick` (bundled the same way `Deps` bundles the environment).
/// `with_screen` is the explicit attach flag (`Cmd+Enter`/`withScreen`
/// invoke arg); `screen_required` is the screen-only variant's
/// failed-shot→error rule; `regenerate` is `retry`'s
/// re-ask-the-last-turn mode; `listen_id` binds the send to a listen
/// doc's own ask session (`send` only — a retry inherits the link from
/// its session row); `preset` is the armed `instruct` preset's id
/// (`presetId` invoke arg) — resolved to its text inside `kick`; only a
/// RESOLVED id persists on the user row (so `retry` re-arms it) and
/// rides the `loading` emits.
#[derive(Default)]
pub(crate) struct SendOpts<'a> {
    pub text: &'a str,
    pub with_screen: bool,
    pub screen_required: bool,
    pub regenerate: bool,
    pub listen_id: Option<i64>,
    /// The armed preset id — resolved against the catalog +
    /// `prompts.custom` in `kick`; `None` for a plain send. A resolvable
    /// id persists on the user row either way (the `· name` provenance
    /// covers expanding presets too); only a non-`{input}` preset
    /// contributes instruction text.
    pub preset: Option<String>,
    /// The `{lang}` param badge's per-send value — substitutes the
    /// preset text's `{lang}` placeholder; `None` resolves it to the
    /// configured main language.
    pub preset_lang: Option<String>,
    /// The composer's pending images — normalized JPEG base64 payloads;
    /// empty on screen-only sends and retries (a retried turn re-reads
    /// its persisted row's attachments instead).
    pub attachments: Vec<AskAttachmentInput>,
}

/// One in-flight ask — everything a single send streams into. Runs are
/// keyed by the session they write into: multi-chat is a map of these,
/// not one slot.
pub(crate) struct Run {
    /// The run's generation — rides every `ask:*` packet as `run` and
    /// tags `ask_runs` snapshots.
    gen: u64,
    /// Cancel handle — `ask_stop` kills this run's chain; every other
    /// session's run streams on untouched.
    cancel: CancellationToken,
    /// Folded `ask:state` — the per-session busy flag a same-session
    /// send checks (an `idle` entry never blocks the next send).
    state: AskState,
    /// The run's submitted question (set at claim — the `loading`
    /// emits just echo it).
    question: String,
    /// Last completed reply — `ask_runs` folds the live tail onto a
    /// remounting chat.
    response: String,
    /// The user-turn attachments folded out of `loading` — resync so a
    /// re-expanded card renders the message's images.
    attachments: Vec<MessageAttachment>,
    /// The run's last `ask:error` — kept for `ask_runs` resyncs until a
    /// `loading` boundary supersedes it.
    error: Option<serde_json::Value>,
}

impl Run {
    /// Fold one emitted packet into this run's resync snapshot — the
    /// state word, the `loading` boundary (clears the stale error,
    /// picks up the user turn's persisted attachments), `done`'s
    /// authoritative reply, and `error` kept for resync.
    fn fold(&mut self, name: &str, payload: &serde_json::Value) {
        if name == EV_STATE {
            self.state = match payload["state"].as_str() {
                Some("loading") => AskState::Loading,
                Some("streaming") => AskState::Streaming,
                _ => AskState::Idle,
            };
            if payload["state"].as_str() == Some("loading") {
                self.error = None;
                if let Some(atts) = payload.get("attachments") {
                    if let Ok(atts) =
                        serde_json::from_value::<Vec<MessageAttachment>>(atts.clone())
                    {
                        self.attachments = atts;
                    }
                }
            }
        } else if name == EV_DONE {
            if let Some(full) = payload["full"].as_str() {
                self.response = full.to_string();
            }
        } else if name == EV_ERROR {
            self.error = Some(payload.clone());
        }
    }
}

/// Many asks at once — one live run per session. `AskService` no
/// longer holds a single busy slot: the send-time `claim` refuses only
/// a send aimed at a session that is ALREADY running, and the composer
/// shows its stop affordance per visible session.
pub struct AskService {
    /// Live and finished runs by session id (`None` = the run never
    /// resolved a session — a DB hiccup still streams). The busy check
    /// + `ask_stop` + `ask_runs` all key on this. Lock order: `runs` →
    /// `db` — session resolution inside the claim critical section may
    /// mint/end rows, but never the reverse.
    runs: Mutex<HashMap<Option<i64>, Run>>,
    /// The last sessionless `ask:error` — `pre_spawn_error` fires before
    /// any session exists, so its payload resyncs through `ask_runs`
    /// under a `null` session (the webview passes sessionless packets).
    orphan_error: Mutex<Option<serde_json::Value>>,
    /// Bumped per accepted `send`: a stale (stopped/superseded) task's
    /// trailing events are dropped instead of clobbering a newer run's
    /// fold — the value also tags every `ask:*` payload as `run`, the
    /// webview's dead-packet filter. Global and monotonic, so the
    /// webview's newest-seen bound works across sessions.
    generation: AtomicU64,
}

impl AskService {
    pub fn new() -> Self {
        Self {
            runs: Mutex::new(HashMap::new()),
            orphan_error: Mutex::new(None),
            generation: AtomicU64::new(0),
        }
    }

    /// The `ask_runs` resync payload — every run's snapshot plus the
    /// sessionless orphan-error entry when one is set. Each entry
    /// carries `session_id`/`run` so the webview folds a live tail
    /// only onto the session it is showing.
    pub fn runs_payload(&self) -> Vec<serde_json::Value> {
        let mut out: Vec<_> = self
            .runs
            .lock()
            .iter()
            .map(|(sid, run)| {
                json!({
                    "session_id": sid,
                    "run": run.gen,
                    "state": run.state.as_str(),
                    "question": run.question,
                    "response": run.response,
                    "error": run.error,
                    "attachments": run.attachments,
                })
            })
            .collect();
        if let Some(error) = self.orphan_error.lock().clone() {
            out.push(json!({
                "session_id": null,
                "state": AskState::Idle.as_str(),
                "question": "",
                "response": "",
                "error": error,
                "attachments": Vec::<MessageAttachment>::new(),
            }));
        }
        out
    }

    /// The per-session send gate, run under one `runs` critical section
    /// with session resolution: `Some(cancel)` registers the run —
    /// `None` means the session already owns a live run and the send is
    /// refused (a run on a DIFFERENT session never blocks).
    fn claim(
        &self,
        session_id: Option<i64>,
        question: &str,
        gen: u64,
    ) -> Option<CancellationToken> {
        let mut runs = self.runs.lock();
        if let Some(run) = runs.get(&session_id) {
            if run.state != AskState::Idle {
                log::warn!("ask::send: session busy; ignoring send");
                return None;
            }
        }
        let cancel = CancellationToken::new();
        runs.insert(
            session_id,
            Run {
                gen,
                cancel: cancel.clone(),
                state: AskState::Loading,
                question: question.to_string(),
                response: String::new(),
                attachments: Vec::new(),
                error: None,
            },
        );
        Some(cancel)
    }

    /// The emit-closure's per-run gate + fold, in one critical section:
    /// a packet from a stopped/superseded/never-registered run returns
    /// `false` (drop — a cancel or a newer send retired it); a live
    /// run's packet folds its state/response/attachments/error so
    /// `ask_runs` resyncs track it. A sessionless run registers under
    /// `None`, so its packets gate and fold the same way.
    fn fold_emit(
        &self,
        session_id: Option<i64>,
        gen: u64,
        name: &str,
        payload: &serde_json::Value,
    ) -> bool {
        let mut runs = self.runs.lock();
        let Some(run) = runs.get_mut(&session_id) else {
            return false;
        };
        if run.gen != gen {
            return false;
        }
        run.fold(name, payload);
        true
    }

    /// The service-emits fold — `kick_error`/`pre_spawn_error` packets
    /// have no run to gate against (they fire before one registers),
    /// so they must not gen-match the table: they fold straight into
    /// the orphan-error slot, the `ask_runs` resync entry, keeping the
    /// loading-clears / everything-else-keeps rule.
    fn fold_orphan(&self, name: &str, payload: &serde_json::Value) {
        match name {
            EV_ERROR => *self.orphan_error.lock() = Some(payload.clone()),
            EV_STATE if payload["state"].as_str() == Some("loading") => {
                *self.orphan_error.lock() = None;
            }
            _ => {}
        }
    }

    /// Remove a run from the table — `stop`'s bookkeeping half. The
    /// emit guard drops the retired run's stragglers the moment the
    /// entry is gone; the returned run's token cancels the chain and
    /// its `gen`/`state` decide whether a terminal `idle` is owed.
    fn retire(&self, session_id: Option<i64>) -> Option<Run> {
        self.runs.lock().remove(&session_id).map(|run| {
            run.cancel.cancel();
            run
        })
    }

    /// Ask a free-text question. `with_screen` is the explicit attach
    /// flag (`Cmd+Enter`/`withScreen` invoke arg) — it forces a screen
    /// read regardless of the text's intent heuristic. `listen_id`
    /// binds the send to a listen doc: the question lands in that doc's
    /// own ask session (reopened or minted — one chat per doc) and its
    /// summary+transcript becomes the meeting context. `attachments`
    /// are the composer's normalized JPEGs — decoded and persisted
    /// before the first provider call. A send into a session whose own
    /// run is still live is ignored (warn-logged) — runs on OTHER
    /// sessions never block.
    ///
    /// Returns `false` only when the send was refused on the
    /// same-session busy rule — the composer keeps the draft; every
    /// other outcome (spawned, or a pre-flight error the card renders)
    /// counts as accepted. The stream itself runs on a
    /// `tauri::async_runtime::spawn` task so the calling command handler
    /// never blocks on the LLM.
    #[allow(clippy::too_many_arguments)]
    pub fn send(
        self: &Arc<Self>,
        app: &AppHandle,
        deps: &Deps<'_>,
        text: &str,
        with_screen: bool,
        listen_id: Option<i64>,
        preset: Option<String>,
        preset_lang: Option<String>,
        attachments: Vec<AskAttachmentInput>,
    ) -> bool {
        self.kick(
            app,
            deps,
            SendOpts {
                text,
                with_screen,
                listen_id,
                preset,
                preset_lang,
                attachments,
                ..SendOpts::default()
            },
        )
    }

    /// The camera button's screen-only ask: fixed prompt, screen
    /// REQUIRED — a failed one-shot errors instead of degrading to
    /// text-only.
    pub fn send_screen_only(self: &Arc<Self>, app: &AppHandle, deps: &Deps<'_>) -> bool {
        self.kick(
            app,
            deps,
            SendOpts {
                text: SCREEN_ONLY_PROMPT,
                with_screen: true,
                screen_required: true,
                ..SendOpts::default()
            },
        )
    }

    /// Regenerate the last answer: re-runs the active ask session's last
    /// user turn — `send_chain`'s `regenerate` path skips persisting a
    /// second user row and drops the rejected reply's row, so a reload
    /// never replays it. A retry with no prior user turn is a no-op.
    pub fn retry(self: &Arc<Self>, app: &AppHandle, deps: &Deps<'_>) -> bool {
        let last = deps
            .db
            .session_active_id("ask")
            .ok()
            .flatten()
            .and_then(|sid| deps.db.messages_for(sid).ok())
            .and_then(|rows| {
                rows.iter()
                    .rev()
                    .find(|r| r.role == "user")
                    .map(|r| (r.content.clone(), r.preset.clone()))
            });
        let Some((text, preset)) = last else {
            log::warn!("ask::retry: no user turn to regenerate");
            return false;
        };
        self.kick(
            app,
            deps,
            SendOpts {
                text: &text,
                preset,
                regenerate: true,
                ..SendOpts::default()
            },
        )
    }

    /// `ask_close`: collapse the card — and ONLY collapse. Runs are
    /// never killed by leaving a view (`stop` is the composer's sole
    /// cancel path): a detached run keeps streaming into its own
    /// session, and a remounting chat re-attaches via `ask_runs`.
    pub fn close(&self, app: &AppHandle, pool: &Mutex<WindowPool>) {
        pool.lock().set_chat_open(app, false);
    }

    /// `leave_main` teardown — a gate exit is not a view leave: cancel
    /// EVERY run so no stream keeps writing after the app leaves the
    /// surface that owns it. Each live run's tagged `idle` still goes
    /// out (mirrors settle; the webview's dead-packet filters retire
    /// its strays) — a finished run's rows already persisted where it
    /// belongs.
    pub fn retire_all(&self, app: &AppHandle) {
        let retired: Vec<(Option<i64>, Run)> = {
            let mut runs = self.runs.lock();
            runs.drain().collect()
        };
        // The pre-flight error slot belongs to the left surface too —
        // a re-entered bar must not resync a dead send's failure.
        *self.orphan_error.lock() = None;
        for (sid, run) in retired {
            run.cancel.cancel();
            if run.state != AskState::Idle {
                let _ = app.emit_to(
                    BAR_LABEL,
                    EV_STATE,
                    json!({
                        "state": "idle",
                        "run": run.gen,
                        "session_id": sid,
                    }),
                );
            }
        }
    }

    /// `ask_stop`: cancel ONE session's in-flight run without collapsing
    /// the card. Removing the run's table entry makes every trailing
    /// emit drop at `fold_emit`'s check (a failover `loading` re-emit,
    /// the cancelled chain's own terminal `idle` — an emit already in
    /// the IPC pipe is the webview's to drop); cancelling the token
    /// then ends the chain itself — the persisted user row stays, the
    /// partial reply does not. Since the chain's own terminal emit is
    /// suppressed, `stop` emits `ask:state{idle}` itself when the run
    /// was actually live — tagged with the killed `run`/`session_id`
    /// so exactly that run's in-flight packets retire.
    pub fn stop(&self, app: &AppHandle, session_id: i64) {
        let Some(run) = self.retire(Some(session_id)) else {
            return;
        };
        if run.state != AskState::Idle {
            let _ = app.emit_to(
                BAR_LABEL,
                EV_STATE,
                json!({
                    "state": "idle",
                    "run": run.gen,
                    "session_id": session_id,
                }),
            );
        }
    }

    /// Shared pre-flight + spawn behind `send`/`send_screen_only`/`retry`.
    /// The [`SendOpts`] fields pick the run's mode — see the struct.
    fn kick(self: &Arc<Self>, app: &AppHandle, deps: &Deps<'_>, opts: SendOpts<'_>) -> bool {
        let SendOpts {
            text,
            with_screen,
            screen_required,
            regenerate,
            listen_id,
            preset,
            preset_lang,
            attachments,
        } = opts;
        // Fresh run id up-front — every emit path (the spawn's fold, a
        // pre-flight `kick_error`, `pre_spawn_error`) tags it.
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // A send arriving with the card closed starts a NEW conversation —
        // the pill input isn't a follow-up field. Read before the flag flips.
        // A regenerate never mints a session — there'd be nothing in it
        // to re-ask.
        let fresh_session = !regenerate && !deps.pool.lock().is_chat_open();
        // The card opens only where there's something to show — a
        // pre-flight error below, or the run once `claim` accepts it —
        // so a session-busy refusal stays a silent no-op for the pill.
        let open_card = || deps.pool.lock().set_chat_open(app, true);
        // Attachment validation precedes session resolution: a rejected
        // send must not mint a ghost session row. The chain decodes
        // again for persistence — this pass is the early no so the
        // sessionless error stays off `runs` (same shape `send_chain`'s
        // guard emits, minus the resolved session tag).
        if attachments.len() > MAX_ATTACHMENTS {
            open_card();
            self.kick_error(
                app,
                gen,
                format!("At most {MAX_ATTACHMENTS} images can be attached per send"),
            );
            return true;
        }
        for input in &attachments {
            if let Err(message) = attachments::decode_attachment(input) {
                open_card();
                self.kick_error(app, gen, message);
                return true;
            }
        }

        // The failover chain: `providers.order` minus disabled/unusable.
        // An empty chain is the "no usable provider" error — nothing to
        // fall back TO, so the card links straight to settings. The
        // screen reader (`[vision]`), the armed preset's resolution, and
        // its `{lang}` substitution all read under the same lock.
        let (candidates, vision, language, instruction, preset_hit, memory) = {
            let cfg = deps.config.lock();
            let ks = deps.keystore.lock();
            let resolved = preset.as_deref().and_then(|id| {
                let p = crate::presets::resolve(id, &cfg.prompts.custom);
                if p.is_none() {
                    log::warn!("ask: preset {id} unresolved — sending without");
                }
                p
            });
            // `{lang}` — the param badge's edited value when the send
            // carried one, else the configured main language.
            let lang_arg = preset_lang.clone().unwrap_or_else(|| {
                crate::prompts::language_name(&cfg.app.main_language).to_string()
            });
            // An expanding preset (`{input}` in its text) contributes no
            // instruction — the webview already folded it into `text`;
            // the id persists below purely as provenance.
            let instruction = resolved
                .as_ref()
                .filter(|p| !crate::presets::is_template(p))
                .map(|p| p.text.replace("{lang}", &lang_arg));
            // The memory hook resolves under the same snapshot — its
            // `[memory]` pick is independent of the chain above (order
            // and disabled switches don't apply), and the `changed`
            // callback only broadcasts `memory:changed` — fire-and-forget,
            // never a lock held into the emit. The snapshot's `enabled`
            // check is a pre-flight only: the scheduled extraction
            // re-reads consent through `consent` (a live lock on the
            // shared config) after acquiring the service gate, so a
            // disable landing mid-stream or mid-queue still stops it.
            let changed: Arc<dyn Fn() + Send + Sync> = {
                let app = app.clone();
                Arc::new(move || {
                    let _ = app.emit(crate::EV_MEMORY_CHANGED, json!({}));
                })
            };
            let consent: Arc<dyn Fn() -> bool + Send + Sync> = {
                let config = Arc::clone(&deps.config);
                Arc::new(move || config.lock().memory.enabled)
            };
            (
                crate::provider_candidates(&cfg, &ks),
                crate::vision_candidate(&cfg, &ks),
                cfg.app.main_language.clone(),
                instruction,
                resolved.is_some(),
                crate::memory::prepare_hook(
                    &cfg,
                    &ks,
                    Arc::clone(&deps.db),
                    Arc::clone(&deps.memory),
                    changed,
                    consent,
                ),
            )
        };
        if candidates.is_empty() {
            open_card();
            self.pre_spawn_error(
                app,
                gen,
                text,
                regenerate,
                json!({
                    "message": "No AI provider is configured — add a key in Settings → Providers",
                    "needs_setup": true,
                }),
            );
            return true;
        }
        // Session resolution moved UP from `send_chain`: the run
        // registers under its session BEFORE spawning, so the
        // per-session busy gate and the emit fold key on it from t=0.
        // (The pipeline keeps an identical resolution fallback for the
        // direct `send_chain` tests.)
        let session_id =
            history::resolve_session(&deps.db, fresh_session, regenerate, listen_id);
        // The per-session busy gate: a live run on THIS session refuses
        // the send — any other session's run streams on.
        let Some(cancel) = self.claim(session_id, text, gen) else {
            return false;
        };
        open_card();
        let svc = Arc::clone(self);
        let app = app.clone();
        let db = Arc::clone(&deps.db);
        // The title sidecar's emit is dedicated — the spawned task's
        // gen-guarded `emit` would drop it once this run's generation
        // ends (stop/superseded), but a landed title write should
        // always refresh the history list.
        let titled: Arc<dyn Fn(i64) + Send + Sync> = {
            let app = app.clone();
            Arc::new(move |sid| {
                let _ = app.emit_to(BAR_LABEL, EV_SESSIONS_CHANGED, json!({"id": sid}));
            })
        };
        let text = text.to_string();
        // The `resolve_screen` inputs — computed here so the spawned
        // task owns plain values/`Arc`s (`deps` is a borrow that dies
        // with this call). `explicit` marks asks that *demand* the
        // screen NOW (Cmd+Enter, screen-only, intent keyword) — the
        // captured frame attaches as a user image; `needs_screen`
        // additionally counts ambient recording (a plain ask still
        // gets the cached context, it just doesn't force a capture).
        let screen_explicit =
            with_screen || screen_required || screen_read::looks_like_screen_intent(&text);
        let needs_screen = screen_explicit || deps.capture_running;
        let read_interval_secs = deps.config.lock().recording.read_interval_secs;
        let reader = Arc::clone(&deps.reader);
        let ring = Arc::clone(&deps.ring);
        let capture_running = deps.capture_running;
        // `None` when the id didn't resolve — a stale/deleted preset
        // must not persist `preset = "u:dead"` on the user row or claim
        // provenance it never had in the `loading` emits (the send
        // itself still proceeds per the warn above).
        let preset_id = preset.filter(|_| preset_hit);
        tauri::async_runtime::spawn(async move {
            // Outgoing events fold into this run's table entry — but
            // only while it IS the table's run: a stopped or superseded
            // task's trailing emits drop at `fold_emit` instead of
            // clobbering a newer send. The `session_id` tag rides in
            // from `send_chain`'s emit wrapper (resolved upstream);
            // `run` tags here so one injection covers every packet.
            let emit = move |name: &str, mut payload: serde_json::Value| {
                payload["run"] = gen.into();
                if !svc.fold_emit(session_id, gen, name, &payload) {
                    return;
                }
                let _ = app.emit_to(BAR_LABEL, name, payload);
            };
            let screen_input = ScreenInput {
                reader: &reader,
                ring: &ring,
                capture_running,
                needs_screen,
                explicit: screen_explicit,
                read_interval_secs,
                screen_permission: crate::permissions::screen_status,
                shot: crate::capture::shot_fullscreen,
            };
            let _ = send_chain(
                candidates,
                vision,
                db.as_ref(),
                &emit,
                &screen_input,
                &cancel,
                ChainOpts {
                    text: &text,
                    session_id,
                    fresh_session,
                    regenerate,
                    listen_id,
                    language: &language,
                    instruction: instruction.as_deref(),
                    preset_id: preset_id.as_deref(),
                    attachments,
                    attachments_root: None,
                    memory,
                    title: Some(TitleSidecar::new(Arc::clone(&db), titled)),
                },
            )
            .await;
        });
        true
    }

    /// A kick-time rejection past the card expand but before any
    /// session work — the same `ask:error` → `ask:state{idle}` shape
    /// `send_chain`'s attachment guard emits, except sessionless (no
    /// run exists yet): the webview's session gate passes it onto the
    /// conversation the user is looking at, and `fold_orphan` keeps
    /// it for `ask_runs` resync.
    fn kick_error(&self, app: &AppHandle, gen: u64, message: String) {
        let emit = |name: &str, mut payload: serde_json::Value| {
            payload["run"] = gen.into();
            self.fold_orphan(name, &payload);
            let _ = app.emit_to(BAR_LABEL, name, payload);
        };
        emit(EV_ERROR, json!({ "message": message }));
        emit(EV_STATE, json!({"state": "idle"}));
    }

    /// Early-exit error (empty provider chain): emit the same
    /// `ask:state{loading}` → `ask:error` → `ask:state{idle}` sequence
    /// `send_chain`'s failure path uses (the `loading` carries the
    /// question so the card's run-reset/header work here too). The
    /// whole sequence is sessionless — no session exists to bind yet —
    /// and the error folds through `fold_orphan` before emitting: it
    /// lands ~0ms after the card starts opening, so the `ask_runs`
    /// resync is the only reliable delivery to the still-mounting
    /// webview.
    fn pre_spawn_error(
        &self,
        app: &AppHandle,
        gen: u64,
        text: &str,
        regenerate: bool,
        payload: serde_json::Value,
    ) {
        // `run`-tagged like the spawned task's emits — `kick` bumped
        // the generation before delegating here. The
        // `attempt`/`regenerate` boundary fields match `make_loading`'s
        // run-start shape so the card folds this the same way.
        let loading = json!({
            "state": "loading",
            "question": text,
            "attempt": 0,
            "regenerate": regenerate,
            "run": gen,
        });
        self.fold_orphan(EV_STATE, &loading);
        let _ = app.emit_to(BAR_LABEL, EV_STATE, loading);
        let mut payload = payload;
        payload["run"] = gen.into();
        self.fold_orphan(EV_ERROR, &payload);
        let _ = app.emit_to(BAR_LABEL, EV_ERROR, payload);
        let idle = json!({"state": "idle", "run": gen});
        self.fold_orphan(EV_STATE, &idle);
        let _ = app.emit_to(BAR_LABEL, EV_STATE, idle);
    }
}

impl Default for AskService {
    fn default() -> Self {
        Self::new()
    }
}

