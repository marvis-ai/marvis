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
//! a sidecar call AFTER `ask:state{idle}` (history shows the
//! first-question fallback until it lands, then `sessions:changed`
//! refreshes it). A provider `MultimodalUnsupported` rejection retries
//! once text-only (per attempt): a screen frame is simply dropped,
//! while the user's own attachments are first described by the
//! `[vision]` reader into an `<attached_images>` block — never
//! silently discarded. [`AskService::close`] aborts any in-flight
//! stream via a [`CancellationToken`].
//!
//! Event protocol (emitted to the `bar` window via `app.emit_to`):
//! - `ask:state` `{"state": "loading"|"streaming"|"idle"}` — `streaming`
//!   fires on each attempt's FIRST token; every `loading` carries
//!   `"question"` so the card resets its buffer + header per run AND
//!   per failover retry (pre-flight errors emit `loading` → `error` →
//!   `idle` too); `send_chain`'s `loading`s also carry `"preset"` —
//!   the armed `instruct` preset id, `null` for a plain send or an id
//!   that didn't resolve.
//! - `ask:chunk` `{"text": token}` per token.
//! - `ask:done` `{"full": full_reply, "provider": id, "model": id,
//!   "usage": {"input": n?, "output": n?} | null}` on success — the pair
//!   that actually answered plus the run's reported token spend.
//! - `ask:error` `{"message": ..., "needs_setup": bool?}` on failure —
//!   `needs_setup` when the chain was empty (no usable provider at all).
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
mod history;
mod pipeline;
mod screen;
mod stream;
mod title;

use self::{pipeline::*, screen::*};

#[cfg(test)]
mod tests;

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
use crate::storage::{Db, MessageAttachment, MessageMeta, NewAttachment, Transcript};
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
/// compiles before `AppState` exists. `db`/`ring`/`reader` are owned
/// `Arc`s (the spawned stream task outlives the call); the rest are
/// locked only during synchronous pre-flight, so plain `&Mutex` borrows
/// suffice.
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
    pub keystore: &'a Mutex<Keystore>,
    pub config: &'a Mutex<Config>,
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

