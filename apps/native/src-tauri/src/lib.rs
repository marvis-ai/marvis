//! `lib.rs` — `AppState`, the command surface, and the startup sequence.
//!
//! Wires every module together: `AppState` is managed via `app.manage(...)`
//! in [`run`]'s `setup`, then reached from commands (`State<'_, AppState>`
//! / `AppHandle`), the hotkey dispatch closure, and the deep-link dispatch
//! closure (both re-resolve state per event so they survive gate
//! transitions).
//!
//! ## App gate (Glass `handleHeaderStateChanged` parity)
//!
//! [`Gate`] = `NeedsUnlock` (no/locked `keys.enc`) → `NeedsPermission`
//! (unlocked, no screen-recording permission) → `Main`. Feature panels,
//! the full hotkey set, and screen capture exist only in `Main`.
//! [`transition_gate`] recomputes the gate after every mutation that can
//! change it (`keystore_init`/`unlock`/`lock`, `permissions_request_screen`,
//! and once at startup) and ALWAYS emits `app:state`.
//!
//! ## Event payloads (webview contract)
//!
//! - `app:state` `{"gate": "needs_unlock"|"needs_permission"|"main"}` —
//!   broadcast to every window on each `transition_gate` call.
//! - `keystore:changed` — the `keystore_status` payload
//!   `{"state": ..., "keys": ...}`, broadcast after every keystore
//!   mutation (init/unlock/lock/set_key/remove_key).
//! - `ask:scroll` `{"dir": "up"|"down"}` — to the `ask` window only,
//!   fired by the scroll hotkeys.

mod ask;
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
mod system_auth;
mod windows;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};

use ask::AskService;
use capture::{FrameSource, MacosCapture, RingBuffer};
use config::Config;
use hotkey::RegisteredHotkeys;
use keystore::{Keystore, KeystoreState};
use llm::{make_provider, ProviderKind};
use storage::{AiMessage, Db, Session};
use windows::{Panel, WindowPool};

/// Frame ring caps from the spec: 120 frames / 64 MB (~60 s horizon).
const RING_MAX_FRAMES: usize = 120;
const RING_MAX_BYTES: usize = 64 * 1024 * 1024;

/// Local Ollama daemon's model list (same host the adapter streams from).
const OLLAMA_TAGS_URL: &str = "http://localhost:11434/api/tags";

/// Which UI state the bar may show. Feature panels, the full hotkey set,
/// and capture exist only in `Main`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// `keys.enc` missing or locked — the bar shows the unlock card.
    NeedsUnlock,
    /// Unlocked but no screen-recording permission — permission card.
    NeedsPermission,
    /// Fully live: panels, full hotkeys, capture.
    Main,
}

