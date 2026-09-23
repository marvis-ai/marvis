//! `lib.rs` — `AppState`, the command surface, and the startup sequence.
//!
//! Wires every module together: `AppState` is managed via `app.manage(...)`
//! in [`run`]'s `setup`, then reached from commands (`State<'_, AppState>`
//! / `AppHandle`), the hotkey dispatch closure, and the deep-link dispatch
//! closure (both re-resolve state per event so they survive gate
//! transitions).
//!
//! ## App gate
//!
//! [`Gate`] = `NeedsPermission` → `Main`. `Main` needs BOTH halves of
//! first-run readiness: `app.onboarding_done` AND screen-recording
//! permission — while the wizard runs the gate never reaches `Main`, so
//! the card can't open and capture + the full hotkey set simply don't
//! exist yet. [`transition_gate`] recomputes the gate after every mutation that
//! can change it (`config_set` on `app.onboarding_done`,
//! `permissions_request_screen`, and once at startup) and ALWAYS emits
//! `app:state`.
//!
//! The bar's visibility is separate from the gate: it floats only when
//! `onboarding_done` AND the wizard isn't on screen — see
//! [`AppState::sync_bar_visibility`].
//!
//! ## Event payloads (webview contract)
//!
//! - `app:state` `{"gate": "needs_permission"|"main"}` — broadcast to
//!   every window on each `transition_gate` call.
//! - `keystore:changed` — the `keystore_status` payload
//!   `{"keys": [[provider, "…last4"], …]}`, broadcast after every key
//!   mutation (`set_key`/`remove_key`). Keys live in plaintext
//!   `keys.json` — there is no lock state.
//! - `alert:show` `{"message": String}` — to the `alert` window only;
//!   the toast that replaced the bar's inline error row
//!   (`alert_current` re-reads it, `alert_dismiss` clears it).

mod ask;
pub mod audio;
mod capture;
mod config;
mod deeplink;
mod hotkey;
mod keystore;
mod llm;
mod paths;
mod permissions;
mod prompts;
mod storage;
pub mod stt;
mod tray;
mod windows;

use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_liquid_glass::LiquidGlassExt;

use ask::AskService;
use capture::{FrameSource, MacosCapture, RingBuffer};
use config::Config;
use hotkey::RegisteredHotkeys;
use keystore::Keystore;
use llm::{make_provider, ProviderKind};
use storage::{AiMessage, Db, Session};
use windows::WindowPool;

/// Frame ring caps from the spec: 120 frames / 64 MB (~60 s horizon).
const RING_MAX_FRAMES: usize = 120;
const RING_MAX_BYTES: usize = 64 * 1024 * 1024;

/// Local Ollama daemon's model list (same host the adapter streams from).
const OLLAMA_TAGS_URL: &str = "http://localhost:11434/api/tags";

/// Which UI state the bar may show. The card, the full hotkey set, and
/// capture exist only in `Main`. (Onboarding isn't a gate state —
/// it's a visibility overlay on top: `onboarding_done` is one of `Main`'s
/// two preconditions, so the wizard can never share the screen with the
/// bar's live machinery.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Onboarding incomplete OR no screen-recording permission — the
    /// permission card covers both (during onboarding the bar is hidden
    /// anyway, so the label never misleads).
    NeedsPermission,
    /// Fully live: card, full hotkeys, capture.
    Main,
}

impl Gate {
    /// The `app:state` payload value.
    fn name(self) -> &'static str {
        match self {
            Gate::NeedsPermission => "needs_permission",
            Gate::Main => "main",
        }
    }
}

/// Everything commands, hotkey dispatch, and deep links touch. Field
/// types follow the consumers: `Deps` borrows `&Mutex<…>` (so `keystore`,
/// `config`, `pool` are plain `Mutex`es) while `db`/`ring`/`ask` are
/// `Arc`s shared with spawned tasks and the capture callback.
pub struct AppState {
    keystore: Mutex<Keystore>,
    config: Mutex<Config>,
    db: Arc<Db>,
    ring: Arc<Mutex<RingBuffer>>,
    capture: Mutex<Option<MacosCapture>>,
    ask: Arc<AskService>,
    pool: Mutex<WindowPool>,
    /// Currently live set — delta-swapped in place by [`swap_hotkeys`]
    /// (shared pairs are never re-registered; macOS refuses duplicates).
    hotkeys: Mutex<Option<RegisteredHotkeys>>,
    gate: Mutex<Gate>,
    /// Serializes `transition_gate` — the gate swap, its side-effects
    /// (enter/leave Main), and the `app:state` emit must be one critical
    /// section or two overlapping transitions can interleave (stale
    /// emit landing last, or hotkeys/capture from a dead gate state).
    gate_transition: Mutex<()>,
    /// Payload of the alert toast currently on screen (`None` when
    /// dismissed). Kept server-side so the toast can re-read it on mount
    /// via `alert_current` — an `alert:show` emit that races the
    /// webview's listener would otherwise be lost.
    alert: Mutex<Option<serde_json::Value>>,
}

impl AppState {
    /// The ask pipeline's borrow bundle: `db` clones the `Arc` (the
    /// spawned stream outlives the call), the rest are short-lived
    /// `&Mutex` borrows used only in pre-flight.
    fn deps(&self) -> ask::Deps<'_> {
        ask::Deps {
            db: Arc::clone(&self.db),
            ring: &self.ring,
            keystore: &self.keystore,
            config: &self.config,
            pool: &self.pool,
        }
    }

    /// `app.onboarding_done` — the wizard-completion flag that both the
    /// gate (`app_gate`) and the bar's visibility rule read.
    pub(crate) fn onboarding_done(&self) -> bool {
        self.config.lock().app.onboarding_done
    }

    /// The gate read `WindowPool::set_chat_open` needs — opening the
    /// card is `Main`-only.
    pub(crate) fn gate_is_main(&self) -> bool {
        *self.gate.lock() == Gate::Main
    }

    /// The single source of truth for bar visibility. Re-reads
    /// `onboarding_done` and asks the pool to reconcile — the bar floats
    /// only when onboarding is done AND the wizard isn't on screen
    /// (a re-run hides it again until the prefs window closes). Called by
    /// every prefs show/hide path and by `config_set` writes to
    /// `app.onboarding_done`.
    pub(crate) fn sync_bar_visibility(&self) {
        let done = self.onboarding_done();
        self.pool.lock().set_bar_shown(done);
    }

    /// Headless constructor for tests: filesystem-bound fields bind under
    /// `root` so tests never touch `~/.marvis`; windows/capture/hotkeys
    /// need a runtime, so the pool is empty and those slots stay `None`.
    /// The keystore is a plain plaintext file — no crypto in tests.
    #[cfg(test)]
    fn for_test(root: &std::path::Path) -> Self {
        Self {
            keystore: Mutex::new(Keystore::at(root.join("keys.json"))),
            config: Mutex::new(Config::load_from(root.join("config.toml")).unwrap_or_default()),
            db: Arc::new(Db::at(root.join("marvis.db")).expect("test db")),
            ring: Arc::new(Mutex::new(RingBuffer::new(RING_MAX_FRAMES, RING_MAX_BYTES))),
            capture: Mutex::new(None),
            ask: Arc::new(AskService::new()),
            pool: Mutex::new(WindowPool::new_empty()),
            hotkeys: Mutex::new(None),
            gate: Mutex::new(Gate::NeedsPermission),
            gate_transition: Mutex::new(()),
            alert: Mutex::new(None),
        }
    }
}