/// One ask at a time; the Task-14 `AppState` share.
pub struct AskService {
    state: Mutex<AskState>,
    /// Cancel handle for the in-flight stream; replaced per run/close.
    cancel: Mutex<CancellationToken>,
    /// Last completed assistant reply (scroll/status consumers).
    current_response: Mutex<String>,
    /// Last submitted user text.
    current_question: Mutex<String>,
    /// The run's user-turn attachments (persisted metadata folded out
    /// of `ask:state{loading}`) — `ask_current` resyncs them so a
    /// re-expanded card renders the message's images.
    current_attachments: Mutex<Vec<MessageAttachment>>,
    /// The last `ask:error` payload — kept so `ask_current` can resync
    /// an error that fired before the webview was listening (pre-flight
    /// errors land ~0ms after the card starts opening).
    last_error: Mutex<Option<serde_json::Value>>,
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
            current_attachments: Mutex::new(Vec::new()),
            last_error: Mutex::new(None),
            generation: AtomicU64::new(0),
        }
    }

    pub fn state(&self) -> AskState {
        *self.state.lock()
    }

    pub fn current_response(&self) -> String {
        self.current_response.lock().clone()
    }

    pub fn current_question(&self) -> String {
        self.current_question.lock().clone()
    }

    /// The `ask_current` resync payload: live `state` + the run's
    /// question/reply tail + the last `ask:error` (`null` normally —
    /// kept so a pre-flight error that beat the webview's `listen()`
    /// still renders on mount).
    pub fn current_payload(&self) -> serde_json::Value {
        json!({
            "state": self.state().as_str(),
            "question": self.current_question(),
            "response": self.current_response(),
            "error": self.last_error.lock().clone(),
            "attachments": self.current_attachments.lock().clone(),
        })
    }

    /// Ask a free-text question. `with_screen` is the explicit attach
    /// flag (`Cmd+Enter`/`withScreen` invoke arg) — it forces a screen
    /// read regardless of the text's intent heuristic. `listen_id`
    /// binds the send to a listen doc: the question lands in that doc's
    /// own ask session (reopened or minted — one chat per doc) and its
    /// summary+transcript becomes the meeting context. `attachments`
    /// are the composer's normalized JPEGs — decoded and persisted
    /// before the first provider call. A send while
    /// `Loading`/`Streaming` is ignored (warn-logged) rather than
    /// cancel-then-send — the in-flight stream keeps running.
    ///
    /// Returns after synchronous pre-flight; the stream itself runs on a
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
    ) {
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
        );
    }

    /// The camera button's screen-only ask: fixed prompt, screen
    /// REQUIRED — a failed one-shot errors instead of degrading to
    /// text-only.
    pub fn send_screen_only(self: &Arc<Self>, app: &AppHandle, deps: &Deps<'_>) {
        self.kick(
            app,
            deps,
            SendOpts {
                text: SCREEN_ONLY_PROMPT,
                with_screen: true,
                screen_required: true,
                ..SendOpts::default()
            },
        );
    }

    /// Regenerate the last answer: re-runs the active ask session's last
    /// user turn — `send_chain`'s `regenerate` path skips persisting a
    /// second user row and drops the rejected reply's row, so a reload
    /// never replays it. A retry with no prior user turn is a no-op.
    pub fn retry(self: &Arc<Self>, app: &AppHandle, deps: &Deps<'_>) {
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
            return;
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
        );
    }

    /// `ask_close`: cancel the in-flight stream (its `select!` arm emits
    /// the final `ask:state{idle}`), mint a fresh token for the next run,
    /// reset state, collapse the card. Emits nothing itself.
    ///
    /// Order matters: replace the token BEFORE flipping state to Idle —
    /// a `send` gated on `Idle` must never clone the cancelled token.
    pub fn close(&self, app: &AppHandle, pool: &Mutex<WindowPool>) {
        self.cancel.lock().cancel();
        *self.cancel.lock() = CancellationToken::new();
        *self.state.lock() = AskState::Idle;
        *self.last_error.lock() = None;
        *self.current_attachments.lock() = Vec::new();
        pool.lock().set_chat_open(app, false);
    }

    /// Shared pre-flight + spawn behind `send`/`send_screen_only`/`retry`.
    /// The [`SendOpts`] fields pick the run's mode — see the struct.
    fn kick(self: &Arc<Self>, app: &AppHandle, deps: &Deps<'_>, opts: SendOpts<'_>) {
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
        // Run boundary: the `ask_current` resync tail must not leak the
        // previous run's reply, attachments, or error into this one.
        *self.current_response.lock() = String::new();
        *self.current_attachments.lock() = Vec::new();
        *self.last_error.lock() = None;
        // A send arriving with the card closed starts a NEW conversation —
        // the pill input isn't a follow-up field. Read before the flag flips.
        // A regenerate never mints a session — there'd be nothing in it
        // to re-ask.
        let fresh_session = !regenerate && !deps.pool.lock().is_chat_open();
        // Expand first so any pre-flight error still renders in the card.
        deps.pool.lock().set_chat_open(app, true);

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
            // never a lock held into the emit.
            let changed: Arc<dyn Fn() + Send + Sync> = {
                let app = app.clone();
                Arc::new(move || {
                    let _ = app.emit(crate::memory::EV_MEMORY_CHANGED, json!({}));
                })
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
                ),
            )
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
        let cancel = self.cancel.lock().clone();
        let svc = Arc::clone(self);
        let app = app.clone();
        let db = Arc::clone(&deps.db);
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
                    fresh_session,
                    regenerate,
                    listen_id,
                    language: &language,
                    instruction: instruction.as_deref(),
                    preset_id: preset_id.as_deref(),
                    attachments,
                    attachments_root: None,
                    memory,
                },
            )
            .await;
        });
    }

    /// Early-exit error (empty provider chain):
    /// the busy check already flipped state to Loading — emit the same
    /// `ask:state{loading}` → `ask:error` → `ask:state{idle}` sequence
    /// `send_with`'s failure path uses (the `loading` carries the
    /// question so the card's run-reset/header work here too), then
    /// reset Rust-side state to match. The error folds through
    /// [`observe`] before emitting — it lands ~0ms after the card
    /// starts opening, so the `ask_current` resync is the only reliable
    /// delivery to the still-mounting webview.
    fn pre_spawn_error(&self, app: &AppHandle, text: &str, payload: serde_json::Value) {
        let _ = app.emit_to(
            BAR_LABEL,
            EV_STATE,
            json!({"state": "loading", "question": text}),
        );
        self.observe(EV_ERROR, &payload);
        let _ = app.emit_to(BAR_LABEL, EV_ERROR, payload);
        *self.state.lock() = AskState::Idle;
        let _ = app.emit_to(BAR_LABEL, EV_STATE, json!({"state": "idle"}));
    }

    /// Fold one outgoing event into Rust-side state: `ask:state` is
    /// folded by the emit closure under its generation-guarded lock (the
    /// busy-check source of truth); `ask:done` records the reply here;
    /// `ask:error` is kept for `ask_current` resyncs until a `loading`
    /// boundary (a new run or a failover retry) supersedes it — the
    /// trailing `idle` must NOT clear it, or the cold-open pre-flight
    /// error would be lost before the webview starts listening.
    fn observe(&self, name: &str, payload: &serde_json::Value) {
        if name == EV_DONE {
            if let Some(full) = payload["full"].as_str() {
                *self.current_response.lock() = full.to_string();
            }
        } else if name == EV_ERROR {
            *self.last_error.lock() = Some(payload.clone());
        } else if name == EV_STATE && payload["state"].as_str() == Some("loading") {
            *self.last_error.lock() = None;
            if let Some(atts) = payload.get("attachments") {
                if let Ok(atts) =
                    serde_json::from_value::<Vec<MessageAttachment>>(atts.clone())
                {
                    *self.current_attachments.lock() = atts;
                }
            }
        }
    }
}

impl Default for AskService {
    fn default() -> Self {
        Self::new()
    }
}