impl Gate {
    /// The `app:state` payload value.
    fn name(self) -> &'static str {
        match self {
            Gate::NeedsUnlock => "needs_unlock",
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
    click_through: AtomicBool,
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

    /// Headless constructor for tests: filesystem-bound fields bind under
    /// `root` so tests never touch `~/.marvis`; windows/capture/hotkeys
    /// need a runtime, so the pool is empty and those slots stay `None`.
    /// The keystore gets a `StaticDekProvider` — no Keychain/LA in tests.
    #[cfg(test)]
    fn for_test(root: &std::path::Path) -> Self {
        Self {
            keystore: Mutex::new(Keystore::at(
                root.join("keys.enc"),
                Arc::new(system_auth::StaticDekProvider([7; 32])),
            )),
            config: Mutex::new(Config::load_from(root.join("config.toml")).unwrap_or_default()),
            db: Arc::new(Db::at(root.join("marvis.db")).expect("test db")),
            ring: Arc::new(Mutex::new(RingBuffer::new(RING_MAX_FRAMES, RING_MAX_BYTES))),
            capture: Mutex::new(None),
            ask: Arc::new(AskService::new()),
            pool: Mutex::new(WindowPool::new_empty()),
            hotkeys: Mutex::new(None),
            gate: Mutex::new(Gate::NeedsUnlock),
            gate_transition: Mutex::new(()),
            click_through: AtomicBool::new(false),
        }
    }
}

// ---------------------------------------------------------------------------
// App gate
// ---------------------------------------------------------------------------

/// `Unset|Locked → NeedsUnlock`; `Unlocked && !screen_status() →
/// NeedsPermission`; else `Main`.
fn app_gate(state: &AppState) -> Gate {
    match state.keystore.lock().state() {
        KeystoreState::Unset | KeystoreState::Locked => Gate::NeedsUnlock,
        KeystoreState::Unlocked(_) => {
            if permissions::screen_status() {
                Gate::Main
            } else {
                Gate::NeedsPermission
            }
        }
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

/// `Main` entry: feature panels (created once — the call is idempotent),
/// full hotkey set, capture. Every step warns and continues — a failed
/// piece must never wedge the gate.
fn enter_main(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Err(e) = state.pool.lock().create_feature_windows(app) {
        log::warn!("gate: create_feature_windows failed: {e}");
    }
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

/// `Main` exit (keystore lock): stop + drop capture, hide every panel,
/// downgrade hotkeys to the gated limited set.
fn leave_main(app: &AppHandle) {
    let state = app.state::<AppState>();
    // Cancel any in-flight ask — an unbounded stream left running would
    // hold `AskState::Streaming` past re-unlock and wedge every future
    // send on the busy-check until it resolves on its own.
    state.ask.close(&state.pool);
    // Take the capture out and release the lock BEFORE `stop()` — it
    // joins the capture worker, which must not hold `state.capture`
    // while `capture_status` waits on it.
    let capture = state.capture.lock().take();
    if let Some(capture) = capture {
        capture.stop();
    }
    {
        let mut pool = state.pool.lock();
        for panel in Panel::ALL {
            pool.hide(panel);
        }
    }
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
            hotkey::Action::ToggleVisibility => state.pool.lock().toggle_all(),
            // `next_step` fires the screen-only ask (Task-11 brief's table).
            hotkey::Action::NextStep | hotkey::Action::ScreenOnly => {
                state.ask.send_screen_only(&app, &state.deps());
            }
            hotkey::Action::Move(dir) => state.pool.lock().move_bar_step(dir),
            hotkey::Action::ToggleClickThrough => {
                let on = !state.click_through.fetch_not(Ordering::Relaxed);
                state.pool.lock().set_click_through(on);
            }
            hotkey::Action::ScrollUp => {
                let _ = app.emit_to("ask", "ask:scroll", json!({ "dir": "up" }));
            }
            hotkey::Action::ScrollDown => {
                let _ = app.emit_to("ask", "ask:scroll", json!({ "dir": "down" }));
            }
            hotkey::Action::MoveToDisplay(n) => state.pool.lock().move_bar_to_display(n),
            hotkey::Action::SnapEdge(dir) => state.pool.lock().snap_edge(dir),
        }
    }
}

/// Map [`deeplink::Action`]s: `Ask` runs only while the gate is `Main`
/// (gated — the bar shows the unlock card anyway); `Focus` surfaces the
/// bar; `Ignore` never reaches the dispatch (filtered inside `init`).
fn deeplink_dispatch(app: &AppHandle) -> impl Fn(deeplink::Action) + Send + Sync + 'static {
    let app = app.clone();
    move |action| match action {
        deeplink::Action::Ask(text) => {
            let state = app.state::<AppState>();
            let bar = state.pool.lock().bar().cloned();
            if let Some(bar) = bar {
                let _ = bar.set_focus();
            }
            if *state.gate.lock() == Gate::Main {
                state.ask.send(&app, &state.deps(), &text);
            } else {
                // Not ready yet — the spec's catch-all applies: surface
                // the bar so the user lands on the unlock/permission card
                // instead of the link silently going nowhere.
                log::warn!(
                    "deeplink: ask gated off (gate = {:?}); focused bar",
                    *state.gate.lock()
                );
            }
        }
        deeplink::Action::Focus => {
            let state = app.state::<AppState>();
            // Clone the handle out so the pool guard drops before `state`.
            let bar = state.pool.lock().bar().cloned();
            if let Some(bar) = bar {
                let _ = bar.set_focus();
            }
        }
        deeplink::Action::Ignore => {}
    }
}

// ---------------------------------------------------------------------------
// Shared payload/validation helpers
// ---------------------------------------------------------------------------

/// `keystore_status` return value and `keystore:changed` payload:
/// `{"state": "Unset"|"Locked"|"Unlocked", "keys": [[provider, "…last4"|null], …]}`.
/// Masked only — plaintext keys never leave `Keystore`.
fn keystore_status_payload(keystore: &Keystore) -> serde_json::Value {
    let state = match keystore.state() {
        KeystoreState::Unset => "Unset",
        KeystoreState::Locked => "Locked",
        KeystoreState::Unlocked(_) => "Unlocked",
    };
    json!({ "state": state, "keys": keystore.masked_status() })
}

/// Broadcast the (masked) keystore status after any mutation.
fn emit_keystore_changed(app: &AppHandle, keystore: &Keystore) {
    let _ = app.emit("keystore:changed", keystore_status_payload(keystore));
}

/// Static per-provider model lists (spec); Ollama's comes from the daemon.
fn static_models(kind: ProviderKind) -> &'static [&'static str] {
    match kind {
        ProviderKind::OpenAi => &["gpt-4o", "gpt-4o-mini", "o4-mini"],
        ProviderKind::Anthropic => &["claude-sonnet-4-5", "claude-opus-4-1"],
        ProviderKind::Gemini => &["gemini-2.5-pro", "gemini-2.5-flash"],
        ProviderKind::Ollama => &[],
    }
}