// ---------------------------------------------------------------------------
// App gate
// ---------------------------------------------------------------------------

/// `Main` needs both first-run halves: `app.onboarding_done` AND
/// `permissions::screen_status()`. While the wizard runs the gate stays
/// `NeedsPermission`, so `enter_main` (capture, full hotkeys) can't
/// fire underneath it — finishing onboarding re-evaluates through
/// `config_set` → `transition_gate`.
fn app_gate(state: &AppState) -> Gate {
    let ready = state.config.lock().app.onboarding_done && permissions::screen_status();
    if ready {
        Gate::Main
    } else {
        Gate::NeedsPermission
    }
}

/// Recompute the gate, run the transition's side-effects, then ALWAYS
/// emit `app:state` `{"gate": ...}` — called after every mutation that can
/// change the gate and once at startup.
fn transition_gate(app: &AppHandle) {
    let state = app.state::<AppState>();
    // Serialize the whole transition: the gate read+swap, the Main
    // side-effects, and the `app:state` emit are one critical section.
    // Nothing else locks `gate_transition`, so this cannot deadlock.
    let _transition = state.gate_transition.lock();
    let new_gate = app_gate(&state);
    let old_gate = std::mem::replace(&mut *state.gate.lock(), new_gate);
    if old_gate != new_gate {
        if new_gate == Gate::Main {
            enter_main(app);
        } else if old_gate == Gate::Main {
            leave_main(app);
        }
    }
    let _ = app.emit("app:state", json!({ "gate": new_gate.name() }));
}

/// `Main` entry: full hotkey set + capture. Every step warns and
/// continues — a failed piece must never wedge the gate.
fn enter_main(app: &AppHandle) {
    let state = app.state::<AppState>();
    swap_hotkeys(app, true);
    let mut slot = state.capture.lock();
    if slot.is_none() {
        match MacosCapture::new() {
            Ok(capture) => {
                let ring = Arc::clone(&state.ring);
                capture.start(Box::new(move |frame| ring.lock().push(frame)));
                // Store only on success — a silently-failed start must
                // not block a later retry (`is_running` is false when
                // SCStream rejected the handler or `start_capture` failed).
                if capture.is_running() {
                    *slot = Some(capture);
                } else {
                    log::warn!("gate: screen capture failed to start");
                }
            }
            Err(e) => log::warn!("gate: screen capture failed to init: {e}"),
        }
    }
}

/// `Main` exit (onboarding reset / permission revoked): cancel the
/// in-flight ask and collapse the card, stop + drop capture, hide the
/// alert toast, downgrade hotkeys to the gated limited set.
fn leave_main(app: &AppHandle) {
    let state = app.state::<AppState>();
    // Cancel any in-flight ask — an unbounded stream left running would
    // hold `AskState::Streaming` past a leave/re-enter and wedge every
    // future send on the busy-check until it resolves on its own.
    state.ask.close(app, &state.pool);
    // Take the capture out and release the lock BEFORE `stop()` — it
    // joins the capture worker, which must not hold `state.capture`
    // while `capture_status` waits on it.
    let capture = state.capture.lock().take();
    if let Some(capture) = capture {
        capture.stop();
    }
    state.pool.lock().hide_alert();
    swap_hotkeys(app, false);
}

/// Delta-swap to the full (`all = true`) or limited set via
/// [`hotkey::swap_hotkey_set`]: pairs shared with the live set stay
/// registered untouched (macOS Carbon refuses duplicate registration of
/// a combo, so a register-everything-then-unregister swap can never
/// succeed — the limited set is a subset of the full one). On failure
/// the swap has already rolled back; the returned `restored` set is what
/// the OS still has bound, so it's stored either way.
fn swap_hotkeys(app: &AppHandle, all: bool) {
    let state = app.state::<AppState>();
    let binds = state.config.lock().hotkeys.clone();
    let dispatch = hotkey_dispatch(app);
    // Hold the guard across the swap: a concurrent swap seeing `None`
    // would compute a full `add` set against still-live OS bindings and
    // wedge every future swap. `state.hotkeys` is locked nowhere else,
    // so holding it here cannot deadlock.
    let mut slot = state.hotkeys.lock();
    let prev = slot.take().unwrap_or_default();
    match hotkey::swap_hotkey_set(app, &binds, !all, dispatch, prev) {
        Ok(set) => {
            *slot = Some(set);
        }
        Err(e) => {
            log::warn!("hotkey swap failed (all={all}): {e}");
            *slot = Some(e.restored);
        }
    }
}

// ---------------------------------------------------------------------------
// Dispatch closures
// ---------------------------------------------------------------------------

/// Map [`hotkey::Action`]s onto pool/ask calls. Owns an `AppHandle` and
/// re-resolves `AppState` per press, so the same closure works for both
/// registration scopes and across gate transitions.
fn hotkey_dispatch(app: &AppHandle) -> impl Fn(hotkey::Action) + Send + Sync + 'static {
    let app = app.clone();
    move |action| {
        let state = app.state::<AppState>();
        match action {
            hotkey::Action::ToggleVisibility => state.pool.lock().toggle_chat(&app),
            // `next_step` and `screen_only` fire the screen-only ask.
            // The gate guard mirrors `ask_send_screen_only`'s — during
            // onboarding (gate != Main) the card can't open.
            hotkey::Action::NextStep | hotkey::Action::ScreenOnly => {
                if *state.gate.lock() == Gate::Main {
                    state.ask.send_screen_only(&app, &state.deps());
                }
            }
            hotkey::Action::ShowSettings => show_settings(&app),
        }
    }
}

/// Map [`deeplink::Action`]s: `Ask` runs only while the gate is `Main`;
/// `Focus` surfaces the bar; `Ignore` never reaches the dispatch
/// (filtered inside `init`). While onboarding runs the bar is hidden, so
/// neither action may surface it.
fn deeplink_dispatch(app: &AppHandle) -> impl Fn(deeplink::Action) + Send + Sync + 'static {
    let app = app.clone();
    move |action| match action {
        deeplink::Action::Ask(text) => {
            let state = app.state::<AppState>();
            if *state.gate.lock() == Gate::Main {
                let bar = state.pool.lock().bar().cloned();
                if let Some(bar) = bar {
                    let _ = bar.set_focus();
                }
                state.ask.send(&app, &state.deps(), &text);
            } else {
                // Not ready yet — if the bar is visible (onboarding done,
                // permission pending) surface its gate card instead of
                // the link silently going nowhere.
                log::warn!("deeplink: ask gated off (gate = {:?})", *state.gate.lock());
                if state.onboarding_done() {
                    let bar = state.pool.lock().bar().cloned();
                    if let Some(bar) = bar {
                        let _ = bar.set_focus();
                    }
                }
            }
        }
        deeplink::Action::Focus => {
            let state = app.state::<AppState>();
            // During onboarding the bar is hidden — nothing to focus.
            if !state.onboarding_done() {
                return;
            }
            // Clone the handle out so the pool guard drops before `state`.
            let bar = state.pool.lock().bar().cloned();
            if let Some(bar) = bar {
                let _ = bar.set_focus();
            }
        }
        deeplink::Action::Ignore => {}
    }
}