/// Model for `validate()` probes: the configured selection when it belongs
/// to this provider, else the provider's first static model (`""` for
/// Ollama — its `validate` hits `/api/tags` and never names a model).
fn validate_model(state: &AppState, kind: ProviderKind) -> String {
    let cfg = state.config.lock();
    if cfg.models.llm_provider == kind.as_str() && !cfg.models.llm_model.is_empty() {
        cfg.models.llm_model.clone()
    } else {
        static_models(kind)
            .first()
            .copied()
            .unwrap_or_default()
            .to_string()
    }
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

// ---------------------------------------------------------------------------
// Commands — keystore
// ---------------------------------------------------------------------------

/// `{"state": "Unset"|"Locked"|"Unlocked", "keys": masked_status()}` — the
/// shape Bar/SettingsPanel render; never carries plaintext.
#[tauri::command]
fn keystore_status(state: State<'_, AppState>) -> serde_json::Value {
    keystore_status_payload(&state.keystore.lock())
}

/// First-run: create `keys.enc` (empty keyring) and leave the store
/// unlocked, then re-evaluate the gate. Returns the new status payload.
/// Refuses to run once a keystore exists — `Keystore::init` overwrites
/// `keys.enc`, so a stray invoke while Locked/Unlocked would destroy
/// every stored key. `init` is silent — the DEK keychain item is created
/// without an auth prompt (the next `keystore_unlock` prompts).
#[tauri::command]
fn keystore_init(app: AppHandle) -> Result<serde_json::Value, String> {
    let state = app.state::<AppState>();
    {
        let mut ks = state.keystore.lock();
        if !matches!(ks.state(), KeystoreState::Unset) {
            return Err("keystore already initialized".to_string());
        }
        ks.init().map_err(|e| e.to_string())?;
        emit_keystore_changed(&app, &ks);
    }
    transition_gate(&app);
    let payload = keystore_status_payload(&state.keystore.lock());
    Ok(payload)
}

/// Decrypt `keys.enc` into memory — fetching the DEK shows the
/// system-auth prompt (Touch ID / password); cancels and keychain errors
/// surface as the command error. Re-evaluates the gate on success.
#[tauri::command]
fn keystore_unlock(app: AppHandle) -> Result<serde_json::Value, String> {
    let state = app.state::<AppState>();
    {
        let mut ks = state.keystore.lock();
        ks.unlock().map_err(|e| e.to_string())?;
        emit_keystore_changed(&app, &ks);
    }
    transition_gate(&app);
    let payload = keystore_status_payload(&state.keystore.lock());
    Ok(payload)
}

/// Delete `keys.enc` → `Unset` — the recovery path when the file is
/// `Obsolete`/corrupt or the keychain DEK is lost. The keychain item is
/// kept; the next `keystore_init` reuses it.
#[tauri::command]
fn keystore_reset(app: AppHandle) -> Result<serde_json::Value, String> {
    let state = app.state::<AppState>();
    {
        let mut ks = state.keystore.lock();
        ks.reset().map_err(|e| e.to_string())?;
        emit_keystore_changed(&app, &ks);
    }
    transition_gate(&app);
    let payload = keystore_status_payload(&state.keystore.lock());
    Ok(payload)
}

/// Drop the keyring back to `Locked`; the gate leaves `Main` (capture
/// stops, panels hide, hotkeys downgrade) inside `transition_gate`.
#[tauri::command]
fn keystore_lock(app: AppHandle) -> Result<serde_json::Value, String> {
    let state = app.state::<AppState>();
    {
        let mut ks = state.keystore.lock();
        ks.lock();
        emit_keystore_changed(&app, &ks);
    }
    transition_gate(&app);
    let payload = keystore_status_payload(&state.keystore.lock());
    Ok(payload)
}

/// Validate the key against the provider FIRST — a bad key never reaches
/// `keys.enc` — then store and broadcast `keystore:changed`. Async because
/// validation is a provider HTTP call.
#[tauri::command]
async fn keystore_set_key(
    app: AppHandle,
    provider: String,
    key: String,
) -> Result<serde_json::Value, String> {
    let Some(kind) = ProviderKind::from_str(&provider) else {
        return Err(format!("unknown provider {provider:?}"));
    };
    let model = {
        let state = app.state::<AppState>();
        validate_model(&state, kind)
    };
    make_provider(kind, Some(key.clone()), model)
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
    let model = {
        let state = app.state::<AppState>();
        validate_model(&state, kind)
    };
    match make_provider(kind, Some(key), model).validate().await {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `{"provider": ..., "model": ...}` — the configured LLM selection.
#[tauri::command]
fn model_get_selected(state: State<'_, AppState>) -> serde_json::Value {
    let cfg = state.config.lock();
    json!({ "provider": cfg.models.llm_provider, "model": cfg.models.llm_model })
}

/// Persist the LLM selection to `config.toml`; returns the saved pair.
#[tauri::command]
fn model_set_selected(
    state: State<'_, AppState>,
    provider: String,
    model: String,
) -> Result<serde_json::Value, String> {
    if ProviderKind::from_str(&provider).is_none() {
        return Err(format!("unknown provider {provider:?}"));
    }
    let mut cfg = state.config.lock();
    cfg.models.llm_provider = provider;
    cfg.models.llm_model = model;
    config::save(&cfg).map_err(|e| e.to_string())?;
    Ok(json!({ "provider": cfg.models.llm_provider, "model": cfg.models.llm_model }))
}

/// Static per-provider lists (spec); Ollama resolves `GET /api/tags`.
/// Unknown providers and Ollama fetch errors → empty list.
#[tauri::command]
async fn model_list_available(provider: String) -> Vec<String> {
    match ProviderKind::from_str(&provider) {
        Some(ProviderKind::Ollama) => ollama_models().await,
        Some(kind) => static_models(kind).iter().map(|s| s.to_string()).collect(),
        None => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Commands — ask
// ---------------------------------------------------------------------------

/// Fire an ask; returns after synchronous pre-flight — tokens stream to
/// the `ask` window as `ask:*` events on a spawned task.
#[tauri::command]
fn ask_send(app: AppHandle, text: String) {
    let state = app.state::<AppState>();
    // Crafted-invoke guard: the shipped UI gates sends behind `Main`,
    // but an Ollama-configured ask would otherwise proceed while locked —
    // DB writes, network, and emits to a hidden window. Drop instead.
    if *state.gate.lock() != Gate::Main {
        log::warn!("ask_send dropped while gate != Main");
        return;
    }
    state.ask.send(&app, &state.deps(), &text);
}

/// Cancel the in-flight stream and hide the ask panel.
#[tauri::command]
fn ask_close(state: State<'_, AppState>) {
    state.ask.close(&state.pool);
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

/// `Cmd+/` behaviour as a command: hide all panels / restore the set.
#[tauri::command]
fn window_toggle_all(state: State<'_, AppState>) {
    state.pool.lock().toggle_all();
}

#[tauri::command]
fn window_show_settings(state: State<'_, AppState>) {
    let pool = state.pool.lock();
    let mut pool = pool;
    pool.show(Panel::Settings);
    if let Some(win) = pool.panel_window(Panel::Settings) {
        let _ = win.set_focus();
    }
}

#[tauri::command]
fn window_hide_settings(state: State<'_, AppState>) {
    state.pool.lock().hide(Panel::Settings);
}

/// `name` is the window label (`"ask"|"listen"|"settings"`); `height` is
/// the requested content height in px — the pool clamps and animates.
#[tauri::command]
fn window_adjust_height(state: State<'_, AppState>, name: String, height: f64) {
    state.pool.lock().adjust_height(&name, height);
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

// ---------------------------------------------------------------------------
// Commands — config / app
// ---------------------------------------------------------------------------

#[tauri::command]
fn config_get(state: State<'_, AppState>) -> Config {
    state.config.lock().clone()
}

/// Limited writable surface: `models.llm_provider`, `models.llm_model`,
/// `hotkeys.<action>`, `window.bar_x`, `window.bar_y` (number sets, null
/// clears). Persists `config.toml` and returns the updated config. A
/// `hotkeys.*` write re-registers the active set (full or limited,
/// matching the current gate).
#[tauri::command]
fn config_set(app: AppHandle, key: String, value: serde_json::Value) -> Result<Config, String> {
    let state = app.state::<AppState>();
    let mut hotkeys_changed = false;
    {
        let mut cfg = state.config.lock();
        match key.as_str() {
            "models.llm_provider" => {
                let v = value
                    .as_str()
                    .ok_or("models.llm_provider must be a string")?;
                if ProviderKind::from_str(v).is_none() {
                    return Err(format!("unknown provider {v:?}"));
                }
                cfg.models.llm_provider = v.to_string();
            }
            "models.llm_model" => {
                cfg.models.llm_model = value
                    .as_str()
                    .ok_or("models.llm_model must be a string")?
                    .to_string();
            }
            "window.bar_x" => cfg.window.bar_x = window_pref_value(&value)?,
            "window.bar_y" => cfg.window.bar_y = window_pref_value(&value)?,
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
    let updated = state.config.lock().clone();
    Ok(updated)
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
            // `Keystore::new` detects Unset (no file) vs Locked from disk.
            let keystore = Keystore::new();
            // Bar only — feature panels are created when the gate opens.
            let pool = WindowPool::create_bar_only(handle)?;
            app.manage(AppState {
                keystore: Mutex::new(keystore),
                config: Mutex::new(cfg),
                db: Arc::new(db),
                ring: Arc::new(Mutex::new(RingBuffer::new(RING_MAX_FRAMES, RING_MAX_BYTES))),
                capture: Mutex::new(None),
                ask: Arc::new(AskService::new()),
                pool: Mutex::new(pool),
                hotkeys: Mutex::new(None),
                gate: Mutex::new(Gate::NeedsUnlock),
                gate_transition: Mutex::new(()),
                click_through: AtomicBool::new(false),
            });
            deeplink::init(handle, deeplink_dispatch(handle))?;
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
            // emits `app:state` either way.
            transition_gate(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            keystore_status,
            keystore_init,
            keystore_unlock,
            keystore_reset,
            keystore_lock,
            keystore_set_key,
            keystore_remove_key,
            model_validate_key,
            model_get_selected,
            model_set_selected,
            model_list_available,
            ask_send,
            ask_close,
            listen_stub,
            window_toggle_all,
            window_show_settings,
            window_hide_settings,
            window_adjust_height,
            permissions_status,
            permissions_request_screen,
            permissions_request_mic,
            permissions_open_prefs,
            capture_status,
            session_list,
            session_get,
            session_delete,
            config_get,
            config_set,
            quit_application,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU32;

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
        // A fresh dir always gates on unlock (no keys.enc → Unset), and
        // `app_gate` short-circuits before touching CoreGraphics.
        assert_eq!(app_gate(&state), Gate::NeedsUnlock);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