/// Map tray menu ids onto pool/app calls — same re-resolve-per-event
/// shape as [`hotkey_dispatch`]. `Toggle` mirrors `Cmd+/`
/// (`window_toggle_all`), `Settings` mirrors `Cmd+,`; `Quit` is
/// `app.exit(0)`.
fn tray_menu_dispatch() -> impl Fn(&AppHandle, tauri::menu::MenuEvent) + Send + Sync + 'static {
    move |app, event| match event.id().as_ref() {
        tray::MENU_TOGGLE => app.state::<AppState>().pool.lock().toggle_chat(app),
        tray::MENU_SETTINGS => show_settings(app),
        tray::MENU_QUIT => app.exit(0),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Shared payload/validation helpers
// ---------------------------------------------------------------------------

/// `keystore_status` return value and `keystore:changed` payload:
/// `{"keys": [[provider, "…last4"], …]}`. Masked only — plaintext keys
/// never leave `Keystore`. (There is no lock state: `keys.json` is
/// plaintext-on-disk inside the 0700 `~/.marvis` root.)
fn keystore_status_payload(keystore: &Keystore) -> serde_json::Value {
    json!({ "keys": keystore.masked_status() })
}

/// Raise the alert toast with `message`.
///
/// The toast is a window of its own because the bar is a fixed-height
/// capsule (112⇄480 wide) — the old inline error row squeezed the
/// pill's content. It is purely informational and auto-dismisses.
fn show_alert(app: &AppHandle, message: &str) {
    let state = app.state::<AppState>();
    let payload = json!({ "message": message });
    *state.alert.lock() = Some(payload.clone());
    let _ = app.emit_to(windows::ALERT_LABEL, "alert:show", payload);
    state.pool.lock().show_alert();
}

/// Surface the prefs window in settings mode. Works at ANY gate —
/// `Cmd+,`/the tray item must respond even mid-onboarding.
fn show_settings(app: &AppHandle) {
    app.state::<AppState>()
        .pool
        .lock()
        .show_prefs(app, "settings");
}

/// Broadcast the (masked) keystore status after any mutation.
fn emit_keystore_changed(app: &AppHandle, keystore: &Keystore) {
    let _ = app.emit("keystore:changed", keystore_status_payload(keystore));
}

/// `(model, base_url)` args for building a provider. `model` is the
/// provider's remembered pick (`providers.models.<id>`), else its first
/// static model (`""` for Ollama — its `validate` hits `/api/tags` and
/// never names a model — and for a compatible endpoint, whose validation
/// hits `/models`). `base_url` is the configured `compat.base_url`
/// (only `Compatible` reads it; `None` when unset).
fn provider_args(state: &AppState, kind: ProviderKind) -> (String, Option<String>) {
    let cfg = state.config.lock();
    let model = cfg.providers.model_for(kind.as_str());
    let base_url = Some(cfg.compat.base_url.clone()).filter(|u| !u.is_empty());
    (model, base_url)
}

/// One usable entry in the failover chain — the ask pipeline tries
/// candidates front-to-back until one answers.
pub(crate) struct ProviderCandidate {
    /// Provider id (`ProviderKind::as_str`) — reported on `ask:done`.
    pub id: String,
    /// The model the adapter was built with — reported on `ask:done`.
    pub model: String,
    pub provider: Box<dyn llm::Provider>,
}

/// The failover chain in priority order: `providers.order`, minus the
/// disabled, minus the unusable — no key where one is required, no
/// `compat.base_url` for `compatible`, no resolvable model. The first
/// entry is what `model_get_selected` reports; the ask pipeline walks the
/// rest on failure.
pub(crate) fn provider_candidates(cfg: &Config, ks: &Keystore) -> Vec<ProviderCandidate> {
    cfg.providers
        .order
        .iter()
        .filter(|id| cfg.providers.is_enabled(id))
        .filter_map(|id| {
            let kind = ProviderKind::from_str(id)?;
            let api_key = ks.key(id);
            if api_key.is_none() && !kind.key_optional() {
                return None; // no key — can't answer
            }
            let base_url = Some(cfg.compat.base_url.clone()).filter(|u| !u.is_empty());
            if kind == ProviderKind::Compatible && base_url.is_none() {
                return None; // endpoint never configured
            }
            let model = cfg.providers.model_for(id);
            if model.is_empty() {
                return None; // live-list provider with nothing selected
            }
            Some(ProviderCandidate {
                id: id.clone(),
                provider: make_provider(kind, api_key, model.clone(), base_url),
                model,
            })
        })
        .collect()
}

/// The configured screen reader (`[vision]`): `vision.provider` resolved
/// the same way as a chain candidate — key where one is required,
/// `compat.base_url` for a compatible endpoint, a resolvable model from
/// `vision.models.<id>` or the provider's vision default — but
/// INDEPENDENT of `providers.order`/`disabled` (a chat-disabled provider
/// may still read the screen). `None` = off or unusable; the ask then
/// attaches the frame to the answering provider as before.
pub(crate) fn vision_candidate(cfg: &Config, ks: &Keystore) -> Option<ProviderCandidate> {
    let kind = ProviderKind::from_str(&cfg.vision.provider).filter(|k| k.is_vision())?;
    let api_key = ks.key(kind.as_str());
    if api_key.is_none() && !kind.key_optional() {
        return None; // no key — can't read
    }
    let base_url = Some(cfg.compat.base_url.clone()).filter(|u| !u.is_empty());
    if kind == ProviderKind::Compatible && base_url.is_none() {
        return None; // endpoint never configured
    }
    let model = cfg.vision.model_for(kind.as_str());
    if model.is_empty() {
        return None; // compatible with no model id typed yet
    }
    Some(ProviderCandidate {
        id: kind.as_str().to_string(),
        provider: make_provider(kind, api_key, model.clone(), base_url),
        model,
    })
}

/// `GET /api/tags` → model names; any error (daemon down, bad body) maps
/// to an empty list — the dropdown just shows nothing.
async fn ollama_models() -> Vec<String> {
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(llm::CONNECT_TIMEOUT)
        .timeout(llm::VALIDATE_TIMEOUT)
        .build()
    else {
        return Vec::new();
    };
    let Ok(resp) = client.get(OLLAMA_TAGS_URL).send().await else {
        return Vec::new();
    };
    let Ok(body) = resp.json::<serde_json::Value>().await else {
        return Vec::new();
    };
    body["models"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter_map(|m| m["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// `window.bar_*` accepts a JSON number (set) or null (forget the pref).
fn window_pref_value(value: &serde_json::Value) -> Result<Option<f64>, String> {
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_f64()
            .map(Some)
            .ok_or_else(|| "window position must be a number or null".to_string())
    }
}

/// Write the bar's live rect into `window.bar_x/bar_y` — "remembered
/// position" with no settings row. Called from the bar's debounced
/// `Moved` hook (windows/mod.rs `BAR_MOVE_GEN`), so it fires once per
/// drag/snap/morph settle, not per pixel. Broadcasts `config:changed`
/// like any other write.
pub(crate) fn persist_bar_position(app: &AppHandle) {
    let state = app.state::<AppState>();
    // The idle capsule rect, not the live one: a drag while the bar is
    // expanded (480) must persist the capsule's anchor, else relaunch
    // shifts the capsule left by half the expansion.
    let rect = state.pool.lock().idle_bar_rect();
    let updated = {
        let mut cfg = state.config.lock();
        if cfg.window.bar_x == Some(rect.x) && cfg.window.bar_y == Some(rect.y) {
            return; // nothing moved since the last write
        }
        cfg.window.bar_x = Some(rect.x);
        cfg.window.bar_y = Some(rect.y);
        if let Err(e) = config::save(&cfg) {
            log::warn!("persist bar position failed: {e}");
            return;
        }
        cfg.clone()
    };
    let _ = app.emit("config:changed", &updated);
}

// ---------------------------------------------------------------------------
// Commands — keystore
// ---------------------------------------------------------------------------

/// `{"keys": masked_status()}` — the shape the Providers tab renders;
/// never carries plaintext. No init/unlock/lock commands exist: the
/// store is plaintext `keys.json` and is always ready.
#[tauri::command]
fn keystore_status(state: State<'_, AppState>) -> serde_json::Value {
    keystore_status_payload(&state.keystore.lock())
}

/// Validate the key against the provider FIRST — a bad key never reaches
/// `keys.json` — then store and broadcast `keystore:changed`. Async
/// because validation is a provider HTTP call.
#[tauri::command]
async fn keystore_set_key(
    app: AppHandle,
    provider: String,
    key: String,
) -> Result<serde_json::Value, String> {
    let Some(kind) = ProviderKind::from_str(&provider) else {
        return Err(format!("unknown provider {provider:?}"));
    };
    let (model, base_url) = {
        let state = app.state::<AppState>();
        provider_args(&state, kind)
    };
    make_provider(kind, Some(key.clone()), model, base_url)
        .validate()
        .await
        .map_err(|e| e.to_string())?;
    let state = app.state::<AppState>();
    let payload = {
        let mut ks = state.keystore.lock();
        ks.set_key(&provider, &key).map_err(|e| e.to_string())?;
        emit_keystore_changed(&app, &ks);
        keystore_status_payload(&ks)
    };
    Ok(payload)
}

/// Remove a provider key and broadcast `keystore:changed`.
#[tauri::command]
fn keystore_remove_key(app: AppHandle, provider: String) -> Result<serde_json::Value, String> {
    let state = app.state::<AppState>();
    let payload = {
        let mut ks = state.keystore.lock();
        ks.remove_key(&provider).map_err(|e| e.to_string())?;
        emit_keystore_changed(&app, &ks);
        keystore_status_payload(&ks)
    };
    Ok(payload)
}

// ---------------------------------------------------------------------------
// Commands — models
// ---------------------------------------------------------------------------

/// Test a candidate key WITHOUT storing it: `{"ok": true}` or
/// `{"ok": false, "error": "..."}` — validation failures are data, not
/// command errors, so the UI can render them inline.
#[tauri::command]
async fn model_validate_key(app: AppHandle, provider: String, key: String) -> serde_json::Value {
    let Some(kind) = ProviderKind::from_str(&provider) else {
        return json!({ "ok": false, "error": format!("unknown provider {provider:?}") });
    };
    let (model, base_url) = {
        let state = app.state::<AppState>();
        provider_args(&state, kind)
    };
    // `Some(key)` is also right for a keyless compatible endpoint: the
    // adapter treats an empty key as "no auth header", and the frontend
    // passes "" when the endpoint is open.
    match make_provider(kind, Some(key), model, base_url)
        .validate()
        .await
    {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `{"provider": ..., "model": ...}` — the provider that would answer an
/// ask right now (the first usable entry in `providers.order`), or `null`
/// when no provider is usable. Replaces the old `[models] llm_*` pair:
/// selection is now per-provider memory (`providers.models`) plus the
/// priority order — whichever enabled provider sorts first answers.
#[tauri::command]
fn model_get_selected(state: State<'_, AppState>) -> serde_json::Value {
    let cfg = state.config.lock();
    let ks = state.keystore.lock();
    match provider_candidates(&cfg, &ks).into_iter().next() {
        Some(c) => json!({ "provider": c.id, "model": c.model }),
        None => serde_json::Value::Null,
    }
}

/// Persist `provider`'s model pick to `providers.models.<id>`; returns
/// the EFFECTIVE selection — the first usable provider in
/// `providers.order` — or `null` when none is usable. A write for a
/// non-primary provider must not relabel who's answering, so the return
/// resolves the chain rather than echoing the write. Broadcasts
/// `config:changed` like every config write.
#[tauri::command]
fn model_set_selected(
    app: AppHandle,
    provider: String,
    model: String,
) -> Result<serde_json::Value, String> {
    if ProviderKind::from_str(&provider).is_none() {
        return Err(format!("unknown provider {provider:?}"));
    }
    let state = app.state::<AppState>();
    let (updated, resolved) = {
        let mut cfg = state.config.lock();
        cfg.providers.models.insert(provider, model);
        config::save(&cfg).map_err(|e| e.to_string())?;
        let ks = state.keystore.lock();
        let resolved = provider_candidates(&cfg, &ks).into_iter().next();
        (cfg.clone(), resolved)
    };
    let _ = app.emit("config:changed", &updated);
    Ok(match resolved {
        Some(c) => json!({ "provider": c.id, "model": c.model }),
        None => serde_json::Value::Null,
    })
}

/// Drag-order write from Settings → Providers: `order` must be a
/// permutation of the known provider ids — anything else is rejected so
/// a stale frontend can't silently drop or invent a provider.
#[tauri::command]
fn providers_reorder(app: AppHandle, order: Vec<String>) -> Result<Config, String> {
    let state = app.state::<AppState>();
    let mut sorted = order.clone();
    sorted.sort();
    let mut known: Vec<String> = ProviderKind::ALL
        .iter()
        .map(|k| k.as_str().to_string())
        .collect();
    known.sort();
    if sorted != known {
        return Err("order must be a permutation of the provider catalog".to_string());
    }
    let updated = {
        let mut cfg = state.config.lock();
        cfg.providers.order = order;
        config::save(&cfg).map_err(|e| e.to_string())?;
        cfg.clone()
    };
    let _ = app.emit("config:changed", &updated);
    Ok(updated)
}

/// Flip one provider's enabled switch. Disabled providers keep their
/// slot in `order` and their stored key — they're skipped at ask time
/// but stay visible (and draggable) in settings.
#[tauri::command]
fn provider_set_enabled(app: AppHandle, provider: String, enabled: bool) -> Result<Config, String> {
    if ProviderKind::from_str(&provider).is_none() {
        return Err(format!("unknown provider {provider:?}"));
    }
    let state = app.state::<AppState>();
    let updated = {
        let mut cfg = state.config.lock();
        if enabled {
            cfg.providers.disabled.retain(|d| d != &provider);
        } else if !cfg.providers.disabled.contains(&provider) {
            cfg.providers.disabled.push(provider);
        }
        config::save(&cfg).map_err(|e| e.to_string())?;
        cfg.clone()
    };
    let _ = app.emit("config:changed", &updated);
    Ok(updated)
}

/// Static per-provider lists (spec); Ollama resolves `GET /api/tags`,
/// a compatible endpoint resolves `GET {base}/models` with its stored key
/// (if any), and OpenRouter resolves `GET {OPENROUTER_BASE_URL}/models`
/// the same way — falling back to the curated static list when the fetch
/// comes back empty (offline, or a keyed listing with no key stored yet).
/// Unknown providers and fetch errors → empty list.
#[tauri::command]
async fn model_list_available(app: AppHandle, provider: String) -> Vec<String> {
    match ProviderKind::from_str(&provider) {
        Some(ProviderKind::Ollama) => ollama_models().await,
        Some(ProviderKind::Compatible) => {
            let state = app.state::<AppState>();
            let (base_url, key) = {
                let base = state.config.lock().compat.base_url.clone();
                // No stored key → `None` — the listing still works for
                // open endpoints and simply comes back empty for keyed
                // ones, matching the every-failure-is-[] contract.
                let key = state.keystore.lock().key(ProviderKind::Compatible.as_str());
                (base, key)
            };
            llm::compat::list_models(&base_url, key).await
        }
        Some(ProviderKind::OpenRouter) => {
            // `key()` returns an owned Option — the lock drops before await.
            let key = app
                .state::<AppState>()
                .keystore
                .lock()
                .key(ProviderKind::OpenRouter.as_str());
            let live = llm::compat::list_models(llm::compat::OPENROUTER_BASE_URL, key).await;
            if live.is_empty() {
                llm::static_models(ProviderKind::OpenRouter)
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            } else {
                live
            }
        }
        Some(kind) => llm::static_models(kind)
            .iter()
            .map(|s| s.to_string())
            .collect(),
        None => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Commands — ask
// ---------------------------------------------------------------------------

/// Fire an ask; returns after synchronous pre-flight — tokens stream to
/// the `bar` window as `ask:*` events on a spawned task.
#[tauri::command]
fn ask_send(app: AppHandle, text: String) {
    let state = app.state::<AppState>();
    // Crafted-invoke guard: the shipped UI gates sends behind `Main`,
    // but a crafted invoke during onboarding would otherwise proceed —
    // DB writes, network, and emits to a window that doesn't exist yet.
    if *state.gate.lock() != Gate::Main {
        log::warn!("ask_send dropped while gate != Main");
        return;
    }
    state.ask.send(&app, &state.deps(), &text);
}

/// Cancel the in-flight stream and collapse the card.
#[tauri::command]
fn ask_close(app: AppHandle) {
    let state = app.state::<AppState>();
    state.ask.close(&app, &state.pool);
}

/// The bar's camera affordance — same screen-only ask as `Cmd+Shift+S`
/// (fixed prompt, frame required). Same gate guard as `ask_send`.
#[tauri::command]
fn ask_send_screen_only(app: AppHandle) {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("ask_send_screen_only dropped while gate != Main");
        return;
    }
    state.ask.send_screen_only(&app, &state.deps());
}

/// `{"state": "idle"|"loading"|"streaming", "question": ..., "response":
/// ..., "error": {...}|null}` — the live tail a re-expanded chat
/// resyncs from (the persisted session already carries every completed
/// turn); `error` re-delivers the last `ask:error`, which can fire
/// before the webview's `listen()` is up (cold-open pre-flight errors).
#[tauri::command]
fn ask_current(state: State<'_, AppState>) -> serde_json::Value {
    state.ask.current_payload()
}

/// Phase-2 placeholder on the command surface — the listen pipeline
/// (dual STT + live summary) isn't implemented yet.
#[tauri::command]
fn listen_stub() -> &'static str {
    "listen arrives in Phase 2"
}

// ---------------------------------------------------------------------------
// Commands — windows
// ---------------------------------------------------------------------------

/// `Cmd+/` behaviour as a command: collapse/expand the unified card.
#[tauri::command]
fn window_toggle_all(app: AppHandle) {
    app.state::<AppState>().pool.lock().toggle_chat(&app);
}

/// Direct card open/close — the mic button's listen mode and the
/// `capture:permission-needed` collapse use it (toggle semantics would
/// close an open card when the user only wants to switch modes, and
/// `ask_close` would cancel an in-flight text-only ask).
#[tauri::command]
fn window_set_chat_open(app: AppHandle, open: bool) {
    app.state::<AppState>()
        .pool
        .lock()
        .set_chat_open(&app, open);
}

/// Same entry point as `Cmd+,` and the tray's Settings item.
#[tauri::command]
fn window_show_settings(app: AppHandle) {
    show_settings(&app);
}

/// Onboarding mode of the same prefs window — the startup first-run
/// opener and the sidebar's "Re-run setup" both come through here.
#[tauri::command]
fn window_show_onboarding(app: AppHandle) {
    app.state::<AppState>()
        .pool
        .lock()
        .show_prefs(&app, "onboarding");
}

/// Hide the prefs window, then reconcile the bar: if the hidden mode
/// was onboarding and the wizard is done, the bar floats again.
#[tauri::command]
fn window_hide_prefs(state: State<'_, AppState>) {
    state.pool.lock().hide_prefs();
    state.sync_bar_visibility();
}

/// The mode the prefs window was last shown in (`"settings"` |
/// `"onboarding"`, `""` before first use) — read on mount so a
/// `prefs:mode` emit that raced the loading webview still lands.
#[tauri::command]
fn prefs_mode(state: State<'_, AppState>) -> String {
    state.pool.lock().prefs_mode().to_string()
}

// ---------------------------------------------------------------------------
// Commands — alert toast
// ---------------------------------------------------------------------------

/// Raise the alert toast — the webview's only error surface (the bar
/// pill has no room to render one).
#[tauri::command]
fn alert_show(app: AppHandle, message: String) {
    show_alert(&app, &message);
}

/// The live alert payload, or `null` — read by the toast on mount so a
/// show that raced its listener still renders.
#[tauri::command]
fn alert_current(state: State<'_, AppState>) -> serde_json::Value {
    state
        .alert
        .lock()
        .clone()
        .unwrap_or(serde_json::Value::Null)
}

#[tauri::command]
fn alert_dismiss(state: State<'_, AppState>) {
    *state.alert.lock() = None;
    state.pool.lock().hide_alert();
}

/// `height` is the desired TOTAL window height (the frontend measures
/// the whole card) — the pool clamps and animates, anchored edge fixed.
/// Expanded-only.
#[tauri::command]
fn window_adjust_height(state: State<'_, AppState>, height: f64) {
    state.pool.lock().adjust_height(height);
}

/// The webview's pill⇄input morph signal — under liquid glass the
/// capsule IS the window, so the window resizes to match (idle 112,
/// expanded 480, same 64 height and capsule radius).
#[tauri::command]
fn window_set_bar_expanded(state: State<'_, AppState>, expanded: bool) {
    state.pool.lock().set_bar_expanded(expanded);
}

/// Settings → Bar picker: `edge` is `"top"|"bottom"|"left"|"right"`.
/// Snaps (animated) the bar to that work-area edge; the resulting `Moved`
/// event persists `window.bar_x/y` through the debounced write.
#[tauri::command]
fn window_snap_edge(state: State<'_, AppState>, edge: String) -> Result<(), String> {
    let dir = match edge.as_str() {
        "top" => windows::Dir::Up,
        "bottom" => windows::Dir::Down,
        "left" => windows::Dir::Left,
        "right" => windows::Dir::Right,
        _ => return Err(format!("unknown edge {edge:?}")),
    };
    state.pool.lock().snap_edge(dir);
    Ok(())
}

/// Settings → Bar "Re-center": restores the default position —
/// centered on the primary work area, just under the menu bar.
/// Persists through the same `Moved` debounce as a drag.
#[tauri::command]
fn window_recenter(state: State<'_, AppState>) {
    state.pool.lock().recenter_bar();
}

/// The edge the bar is currently nearest (`"top"` | `"bottom"` |
/// `"left"` | `"right"`) — the Bar picker's selected value. Recomputed
/// from the live rect so a just-finished drag reads correctly.
#[tauri::command]
fn window_bar_edge(state: State<'_, AppState>) -> String {
    let mut pool = state.pool.lock();
    pool.refresh_bar_rect();
    match pool.bar_edge() {
        windows::Dir::Up => "top",
        windows::Dir::Down => "bottom",
        windows::Dir::Left => "left",
        windows::Dir::Right => "right",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// Commands — permissions / capture
// ---------------------------------------------------------------------------

/// `{"screen": bool, "mic": "notDetermined"|"restricted"|"denied"|"authorized"}`.
#[tauri::command]
fn permissions_status() -> serde_json::Value {
    json!({
        "screen": permissions::screen_status(),
        "mic": permissions::mic_status(),
    })
}

/// `CGRequestScreenCaptureAccess` may show the system prompt, so it runs
/// on a blocking thread; the gate is re-evaluated afterwards.
#[tauri::command]
async fn permissions_request_screen(app: AppHandle) -> bool {
    let granted = tauri::async_runtime::spawn_blocking(permissions::screen_request)
        .await
        .unwrap_or(false);
    transition_gate(&app);
    granted
}

/// `AVCaptureDevice.requestAccess` MUST NOT run on the main thread — its
/// completion can dispatch to the main queue and deadlock a blocked main
/// thread — so the call is explicitly `spawn_blocking`'d.
#[tauri::command]
async fn permissions_request_mic() -> bool {
    tauri::async_runtime::spawn_blocking(permissions::mic_request)
        .await
        .unwrap_or(false)
}

/// `section` is the full privacy pane name (`Privacy_ScreenCapture`,
/// `Privacy_Microphone`, …). Fire-and-forget `open`.
#[tauri::command]
fn permissions_open_prefs(section: String) -> Result<(), String> {
    permissions::open_prefs(&section).map_err(|e| e.to_string())
}

/// `{"running": bool, "frames": ring.len()}`.
#[tauri::command]
fn capture_status(state: State<'_, AppState>) -> serde_json::Value {
    let running = state
        .capture
        .lock()
        .as_ref()
        .is_some_and(MacosCapture::is_running);
    json!({ "running": running, "frames": state.ring.lock().len() })
}

// ---------------------------------------------------------------------------
// Commands — sessions
// ---------------------------------------------------------------------------

#[tauri::command]
fn session_list(state: State<'_, AppState>) -> Result<Vec<Session>, String> {
    state.db.session_list().map_err(|e| e.to_string())
}

#[tauri::command]
fn session_get(state: State<'_, AppState>, id: i64) -> Result<Vec<AiMessage>, String> {
    state.db.ai_messages_for(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn session_delete(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.db.session_delete(id).map_err(|e| e.to_string())
}

/// "New chat": end the active session of `kind` (`"ask"`) so the next
/// send starts a fresh conversation. `true` when one was ended, `false`
/// when none was open (no junk row created).
#[tauri::command]
fn session_end_active(state: State<'_, AppState>, kind: String) -> Result<bool, String> {
    match state.db.session_active_id(&kind).map_err(|e| e.to_string())? {
        Some(id) => {
            state.db.session_end(id).map_err(|e| e.to_string())?;
            Ok(true)
        }
        None => Ok(false),
    }
}

// ---------------------------------------------------------------------------
// Commands — config / app
// ---------------------------------------------------------------------------

#[tauri::command]
fn config_get(state: State<'_, AppState>) -> Config {
    state.config.lock().clone()
}

/// Limited writable surface: `hotkeys.<action>`, `window.bar_x`,
/// `window.bar_y` (number sets, null clears), `app.onboarding_done`
/// (bool), `app.appearance` (`auto|light|dark`), `app.accent`
/// (`#rrggbb`, `""` resets to the spec slate), `compat.name`,
/// `compat.base_url` (validated http(s) URL; `""` clears),
/// `models.stt_provider` (`deepgram|whisper`), and `models.stt_model`
/// (a trimmed non-empty identifier). Provider order/switches/models have
/// their own commands (`providers_reorder`,
/// `provider_set_enabled`, `model_set_selected`). Persists `config.toml`
/// and returns the updated config. A `hotkeys.*` write re-registers the
/// active set (full or limited, matching the current gate). Every
/// successful write broadcasts `config:changed` so open windows
/// re-render (appearance flips, provider lists, the bar's drag hint).
///
/// `app.onboarding_done` is the wizard's completion write: `true` ends
/// onboarding → `transition_gate` can now reach `Main` (capture, full
/// hotkeys, the card) and the bar appears; `false` (a re-run) reverses it.
#[tauri::command]
fn config_set(app: AppHandle, key: String, value: serde_json::Value) -> Result<Config, String> {
    let state = app.state::<AppState>();
    let mut hotkeys_changed = false;
    let mut onboarding_changed = false;
    {
        let mut cfg = state.config.lock();
        match key.as_str() {
            "window.bar_x" => cfg.window.bar_x = window_pref_value(&value)?,
            "window.bar_y" => cfg.window.bar_y = window_pref_value(&value)?,
            "app.onboarding_done" => {
                let v = value
                    .as_bool()
                    .ok_or("app.onboarding_done must be a bool")?;
                onboarding_changed = cfg.app.onboarding_done != v;
                cfg.app.onboarding_done = v;
            }
            "app.appearance" => {
                let v = value.as_str().ok_or("app.appearance must be a string")?;
                if !matches!(v, "auto" | "light" | "dark") {
                    return Err(format!("unknown appearance {v:?}"));
                }
                cfg.app.appearance = v.to_string();
            }
            "app.accent" => {
                let v = value
                    .as_str()
                    .ok_or("app.accent must be a string")?
                    .trim()
                    .to_string();
                if v.is_empty() {
                    cfg.app.accent = config::DEFAULT_ACCENT.to_string();
                } else {
                    let ok = v.len() == 7
                        && v.starts_with('#')
                        && v[1..].chars().all(|c| c.is_ascii_hexdigit());
                    if !ok {
                        return Err("app.accent must be a #rrggbb color".to_string());
                    }
                    cfg.app.accent = v;
                }
            }
            "compat.name" => {
                cfg.compat.name = value
                    .as_str()
                    .ok_or("compat.name must be a string")?
                    .trim()
                    .to_string();
            }
            "compat.base_url" => {
                let v = value
                    .as_str()
                    .ok_or("compat.base_url must be a string")?
                    .trim();
                if !v.is_empty() && !llm::compat::is_valid_base_url(v) {
                    return Err("compat.base_url must be an http(s):// URL".to_string());
                }
                cfg.compat.base_url = v.to_string();
            }
            "models.stt_provider" => {
                let v = value
                    .as_str()
                    .ok_or("models.stt_provider must be a string")?;
                cfg.models.stt_provider = config::validate_stt_provider(v)?;
            }
            "models.stt_model" => {
                let v = value
                    .as_str()
                    .ok_or("models.stt_model must be a string")?;
                cfg.models.stt_model = config::validate_stt_model(v)?;
            }
            "vision.provider" => {
                let v = value
                    .as_str()
                    .ok_or("vision.provider must be a string")?
                    .trim();
                let ok = v.is_empty() || ProviderKind::from_str(v).is_some_and(|k| k.is_vision());
                if !ok {
                    return Err(format!("unknown vision provider {v:?}"));
                }
                cfg.vision.provider = v.to_string();
            }
            _ if key.starts_with("vision.models.") => {
                let id = &key["vision.models.".len()..];
                if ProviderKind::from_str(id).is_none_or(|k| !k.is_vision()) {
                    return Err(format!("unknown vision provider {id:?}"));
                }
                let model = value
                    .as_str()
                    .ok_or("vision.models.* must be a string")?
                    .trim();
                if model.is_empty() {
                    cfg.vision.models.remove(id);
                } else {
                    cfg.vision.models.insert(id.to_string(), model.to_string());
                }
            }
            _ if key.starts_with("hotkeys.") => {
                let name = &key["hotkeys.".len()..];
                let accel = value.as_str().ok_or("hotkey binding must be a string")?;
                if hotkey::accelerator_for(accel).is_none() {
                    return Err(format!("accelerator {accel:?} doesn't parse"));
                }
                cfg.hotkeys.insert(name.to_string(), accel.to_string());
                hotkeys_changed = true;
            }
            _ => return Err(format!("unknown or read-only config key {key:?}")),
        }
        config::save(&cfg).map_err(|e| e.to_string())?;
    }
    if hotkeys_changed {
        let all = *state.gate.lock() == Gate::Main;
        swap_hotkeys(&app, all);
    }
    if onboarding_changed {
        // Gate first: `enter_main` starts capture while the
        // wizard is still the visible window; then the bar un-hides.
        transition_gate(&app);
        state.sync_bar_visibility();
    }
    let updated = state.config.lock().clone();
    let _ = app.emit("config:changed", &updated);
    Ok(updated)
}

/// `"glass" | "vibrancy" | "none"` — which native material backs the
/// overlay windows. macOS 26+ reports glass; older macOS gets the
/// plugin's NSVisualEffectView fallback; other OSes get none (the
/// webview keeps its CSS frost).
#[tauri::command]
fn surface_material(app: AppHandle) -> &'static str {
    #[cfg(target_os = "macos")]
    {
        if app.liquid_glass().is_supported() {
            "glass"
        } else {
            "vibrancy"
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        "none"
    }
}

#[tauri::command]
fn quit_application(app: AppHandle) {
    app.exit(0);
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::try_init();
    tauri::Builder::default()
        // Order matters: the deep-link plugin must be registered before
        // `deeplink::init` resolves `app.deep_link()` inside `setup`.
        .plugin(tauri_plugin_deep_link::init())
        // Required before `app.global_shortcut()` (hotkey registration).
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_liquid_glass::init())
        .setup(|app| {
            let handle = app.handle();
            // `~/.marvis` must exist before Db/keystore touch it.
            paths::root();
            let db = Db::open()?;
            let cfg = config::load();
            let onboarding_done = cfg.app.onboarding_done;
            // Plaintext `keys.json` — loads eagerly; a missing/corrupt
            // file is just an empty store, never a gate.
            let keystore = Keystore::new();
            // Bar only — it hosts the chat/listen card modes itself; the
            // bar stays hidden until onboarding is done.
            let pool = WindowPool::create_bar_only(handle, onboarding_done)?;
            app.manage(AppState {
                keystore: Mutex::new(keystore),
                config: Mutex::new(cfg),
                db: Arc::new(db),
                ring: Arc::new(Mutex::new(RingBuffer::new(RING_MAX_FRAMES, RING_MAX_BYTES))),
                capture: Mutex::new(None),
                ask: Arc::new(AskService::new()),
                pool: Mutex::new(pool),
                hotkeys: Mutex::new(None),
                gate: Mutex::new(Gate::NeedsPermission),
                gate_transition: Mutex::new(()),
                alert: Mutex::new(None),
            });
            deeplink::init(handle, deeplink_dispatch(handle))?;
            // Warn-and-continue like hotkeys: a missing tray must never
            // wedge startup.
            if let Err(e) = tray::init(handle, tray_menu_dispatch()) {
                log::warn!("tray init failed: {e}");
            }
            // Gated limited set until the gate reaches `Main`; kept in
            // state so the swap contract can unregister it on upgrade.
            let binds = handle.state::<AppState>().config.lock().hotkeys.clone();
            match hotkey::register_limited(handle, &binds, hotkey_dispatch(handle)) {
                Ok(set) => {
                    handle.state::<AppState>().hotkeys.lock().replace(set);
                }
                Err(e) => log::warn!("startup hotkey registration failed: {e}"),
            }
            // Computes the gate, enters `Main` if it's already open, and
            // emits `app:state` either way. Onboarding blocks `Main`, so
            // a first run can never light capture/hotkeys here.
            transition_gate(handle);
            // First run (or any install that predates onboarding_done):
            // the wizard takes the decorated prefs window — alone; the
            // bar was created hidden and `show_prefs` re-syncs its
            // visibility for the mode.
            if !onboarding_done {
                handle
                    .state::<AppState>()
                    .pool
                    .lock()
                    .show_prefs(handle, "onboarding");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            keystore_status,
            keystore_set_key,
            keystore_remove_key,
            model_validate_key,
            model_get_selected,
            model_set_selected,
            model_list_available,
            providers_reorder,
            provider_set_enabled,
            ask_send,
            ask_close,
            ask_send_screen_only,
            ask_current,
            listen_stub,
            alert_show,
            alert_current,
            alert_dismiss,
            window_toggle_all,
            window_set_chat_open,
            window_show_settings,
            window_show_onboarding,
            window_hide_prefs,
            prefs_mode,
            window_adjust_height,
            window_snap_edge,
            window_recenter,
            window_bar_edge,
            window_set_bar_expanded,
            permissions_status,
            permissions_request_screen,
            permissions_request_mic,
            permissions_open_prefs,
            capture_status,
            session_list,
            session_get,
            session_delete,
            session_end_active,
            config_get,
            config_set,
            surface_material,
            quit_application,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Unique temp dir per test; `Db::at` creates it.
    fn tmp_dir() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "marvis-lib-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// Brief smoke test: `config_get` against a temp `AppState` returns
    /// defaults — proves `for_test` builds without a runtime and every
    /// field is wired.
    #[test]
    fn config_get_returns_defaults_on_temp_state() {
        let tmp = tmp_dir();
        let state = AppState::for_test(&tmp);
        assert_eq!(state.config.lock().clone(), Config::default());
        // Fresh dir → onboarding isn't done, so the gate can't be `Main`
        // even if this CI machine happens to have screen permission.
        assert_eq!(app_gate(&state), Gate::NeedsPermission);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The failover chain honours `providers.order`, skips disabled ids,
    /// and drops providers with no key (where required) or no model.
    #[test]
    fn provider_chain_respects_order_enablement_and_usability() {
        let tmp = tmp_dir();
        let state = AppState::for_test(&tmp);
        {
            let mut ks = state.keystore.lock();
            ks.set_key("openai", "sk-a").unwrap();
            ks.set_key("gemini", "AIza-b").unwrap();
        }
        {
            let mut cfg = state.config.lock();
            cfg.providers.order = vec![
                "openai".into(),
                "gemini".into(),
                "anthropic".into(),
                "openrouter".into(),
                "ollama".into(),
                "compatible".into(),
            ];
            cfg.providers.disabled = vec!["gemini".into()];
        }
        let (cfg, ks) = (state.config.lock(), state.keystore.lock());
        let chain = provider_candidates(&cfg, &ks);
        let ids: Vec<&str> = chain.iter().map(|c| c.id.as_str()).collect();
        // openai (keyed) first; gemini disabled; anthropic/openrouter
        // dropped (no key); ollama/compatible dropped (no model set:
        // their static list is empty).
        assert_eq!(ids, vec!["openai"]);
        assert_eq!(chain[0].model, "gpt-4o");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A selected-but-disabled or keyless provider yields to the next
    /// usable entry — this is the fallback the user asked for.
    #[test]
    fn provider_chain_falls_back_past_unusable_providers() {
        let tmp = tmp_dir();
        let state = AppState::for_test(&tmp);
        {
            let mut ks = state.keystore.lock();
            ks.set_key("gemini", "AIza-b").unwrap();
            ks.set_key("anthropic", "sk-ant-c").unwrap();
        }
        {
            let mut cfg = state.config.lock();
            // openai first but no key; gemini disabled; anthropic wins.
            cfg.providers.disabled = vec!["gemini".into()];
        }
        let (cfg, ks) = (state.config.lock(), state.keystore.lock());
        let chain = provider_candidates(&cfg, &ks);
        let ids: Vec<&str> = chain.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["anthropic"]);
        assert_eq!(chain[0].model, "claude-sonnet-4-5");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Ollama and a configured compatible endpoint need no key — both
    /// stay in the chain once they can resolve a model.
    #[test]
    fn provider_chain_keeps_keyless_providers_with_models() {
        let tmp = tmp_dir();
        let state = AppState::for_test(&tmp);
        {
            let mut cfg = state.config.lock();
            cfg.providers
                .models
                .insert("ollama".into(), "qwen3:8b".into());
            cfg.providers
                .models
                .insert("compatible".into(), "llama-3.3-70b".into());
            cfg.compat.base_url = "http://localhost:8000/v1".into();
        }
        let (cfg, ks) = (state.config.lock(), state.keystore.lock());
        let ids: Vec<String> = provider_candidates(&cfg, &ks)
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, vec!["ollama", "compatible"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A compatible provider with no `base_url` configured is unusable —
    /// it must not enter the chain on key-memory alone.
    #[test]
    fn provider_chain_requires_a_base_url_for_compatible() {
        let tmp = tmp_dir();
        let state = AppState::for_test(&tmp);
        {
            let mut cfg = state.config.lock();
            cfg.providers
                .models
                .insert("compatible".into(), "llama-3.3-70b".into());
        }
        let (cfg, ks) = (state.config.lock(), state.keystore.lock());
        let ids: Vec<String> = provider_candidates(&cfg, &ks)
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The screen reader is off until `vision.provider` names a usable
    /// pick — keyed for hosted, `compat.base_url` + a model for
    /// compatible — and resolves its model from `vision.models` or the
    /// vision default.
    #[test]
    fn vision_candidate_resolves_only_when_usable() {
        let tmp = tmp_dir();
        let state = AppState::for_test(&tmp);
        {
            let mut ks = state.keystore.lock();
            ks.set_key("gemini", "AIza-b").unwrap();
        }
        let (cfg, ks) = (state.config.lock(), state.keystore.lock());

        // Unset → off.
        assert!(vision_candidate(&cfg, &ks).is_none());

        // A keyed provider resolves its vision default model.
        let mut cfg = cfg.clone();
        cfg.vision.provider = "gemini".into();
        let c = vision_candidate(&cfg, &ks).unwrap();
        assert_eq!(c.id, "gemini");
        assert_eq!(c.model, "gemini-2.5-flash");

        // `vision.models` beats the default.
        cfg.vision
            .models
            .insert("gemini".into(), "gemini-2.5-pro".into());
        assert_eq!(vision_candidate(&cfg, &ks).unwrap().model, "gemini-2.5-pro");

        // A hosted pick with no key is unusable → off.
        cfg.vision.provider = "openai".into();
        assert!(vision_candidate(&cfg, &ks).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The reader ignores `providers.disabled` — a chat-disabled provider
    /// may still read the screen. Compatible additionally needs the
    /// shared endpoint and an explicit model id.
    #[test]
    fn vision_candidate_ignores_disabled_and_supports_compatible() {
        let tmp = tmp_dir();
        let state = AppState::for_test(&tmp);
        {
            let mut ks = state.keystore.lock();
            ks.set_key("openai", "sk-a").unwrap();
        }
        let mut cfg = state.config.lock().clone();
        cfg.providers.disabled = vec!["openai".into()];
        cfg.vision.provider = "openai".into();
        let ks = state.keystore.lock();
        let c = vision_candidate(&cfg, &ks).unwrap();
        assert_eq!(c.id, "openai");

        // Compatible: no endpoint → unusable; endpoint but no model →
        // unusable; both → resolves the typed id.
        cfg.vision.provider = "compatible".into();
        assert!(vision_candidate(&cfg, &ks).is_none());
        cfg.compat.base_url = "http://localhost:8000/v1".into();
        assert!(vision_candidate(&cfg, &ks).is_none());
        cfg.vision
            .models
            .insert("compatible".into(), "qwen2.5-vl".into());
        assert_eq!(vision_candidate(&cfg, &ks).unwrap().model, "qwen2.5-vl");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
