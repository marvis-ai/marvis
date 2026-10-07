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
//! the card can't open and capture simply doesn't exist yet.
//! [`transition_gate`] recomputes the gate after every mutation that
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
mod dictation;
mod hotkey;
mod keystore;
mod listen;
mod llm;
mod menubar;
mod menus;
mod paths;
mod permissions;
mod prompts;
mod screen_read;
mod sherpa_models;
mod storage;
pub mod stt;
mod tray;
pub mod voice_models;
mod voiceprint;
mod windows;

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};
#[cfg(target_os = "macos")]
use tauri_plugin_liquid_glass::LiquidGlassExt;
use tauri_plugin_opener::OpenerExt;

use ask::AskService;
use capture::controller::{CaptureLifecycle, StartDecision, StopDecision};
use capture::{
    primary_display_source, CaptureSource, FrameSource, PickCandidate, PlatformCapture, RingBuffer,
};
use config::Config;
use dictation::{DictationEvent, DictationService};
use hotkey::RegisteredHotkeys;
use keystore::Keystore;
use listen::{ListenEvent, ListenService};
use llm::{make_provider, ProviderKind};
use storage::{Db, Message, Session, Summary, Transcript};
use windows::WindowPool;

/// Frame ring caps from the spec: 120 frames / 64 MB (~60 s horizon).
const RING_MAX_FRAMES: usize = 120;
const RING_MAX_BYTES: usize = 64 * 1024 * 1024;

/// Local Ollama daemon's model list (same host the adapter streams from).
const OLLAMA_TAGS_URL: &str = "http://localhost:11434/api/tags";

/// Which UI state the bar may show. The card and capture exist only in
/// `Main` — the single global hotkey (show/hide) is chrome-level and
/// registered at startup either way. (Onboarding isn't a gate state —
/// it's a visibility overlay on top: `onboarding_done` is one of `Main`'s
/// two preconditions, so the wizard can never share the screen with the
/// bar's live machinery.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Onboarding incomplete OR no screen-recording permission — the
    /// permission card covers both (during onboarding the bar is hidden
    /// anyway, so the label never misleads).
    NeedsPermission,
    /// Fully live: card, capture.
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
    capture: Mutex<Option<PlatformCapture>>,
    /// The settled-screen describer — fed by the capture callback and
    /// started/stopped alongside capture, so the ambient `[vision]`
    /// read only runs while frames actually flow.
    screen_reader: Arc<screen_read::ScreenReader>,
    /// The picker-selected scope reported by `capture:state` (`None` on
    /// the auto primary-display path). Written under the `capture`
    /// lock — it nests inside it, never the reverse.
    capture_target: Mutex<Option<CaptureTarget>>,
    ask: Arc<AskService>,
    listen: Arc<ListenService>,
    dictation: Arc<DictationService>,
    /// Whisper CLI staged beside the executable by `externalBin`.
    bundled_whisper: Option<std::path::PathBuf>,
    pool: Mutex<WindowPool>,
    /// Currently live set — delta-swapped in place by [`swap_hotkeys`]
    /// (shared pairs are never re-registered; macOS refuses duplicates).
    hotkeys: Mutex<Option<RegisteredHotkeys>>,
    gate: Mutex<Gate>,
    /// Serializes `transition_gate` — the gate swap, its side-effects
    /// (enter/leave Main), and the `app:state` emit must be one critical
    /// section or two overlapping transitions can interleave (stale
    /// emit landing last, or hotkeys/capture from a dead gate state).
    /// The other holders — `capture_start`, `toggle_capture`, and the
    /// picker's `Picked` callback — keep their gate check + capture
    /// start/stop atomic against a concurrent leave-transition the same
    /// way. All holders take locks in the order
    /// `gate_transition` → `gate` → `capture` → `ring`
    /// (`capture_target` nests inside `capture`), so they cannot
    /// deadlock.
    gate_transition: Mutex<()>,
    /// Serializes `listen_start`/`dictation_start` end-to-end — the peer
    /// status check, the mic-permission await, and the service start
    /// must be one critical section or two concurrent first-starts can
    /// both see the peer idle and double-open the microphone. Async so
    /// the guard can be held across the permission await while the
    /// command futures stay `Send`. Stops don't take it: dictation's
    /// start/stop epoch already makes a mid-flight start lose safely.
    speech_lifecycle: tokio::sync::Mutex<()>,
    /// Payload of the alert toast currently on screen (`None` when
    /// dismissed). Kept server-side so the toast can re-read it on mount
    /// via `alert_current` — an `alert:show` emit that races the
    /// webview's listener would otherwise be lost.
    alert: Mutex<Option<serde_json::Value>>,
    voice_models: voice_models::VoiceModelManager,
    sherpa_models: sherpa_models::SherpaModelManager,
    /// Owns the voice-enrollment recording lifecycle (mic capture +
    /// voiceprint persistence). Self-contained — no cross-field locking.
    voice_enroll: voiceprint::VoiceEnroll,
}

impl AppState {
    /// The ask pipeline's borrow bundle: `db`/`ring`/`reader` clone
    /// their `Arc`s (the spawned stream outlives the call), the rest are
    /// short-lived `&Mutex` borrows used only in pre-flight.
    /// `capture_running` snapshots the capture slot so `resolve_screen`
    /// knows whether ring frames are fresh.
    fn deps(&self) -> ask::Deps<'_> {
        ask::Deps {
            db: Arc::clone(&self.db),
            ring: Arc::clone(&self.ring),
            reader: Arc::clone(&self.screen_reader),
            capture_running: self
                .capture
                .lock()
                .as_ref()
                .is_some_and(PlatformCapture::is_running),
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

    /// The configured `#rrggbb` accent — the bar's liquid-glass tint
    /// derives from it, so `config_set` re-applies the effect on
    /// `app.accent` writes.
    pub(crate) fn accent(&self) -> String {
        self.config.lock().app.accent.clone()
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
            screen_reader: Arc::new(screen_read::ScreenReader::new()),
            capture_target: Mutex::new(None),
            ask: Arc::new(AskService::new()),
            listen: Arc::new(ListenService::new()),
            dictation: Arc::new(DictationService::new()),
            bundled_whisper: None,
            pool: Mutex::new(WindowPool::new_empty()),
            hotkeys: Mutex::new(None),
            gate: Mutex::new(Gate::NeedsPermission),
            gate_transition: Mutex::new(()),
            speech_lifecycle: tokio::sync::Mutex::new(()),
            alert: Mutex::new(None),
            voice_models: voice_models::VoiceModelManager::at(
                root.join("models").join("whisper").join("models"),
            ),
            sherpa_models: sherpa_models::SherpaModelManager::at(
                root.join("models").join("sherpa").join("models"),
            ),
            voice_enroll: voiceprint::VoiceEnroll::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// App gate
// ---------------------------------------------------------------------------

/// `Main` needs both first-run halves: `app.onboarding_done` AND
/// `permissions::screen_status()`. While the wizard runs the gate stays
/// `NeedsPermission`, so `enter_main` (capture) can't
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
    // `capture_start`, `toggle_capture`, and the picker's `Picked`
    // callback are the other `gate_transition` holders — all follow
    // the field's documented lock order, so this cannot deadlock.
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

/// `Main` entry: start capture through the shared lifecycle boundary —
/// it warns and continues on failure, so a failed piece never wedges
/// the gate. Settings → Recording's `auto_screenshots` opts out of the
/// ambient start; the bar's record toggle stays a manual override.
fn enter_main(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.config.lock().recording.auto_screenshots {
        match primary_display_source() {
            Ok((source, w, h)) => {
                start_capture(app, source, w, h, None);
            }
            Err(e) => log::warn!("capture: display source failed: {e}"),
        }
    } else {
        // Still broadcast — listeners resync on every Main entry.
        let status = capture_snapshot(&state);
        emit_capture_state(app, &status);
    }
}

/// `Main` exit (onboarding reset / permission revoked): cancel the
/// in-flight ask and collapse the card, stop dictation, stop + drop
/// capture, hide the alert toast.
fn leave_main(app: &AppHandle) {
    let state = app.state::<AppState>();
    // Cancel any in-flight ask — an unbounded stream left running would
    // hold `AskState::Streaming` past a leave/re-enter and wedge every
    // future send on the busy-check until it resolves on its own.
    // Listen is independent of card visibility; only `listen_stop` stops it.
    state.ask.close(app, &state.pool);
    // Dictation is bound to the ask input that leaving Main hides — an
    // invisible session must not keep the microphone. (Its epoch also
    // aborts any `dictation_start` still in flight.) Emit the state only
    // when one was live so bar views resync; `dictation_status` covers
    // the rest on mount.
    let dictation_was_live = state.dictation.status().is_listening();
    let _ = state.dictation.stop();
    if dictation_was_live {
        emit_dictation_state(app, &state.dictation.status());
    }
    // Capture teardown shares the command boundary: the capture leaves
    // the mutex before `stop()` joins the worker, then `capture:state`
    // broadcasts the result.
    stop_capture(app);
    // The picker is part of Main too — a gate leave mid-pick would
    // otherwise strand it visible over a hidden bar.
    let pool = state.pool.lock();
    pool.hide_alert();
    pool.hide_picker();
}

/// The picker-selected capture scope shown by `capture:state` —
/// `None` on the auto primary-display path.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureTarget {
    /// "display" | "window" | "app"
    pub kind: &'static str,
    pub label: String,
}

/// `{"running": bool, "frames": ring.len(), "target": CaptureTarget|null}`
/// — the capture status snapshot returned by the `capture_*` commands
/// and carried by `capture:state`. Frame bytes are never serialized.
type CaptureStatus = serde_json::Value;

/// Live `{"running", "frames", "target"}` snapshot — the single
/// serializer for the status contract.
fn capture_snapshot(state: &AppState) -> CaptureStatus {
    let running = state
        .capture
        .lock()
        .as_ref()
        .is_some_and(PlatformCapture::is_running);
    json!({
        "running": running,
        "frames": state.ring.lock().len(),
        "target": state.capture_target.lock().clone(),
    })
}

/// Broadcast `capture:state` to every window — fire-and-forget like
/// `app:state`; a dead webview must never stall a transition. Also the
/// capture label funnel for the tray menu's Start ⇄ Stop item.
fn emit_capture_state(app: &AppHandle, status: &CaptureStatus) {
    let _ = app.emit("capture:state", status);
    refresh_tray_menu(app);
}

/// The one capture-start boundary — `enter_main`, the `capture_start`
/// command, `toggle_capture`, and the picker's start share it (callers
/// own source construction; `target` is the picker-selected scope or
/// `None` on the auto-display path). Idempotent: a live capture
/// returns the current status untouched. Otherwise a fresh
/// [`PlatformCapture`] is built and started with a callback that feeds
/// the screen reader ahead of the ring, then stored only when
/// `is_running()` — a silently-failed start must not block a later
/// retry. Every step warns and continues; `capture:state` carries the
/// result.
fn start_capture(
    app: &AppHandle,
    source: CaptureSource,
    width: u32,
    height: u32,
    target: Option<CaptureTarget>,
) -> CaptureStatus {
    let state = app.state::<AppState>();
    // Config reads precede the capture lock (existing rule).
    let (fps, read_interval_secs) = {
        let cfg = state.config.lock();
        (cfg.recording.fps, cfg.recording.read_interval_secs)
    };
    {
        let mut slot = state.capture.lock();
        // A stored capture counts as running only while `is_running`
        // holds — a stopped leftover falls through to `Create` and is
        // replaced by the fresh capture below.
        let mut lifecycle = CaptureLifecycle::default();
        if slot.as_ref().is_some_and(PlatformCapture::is_running) {
            lifecycle.mark_running();
        }
        if lifecycle.start_decision() == StartDecision::Create {
            match PlatformCapture::new(source, width, height, fps) {
                Ok(capture) => {
                    let ring = Arc::clone(&state.ring);
                    let reader = Arc::clone(&state.screen_reader);
                    capture.start(Box::new(move |frame| {
                        reader.note_frame(frame.clone());
                        ring.lock().push(frame);
                    }));
                    // Store only on success (`is_running` is false when
                    // SCStream rejected the handler or `start_capture`
                    // failed).
                    if capture.is_running() {
                        *slot = Some(capture);
                        *state.capture_target.lock() = target;
                        lifecycle.mark_running();
                    } else {
                        log::warn!("gate: screen capture failed to start");
                    }
                }
                Err(e) => log::warn!("gate: capture init failed: {e}"),
            }
        }
    }
    // Background reader: resolve [vision] per read so provider changes
    // take effect live; a missing vision config just skips reads.
    if state
        .capture
        .lock()
        .as_ref()
        .is_some_and(PlatformCapture::is_running)
    {
        let app2 = app.clone();
        let describe: screen_read::Describer = Arc::new(move |frame| {
            let app = app2.clone();
            Box::pin(async move {
                let state = app.state::<AppState>();
                let vision = {
                    let cfg = state.config.lock();
                    let ks = state.keystore.lock();
                    crate::vision_candidate(&cfg, &ks)
                };
                match vision {
                    Some(v) => crate::screen_read::describe_screen(
                        &*v.provider,
                        &frame,
                        &tokio_util::sync::CancellationToken::new(),
                    )
                    .await
                    .map(|o| o.map(|r| r.full)),
                    // No [vision] provider — skip the read quietly
                    // (`Ok(None)` keeps the cache and never warns).
                    None => Ok(None),
                }
            })
        });
        state
            .screen_reader
            .start(describe, Duration::from_secs(read_interval_secs.max(1)));
    }
    let status = capture_snapshot(&state);
    emit_capture_state(app, &status);
    status
}

/// The one capture-stop boundary — `leave_main`, the `capture_stop`
/// command, and app teardown share it. Idempotent: an empty slot is a
/// no-op. The capture is taken out of the mutex and the lock released
/// BEFORE `stop()` — it joins the capture worker, which must not hold
/// `state.capture` while `capture_status` waits on it — then
/// `capture:state` carries the result.
fn stop_capture(app: &AppHandle) -> CaptureStatus {
    let state = app.state::<AppState>();
    let capture = {
        let mut slot = state.capture.lock();
        // Anything stored is a live session to end; `stop()` itself
        // no-ops when the worker already isn't running.
        let mut lifecycle = CaptureLifecycle::default();
        if slot.is_some() {
            lifecycle.mark_running();
        }
        match lifecycle.stop_decision() {
            StopDecision::Stop => {
                let taken = slot.take();
                lifecycle.mark_stopped();
                taken
            }
            StopDecision::Noop => None,
        }
    };
    if let Some(capture) = capture {
        capture.stop();
    }
    state.screen_reader.stop();
    *state.capture_target.lock() = None;
    let status = capture_snapshot(&state);
    emit_capture_state(app, &status);
    status
}

/// The `menu.capture` item + `toggle_capture` hotkey shared boundary:
/// stop when live, else the `capture_start` command's gate-checked
/// start — the check and `start_capture` share `gate_transition` so a
/// racing leave-transition can't interleave.
fn toggle_capture(app: &AppHandle) {
    let state = app.state::<AppState>();
    let running = state
        .capture
        .lock()
        .as_ref()
        .is_some_and(PlatformCapture::is_running);
    if running {
        stop_capture(app);
        return;
    }
    let _transition = state.gate_transition.lock();
    if *state.gate.lock() != Gate::Main {
        log::warn!("toggle_capture dropped while gate != Main");
        return;
    }
    match primary_display_source() {
        Ok((source, w, h)) => {
            start_capture(app, source, w, h, None);
        }
        Err(e) => log::warn!("capture: display source failed: {e}"),
    }
}

/// The `window.bar_locked` write shared by the Lock menu item and the
/// `toggle_lock` hotkey — config save + `config:changed` broadcast,
/// then the tray menu's check refreshes.
fn set_bar_locked(app: &AppHandle, locked: bool) {
    let state = app.state::<AppState>();
    let updated = {
        let mut cfg = state.config.lock();
        cfg.window.bar_locked = locked;
        if let Err(e) = config::save(&cfg) {
            log::warn!("persist bar_locked failed: {e}");
            return;
        }
        cfg.clone()
    };
    let _ = app.emit("config:changed", &updated);
    refresh_tray_menu(app);
}

/// The menu Position snap path — the same as `window_snap_edge` plus
/// the tray's edge-check refresh.
fn snap_edge_and_refresh(app: &AppHandle, dir: windows::Dir) {
    app.state::<AppState>().pool.lock().snap_edge(dir);
    refresh_tray_menu(app);
}

/// Rebuild the tray's copy of the shared menu AND sync the persistent
/// menubar's Position checks — labels/checks track live state
/// (recording, listening, nearest edge, lock, hotkey bindings), so
/// each state-change funnel calls here. NEVER call while holding
/// `pool`/`config`/`capture`/`hotkeys` locks: `set_menu`/`set_checked`
/// block on the main thread, and main-thread window-event handlers
/// take `pool` (`Resized` → `enforce_bar_bounds`).
fn refresh_tray_menu(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id("main") {
        match menus::build(app) {
            Ok(menu) => {
                if let Err(e) = tray.set_menu(Some(menu)) {
                    log::warn!("tray menu refresh failed: {e}");
                }
            }
            Err(e) => log::warn!("tray menu rebuild failed: {e}"),
        }
    }
    menubar::sync_position_checks(app);
}

/// Delta-swap to the config's current binding set via
/// [`hotkey::swap_hotkey_set`]: pairs shared with the live set stay
/// registered untouched (macOS Carbon refuses duplicate registration of
/// a combo, so a register-everything-then-unregister swap can never
/// succeed). On failure the swap has already rolled back; the returned
/// `restored` set is what the OS still has bound, so it's stored
/// either way.
fn swap_hotkeys(app: &AppHandle) {
    let state = app.state::<AppState>();
    let binds = state.config.lock().hotkeys.clone();
    let dispatch = hotkey_dispatch(app);
    // Hold the guard across the swap: a concurrent swap seeing `None`
    // would compute a full `add` set against still-live OS bindings and
    // wedge every future swap. `state.hotkeys` is locked nowhere else,
    // so holding it here cannot deadlock.
    let mut slot = state.hotkeys.lock();
    let prev = slot.take().unwrap_or_default();
    match hotkey::swap_hotkey_set(app, &binds, dispatch, prev) {
        Ok(set) => {
            *slot = Some(set);
        }
        Err(e) => {
            log::warn!("hotkey swap failed: {e}");
            *slot = Some(e.restored);
        }
    }
    drop(slot);
    // A rebind rewrites the menu items' accelerator labels.
    refresh_tray_menu(app);
}

// ---------------------------------------------------------------------------
// Dispatch closures
// ---------------------------------------------------------------------------

/// Rust→bar event the `toggle_input` hotkey fires. The webview owns the
/// capsule⇄input morph (and the "open card counts as shown" collapse),
/// so dispatch only emits — it never touches `chat_open`.
const EV_BAR_TOGGLE_INPUT: &str = "bar:toggle-input";
/// Rust→bar events the shared menu and the `start_listen` /
/// `show_history` hotkeys fire. The webview owns both surfaces —
/// dispatch only emits.
const EV_BAR_START_LISTEN: &str = "bar:start-listen";
const EV_BAR_SHOW_HISTORY: &str = "bar:show-history";

/// Map [`hotkey::Action`]s onto pool calls / bar events. Owns an
/// `AppHandle` and re-resolves `AppState` per press, so the same
/// closure survives gate transitions.
fn hotkey_dispatch(app: &AppHandle) -> impl Fn(hotkey::Action) + Send + Sync + 'static {
    let app = app.clone();
    move |action| match action {
        hotkey::Action::ToggleInput => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_TOGGLE_INPUT, ());
        }
        hotkey::Action::ToggleCapture => toggle_capture(&app),
        hotkey::Action::StartListen => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_START_LISTEN, ());
        }
        hotkey::Action::ShowHistory => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_SHOW_HISTORY, ());
        }
        hotkey::Action::ToggleLock => {
            let locked = app.state::<AppState>().config.lock().window.bar_locked;
            set_bar_locked(&app, !locked);
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
                state.ask.send(&app, &state.deps(), &text, false, None);
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

/// Map `menu.*` item ids (menus.rs — shared by the tray menu and the
/// bar's right-click popup) onto the same actions the bar buttons,
/// commands, and hotkeys use. Registered once via `app.on_menu_event`
/// in `setup`: menu events broadcast to EVERY listener, so this must
/// be the only handler matching `menu.*` ids.
fn menu_dispatch() -> impl Fn(&AppHandle, tauri::menu::MenuEvent) + Send + Sync + 'static {
    move |app, event| {
        let id: &str = event.id().as_ref();
        // The four `menu.pos.*` edge ids share one table with the menu
        // builders (menus::POS_EDGES) — looked up here rather than
        // matched, so an edge can't drift from its `Dir`.
        if let Some(dir) = menus::pos_edge_dir(id) {
            snap_edge_and_refresh(app, dir);
            return;
        }
        match id {
            menus::MENU_ASK => {
                let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_TOGGLE_INPUT, ());
            }
            menus::MENU_CAPTURE => toggle_capture(app),
            menus::MENU_LISTEN => {
                let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_START_LISTEN, ());
            }
            menus::MENU_HISTORY => {
                let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_SHOW_HISTORY, ());
            }
            menus::MENU_POS_CENTER => {
                app.state::<AppState>().pool.lock().recenter_bar();
                refresh_tray_menu(app);
            }
            menus::MENU_LOCK => {
                let locked = app.state::<AppState>().config.lock().window.bar_locked;
                set_bar_locked(app, !locked);
            }
            menus::MENU_SETTINGS => show_settings(app),
            menus::MENU_SUPPORT => {
                if let Err(e) = app
                    .opener()
                    .open_url(menubar::SUPPORT_MAILTO, None::<&str>)
                {
                    log::warn!("open support mailto failed: {e}");
                }
            }
            // The item is built only in debug builds (menus.rs), so this
            // arm compiles out in release — `menu.devtools` can't fire
            // there anyway. Menu events carry no window identity: the bar
            // owns the only popup, so it's the target (the tray copy
            // inspects the bar too).
            #[cfg(debug_assertions)]
            menus::MENU_DEVTOOLS => {
                if let Some(bar) = app.get_webview_window(windows::BAR_LABEL) {
                    bar.open_devtools();
                }
            }
            menus::MENU_QUIT => app.exit(0),
            _ => {}
        }
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

/// Deepgram is an STT-only key and must never enter the LLM provider
/// catalog. Key commands accept it alongside the LLM provider ids.
fn is_key_management_provider(provider: &str) -> bool {
    provider == "deepgram" || ProviderKind::from_str(provider).is_some()
}

const DEEPGRAM_UNVERIFIED_MESSAGE: &str =
    "Deepgram key accepted after non-empty shape validation; no live provider probe was performed";

fn normalize_deepgram_key(key: &str) -> Result<String, String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        Err("Deepgram API key must not be empty after trimming".to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn deepgram_validation_payload(key: &str) -> serde_json::Value {
    match normalize_deepgram_key(key) {
        Ok(_) => json!({ "ok": true, "message": DEEPGRAM_UNVERIFIED_MESSAGE }),
        Err(error) => json!({ "ok": false, "error": error }),
    }
}

/// Raise the alert toast with `message`.
///
/// The toast is a window of its own because the bar is a fixed-height
/// capsule (172⇄600 wide) — the old inline error row squeezed the
/// pill's content. It is purely informational and auto-dismisses.
fn show_alert(app: &AppHandle, message: &str) {
    let state = app.state::<AppState>();
    let payload = json!({ "message": message });
    *state.alert.lock() = Some(payload.clone());
    let _ = app.emit_to(windows::ALERT_LABEL, "alert:show", payload);
    state.pool.lock().show_alert();
}

/// Surface the prefs window in settings mode. Works at ANY gate —
/// the tray item must respond even mid-onboarding (the bar's `Cmd+,`
/// is a webview key, so it can't fire while the bar is hidden).
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
    // The bar's `Moved` handler registers before `app.manage(AppState)`
    // — a debounced write that fires inside that gap must not panic on
    // the missing state (the next move persists anyway).
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    // The idle capsule rect, not the live one: a drag while the bar is
    // expanded (600) must persist the capsule's anchor, else relaunch
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
    // A settled drag may have changed the nearest edge — the Position
    // submenu's check tracks it.
    refresh_tray_menu(app);
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

/// Store a key and broadcast `keystore:changed`. Normal LLM provider keys
/// are live-validated before storage; Deepgram keys are trimmed and stored
/// after non-empty shape validation only, without a live provider probe.
#[tauri::command]
async fn keystore_set_key(
    app: AppHandle,
    provider: String,
    key: String,
) -> Result<serde_json::Value, String> {
    if !is_key_management_provider(&provider) {
        return Err(format!("unknown provider {provider:?}"));
    }
    let key = if provider == "deepgram" {
        normalize_deepgram_key(&key)?
    } else {
        let kind = ProviderKind::from_str(&provider).expect("validated provider id");
        let (model, base_url) = {
            let state = app.state::<AppState>();
            provider_args(&state, kind)
        };
        make_provider(kind, Some(key.clone()), model, base_url)
            .validate()
            .await
            .map_err(|e| e.to_string())?;
        key
    };
    let state = app.state::<AppState>();
    let payload = {
        let mut ks = state.keystore.lock();
        ks.set_key(&provider, &key).map_err(|e| e.to_string())?;
        emit_keystore_changed(&app, &ks);
        keystore_status_payload(&ks)
    };
    if provider == "deepgram" {
        refresh_speech_setup(&app);
    }
    Ok(payload)
}

/// Remove a provider key and broadcast `keystore:changed`.
#[tauri::command]
fn keystore_remove_key(app: AppHandle, provider: String) -> Result<serde_json::Value, String> {
    if !is_key_management_provider(&provider) {
        return Err(format!("unknown provider {provider:?}"));
    }
    let state = app.state::<AppState>();
    let payload = {
        let mut ks = state.keystore.lock();
        ks.remove_key(&provider).map_err(|e| e.to_string())?;
        emit_keystore_changed(&app, &ks);
        keystore_status_payload(&ks)
    };
    if provider == "deepgram" {
        refresh_speech_setup(&app);
    }
    Ok(payload)
}

// ---------------------------------------------------------------------------
// Commands — models
// ---------------------------------------------------------------------------

/// Test a candidate key WITHOUT storing it: `{"ok": true}` or
/// `{"ok": false, "error": "..."}` — validation failures are data, not
/// command errors, so the UI can render them inline. Normal LLM keys are
/// live-probed; Deepgram only gets non-empty shape validation and reports
/// that no live provider probe was performed.
#[tauri::command]
async fn model_validate_key(app: AppHandle, provider: String, key: String) -> serde_json::Value {
    if provider == "deepgram" {
        return deepgram_validation_payload(&key);
    }
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
/// the `bar` window as `ask:*` events on a spawned task. `withScreen`
/// (optional) is the explicit attach flag — a screen read runs even when
/// the text shows no intent. `listenId` (optional) binds the send to a
/// listen doc — its own ask session, its summary+transcript as context.
#[tauri::command]
fn ask_send(app: AppHandle, text: String, with_screen: Option<bool>, listen_id: Option<i64>) {
    let state = app.state::<AppState>();
    // Crafted-invoke guard: the shipped UI gates sends behind `Main`,
    // but a crafted invoke during onboarding would otherwise proceed —
    // DB writes, network, and emits to a window that doesn't exist yet.
    if *state.gate.lock() != Gate::Main {
        log::warn!("ask_send dropped while gate != Main");
        return;
    }
    state.ask.send(
        &app,
        &state.deps(),
        &text,
        with_screen.unwrap_or(false),
        listen_id,
    );
}

/// Regenerate the last answer — re-runs the active session's last user
/// turn (ask.rs `AskService::retry`). Same gate guard as `ask_send`.
#[tauri::command]
fn ask_retry(app: AppHandle) {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("ask_retry dropped while gate != Main");
        return;
    }
    state.ask.retry(&app, &state.deps());
}

/// Cancel the in-flight stream and collapse the card.
#[tauri::command]
fn ask_close(app: AppHandle) {
    let state = app.state::<AppState>();
    state.ask.close(&app, &state.pool);
}

/// The bar's camera affordance — a screen-only ask (fixed prompt,
/// frame required). Same gate guard as `ask_send`.
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

const EV_LISTEN_STATE: &str = "listen:state";
const EV_LISTEN_TURN: &str = "listen:turn";
const EV_LISTEN_SUMMARY: &str = "listen:summary";
const EV_LISTEN_ERROR: &str = "listen:error";

fn emit_listen_state(app: &AppHandle, state: &listen::ListenStatus) {
    let _ = app.emit_to(
        windows::BAR_LABEL,
        EV_LISTEN_STATE,
        json!({
            "state": state.state,
            "provider": state.provider,
            "session_id": state.session_id,
            "mic": state.mic,
            "error": state.error,
            "started_at": state.started_at,
            "paused_secs": state.paused_secs,
            "paused_since": state.paused_since,
        }),
    );
    // The shared menu's Start Listening item is disabled while live.
    refresh_tray_menu(app);
}

fn emit_listen_event(app: &AppHandle, event: ListenEvent) {
    match event {
        ListenEvent::Turn(turn) => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_LISTEN_TURN, turn);
        }
        ListenEvent::Summary(summary) => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_LISTEN_SUMMARY, summary);
        }
        ListenEvent::Error {
            message,
            needs_setup,
        } => {
            let _ = app.emit_to(
                windows::BAR_LABEL,
                EV_LISTEN_ERROR,
                json!({ "message": message, "needs_setup": needs_setup }),
            );
            let status = app.state::<AppState>().listen.status();
            // Re-emit the durable status snapshot so a bar opened after the
            // setup failure can resynchronize through the same payload as a
            // normal listen state update.
            emit_listen_state(app, &status);
        }
    }
}

#[tauri::command]
async fn listen_start(app: AppHandle) -> Result<listen::ListenStatus, String> {
    let state = app.state::<AppState>();
    // Serialized with `dictation_start` end-to-end: the peer-status
    // check, the mic-permission await, and the service start are one
    // critical section — two concurrent first-starts can no longer both
    // see the peer idle and double-open the microphone.
    let _lifecycle = state.speech_lifecycle.lock().await;
    if *state.gate.lock() != Gate::Main {
        return Err("Listen is unavailable until setup is complete".into());
    }
    // Mutual exclusion: a live dictation session owns the microphone —
    // a stale/racing `listen_start` must fail instead of stealing it.
    if state.dictation.status().is_listening() {
        return Err("Stop dictation before listening".into());
    }
    let mic_allowed = tauri::async_runtime::spawn_blocking(permissions::mic_request)
        .await
        .unwrap_or(false);
    let config = state.config.lock().clone();
    let keystore = state.keystore.lock().clone();
    let app_for_emit = app.clone();
    let emit = Arc::new(move |event| emit_listen_event(&app_for_emit, event));
    if let Err(error) = state.listen.start(
        Arc::clone(&state.db),
        &keystore,
        &config,
        mic_allowed,
        state.bundled_whisper.as_deref(),
        emit,
    ) {
        // ListenService owns the event contract for failures it emits. In
        // particular, setup failures have already emitted needs_setup:true;
        // re-emitting here would produce a contradictory second event.
        return Err(error.to_string());
    }
    let status = state.listen.status();
    emit_listen_state(&app, &status);
    Ok(status)
}

#[tauri::command]
fn listen_stop(app: AppHandle) {
    let state = app.state::<AppState>();
    state.listen.stop();
    emit_listen_state(&app, &state.listen.status());
}

#[tauri::command]
fn listen_pause(app: AppHandle) {
    let state = app.state::<AppState>();
    if let Some(status) = state.listen.pause() {
        emit_listen_state(&app, &status);
    }
}

#[tauri::command]
fn listen_resume(app: AppHandle) {
    let state = app.state::<AppState>();
    if let Some(status) = state.listen.resume() {
        emit_listen_state(&app, &status);
    }
}

#[tauri::command]
fn listen_status(app: AppHandle) -> listen::ListenStatus {
    // Self-healing read: a cause fixed outside the tracked triggers (e.g.
    // an externally installed whisper-cli) clears on the next resync.
    refresh_speech_setup(&app);
    app.state::<AppState>().listen.status()
}

const EV_DICTATION_STATE: &str = "dictation:state";
const EV_DICTATION_DRAFT: &str = "dictation:draft";
const EV_DICTATION_ERROR: &str = "dictation:error";

/// `dictation:state` carries the whole durable status — `DictationStatus`'s
/// wire fields (`state`/`provider`/`error`) already are the payload.
fn emit_dictation_state(app: &AppHandle, status: &dictation::DictationStatus) {
    let _ = app.emit_to(windows::BAR_LABEL, EV_DICTATION_STATE, status);
}

fn emit_dictation_event(app: &AppHandle, event: DictationEvent) {
    match event {
        DictationEvent::Draft(draft) => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_DICTATION_DRAFT, draft);
        }
        DictationEvent::Error {
            message,
            needs_setup,
        } => {
            let _ = app.emit_to(
                windows::BAR_LABEL,
                EV_DICTATION_ERROR,
                json!({ "message": message, "needs_setup": needs_setup }),
            );
            // Re-emit the durable status snapshot so a bar that missed the
            // failure resynchronizes — same contract as `listen:error`.
            let status = app.state::<AppState>().dictation.status();
            emit_dictation_state(app, &status);
        }
    }
}

/// A durable setup error should live exactly as long as its cause. STT
/// config writes, Deepgram key changes, a mic grant, and completed model
/// downloads each re-run the speech services' pre-flight checks through
/// here — a resolved error resets to `idle` and a still-broken one
/// rewrites to the current reason, both via the usual `*:state` resync,
/// so no webview keeps showing a problem the user already fixed.
pub(crate) fn refresh_speech_setup(app: &AppHandle) {
    let state = app.state::<AppState>();
    let config = state.config.lock().clone();
    let keystore = state.keystore.lock().clone();
    let bundled = state.bundled_whisper.as_deref();
    let sherpa_root = paths::sherpa_models_dir();
    state
        .sherpa_models
        .ensure_punct(&config.models.stt_provider, &config.models.stt_model);
    if let Some(status) = state
        .listen
        .revalidate_setup(&keystore, &config, bundled, &sherpa_root)
    {
        emit_listen_state(app, &status);
    }
    let mic_allowed = permissions::mic_status() == permissions::PermissionState::Authorized;
    if let Some(status) =
        state
            .dictation
            .revalidate_setup(&keystore, &config, bundled, &sherpa_root, mic_allowed)
    {
        emit_dictation_state(app, &status);
    }
}

/// Mic-only dictation into the Ask input: requests mic permission, never
/// opens `SystemAudioSource`, and persists nothing. Mutually exclusive with
/// meeting Listen — the mic button normally stops the other mode first, so
/// a conflict here means a stale/racing invoke and must fail safely.
#[tauri::command]
async fn dictation_start(app: AppHandle) -> Result<dictation::DictationStatus, String> {
    let state = app.state::<AppState>();
    // Serialized with `listen_start` end-to-end: the peer-status check,
    // the mic-permission await, and the service start are one critical
    // section — two concurrent first-starts can no longer both see the
    // peer idle and double-open the microphone.
    let _lifecycle = state.speech_lifecycle.lock().await;
    if *state.gate.lock() != Gate::Main {
        return Err("Dictation is unavailable until setup is complete".into());
    }
    if state.listen.status().is_listening() {
        return Err("Stop listening before dictating".into());
    }
    let mic_allowed = tauri::async_runtime::spawn_blocking(permissions::mic_request)
        .await
        .unwrap_or(false);
    let config = state.config.lock().clone();
    let keystore = state.keystore.lock().clone();
    let app_for_emit = app.clone();
    let emit = Arc::new(move |event| emit_dictation_event(&app_for_emit, event));
    if let Err(error) = state.dictation.start(
        &keystore,
        &config,
        mic_allowed,
        state.bundled_whisper.as_deref(),
        emit,
    ) {
        // DictationService owns the event contract for failures it emits:
        // setup failures have already emitted needs_setup:true, so
        // re-emitting here would produce a contradictory second event.
        return Err(error.to_string());
    }
    // `leave_main` may have run while the session was being built —
    // dictation is bound to the (now hidden) ask input, so a start that
    // outlived `Main` stops itself instead of running invisibly.
    if *state.gate.lock() != Gate::Main {
        let _ = state.dictation.stop();
        emit_dictation_state(&app, &state.dictation.status());
        return Err("Dictation is unavailable until setup is complete".into());
    }
    let status = state.dictation.status();
    emit_dictation_state(&app, &status);
    Ok(status)
}

/// Stop dictation and return the authoritative final draft. Idempotent —
/// repeated calls return an empty final draft and leave the input alone.
#[tauri::command]
fn dictation_stop(app: AppHandle) -> dictation::DictationDraft {
    let state = app.state::<AppState>();
    let draft = state.dictation.stop();
    emit_dictation_state(&app, &state.dictation.status());
    draft
}

/// Live dictation status for bar resync (`idle` | `listening` | `error`).
#[tauri::command]
fn dictation_status(app: AppHandle) -> dictation::DictationStatus {
    refresh_speech_setup(&app);
    app.state::<AppState>().dictation.status()
}

#[tauri::command]
fn voice_models_catalog(state: State<'_, AppState>) -> Vec<voice_models::VoiceModelCatalogPayload> {
    state.voice_models.catalog_payload()
}

#[tauri::command]
fn whisper_status(app: AppHandle) -> voice_models::WhisperDownloadStatus {
    let state = app.state::<AppState>();
    state.voice_models.status(state.bundled_whisper.as_deref())
}

fn safe_voice_error(error: voice_models::VoiceDownloadError) -> String {
    match error {
        voice_models::VoiceDownloadError::Busy => "A voice model download is already active".into(),
        voice_models::VoiceDownloadError::UnknownModel(_) => "Unknown voice model".into(),
        voice_models::VoiceDownloadError::ActiveModel => {
            "Cannot remove the selected voice model".into()
        }
        voice_models::VoiceDownloadError::Cancelled => "Download cancelled".into(),
        voice_models::VoiceDownloadError::Verification => {
            "Downloaded model verification failed".into()
        }
        voice_models::VoiceDownloadError::Download(_) => "Voice model download failed".into(),
    }
}

#[tauri::command]
fn whisper_download(state: State<'_, AppState>, model: String) -> Result<(), String> {
    let entry =
        voice_models::entry_for_id(&model).ok_or_else(|| "Unknown voice model".to_string())?;
    state
        .voice_models
        .start_download(entry.id)
        .map_err(safe_voice_error)
}

#[tauri::command]
async fn whisper_cancel_download(state: State<'_, AppState>) -> Result<(), String> {
    state
        .voice_models
        .cancel_download()
        .await
        .map_err(safe_voice_error)
}

#[tauri::command]
fn whisper_remove_model(
    state: State<'_, AppState>,
    model: String,
) -> Result<voice_models::WhisperDownloadStatus, String> {
    let entry =
        voice_models::entry_for_id(&model).ok_or_else(|| "Unknown voice model".to_string())?;
    let selected = {
        let config = state.config.lock();
        if config.models.stt_provider == "whisper" {
            voice_models::entry_for_id(&config.models.stt_model).map(|entry| entry.id)
        } else {
            None
        }
    };
    state.voice_models.set_selected_model(selected);
    state
        .voice_models
        .remove_model(entry.id)
        .map_err(safe_voice_error)?;
    Ok(state.voice_models.status(state.bundled_whisper.as_deref()))
}

#[tauri::command]
fn sherpa_status(state: State<'_, AppState>) -> sherpa_models::SherpaStatus {
    state.sherpa_models.status()
}

#[tauri::command]
fn sherpa_download(state: State<'_, AppState>, model: String) -> Result<(), String> {
    let entry =
        sherpa_models::entry_for_id(&model).ok_or_else(|| "Unknown voice model".to_string())?;
    state
        .sherpa_models
        .start_download(entry.id)
        .map_err(safe_voice_error)
}

#[tauri::command]
async fn sherpa_cancel_download(state: State<'_, AppState>) -> Result<(), String> {
    state
        .sherpa_models
        .cancel_download()
        .await
        .map_err(safe_voice_error)
}

#[tauri::command]
fn sherpa_remove_model(
    state: State<'_, AppState>,
    model: String,
) -> Result<sherpa_models::SherpaStatus, String> {
    let entry =
        sherpa_models::entry_for_id(&model).ok_or_else(|| "Unknown voice model".to_string())?;
    let selected = {
        let config = state.config.lock();
        if config.models.stt_provider == "sherpa" {
            sherpa_models::stt_entry_for_value(&config.models.stt_model).map(|e| e.id)
        } else {
            None
        }
    };
    state.sherpa_models.set_selected_model(selected);
    state
        .sherpa_models
        .remove_model(entry.id)
        .map_err(safe_voice_error)?;
    Ok(state.sherpa_models.status())
}

// ---------------------------------------------------------------------------
// Commands — voice enrollment
// ---------------------------------------------------------------------------

#[tauri::command]
fn voiceprint_status(state: State<'_, AppState>) -> voiceprint::VoiceprintStatus {
    state.voice_enroll.status()
}

#[tauri::command]
fn voice_enroll_start(state: State<'_, AppState>) -> Result<(), String> {
    state.voice_enroll.start()
}

/// Stop returns the saved take's seconds so the UI can confirm "Ns saved".
#[tauri::command]
fn voice_enroll_stop(state: State<'_, AppState>) -> Result<voiceprint::VoiceEnrollResult, String> {
    state.voice_enroll.stop()
}

#[tauri::command]
fn voice_enroll_cancel(state: State<'_, AppState>) {
    state.voice_enroll.cancel();
}

#[tauri::command]
fn voiceprint_remove(state: State<'_, AppState>) -> Result<(), String> {
    state.voice_enroll.remove()
}

// ---------------------------------------------------------------------------
// Commands — windows
// ---------------------------------------------------------------------------

/// Tray Toggle's behaviour as a command: collapse/expand the unified
/// card. (The global hotkey no longer routes here — it toggles only the
/// bar's input pill via `bar:toggle-input`.)
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

/// Focus the bar window — the `bar:toggle-input` show path needs it so
/// a global-hotkey reveal lands the user's typing in the field.
#[tauri::command]
fn window_focus_bar(app: AppHandle) {
    let state = app.state::<AppState>();
    // Clone the handle out so the pool guard drops before `set_focus`.
    let bar = state.pool.lock().bar().cloned();
    if let Some(bar) = bar {
        let _ = bar.set_focus();
    }
}

/// Same entry point as the bar's `Cmd+,` and the tray's Settings item.
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
/// capsule IS the window, so the window resizes to match (idle 172,
/// expanded 600, same 64 height and capsule radius).
#[tauri::command]
fn window_set_bar_expanded(state: State<'_, AppState>, expanded: bool) {
    state.pool.lock().set_bar_expanded(expanded);
}

/// Settings → Bar picker: `edge` is `"top"|"bottom"|"left"|"right"`.
/// Snaps (animated) the bar to that work-area edge; the resulting `Moved`
/// event persists `window.bar_x/y` through the debounced write.
#[tauri::command]
fn window_snap_edge(app: AppHandle, edge: String) -> Result<(), String> {
    let dir = match edge.as_str() {
        "top" => windows::Dir::Up,
        "bottom" => windows::Dir::Down,
        "left" => windows::Dir::Left,
        "right" => windows::Dir::Right,
        _ => return Err(format!("unknown edge {edge:?}")),
    };
    snap_edge_and_refresh(&app, dir);
    Ok(())
}

/// Settings → Bar "Re-center": restores the default position —
/// the middle of the primary work area.
/// Persists through the same `Moved` debounce as a drag.
#[tauri::command]
fn window_recenter(app: AppHandle) {
    app.state::<AppState>().pool.lock().recenter_bar();
    refresh_tray_menu(&app);
}

/// The edge the bar is currently nearest (`"top"` | `"bottom"` |
/// `"left"` | `"right"`) — the Bar picker's selected value. Recomputed
/// from the live rect so a just-finished drag reads correctly.
#[tauri::command]
fn window_bar_edge(state: State<'_, AppState>) -> String {
    match state.pool.lock().live_bar_edge() {
        windows::Dir::Up => "top",
        windows::Dir::Down => "bottom",
        windows::Dir::Left => "left",
        windows::Dir::Right => "right",
    }
    .to_string()
}

/// The bar webview's right-click entry point: pops the shared menu
/// (menus.rs) under the cursor, built fresh so labels/checks reflect
/// live state. The webview gates this to the idle capsule; the same
/// menu hangs off the tray icon.
#[tauri::command]
fn bar_context_menu(app: AppHandle) -> Result<(), String> {
    let menu = menus::build(&app).map_err(|e| e.to_string())?;
    let bar = app
        .get_webview_window(windows::BAR_LABEL)
        .ok_or("bar window missing")?;
    bar.popup_menu(&menu).map_err(|e| e.to_string())
}

/// Dev-only inspector for webviews without the shared menu (prefs,
/// alert): opens the CALLING window's devtools — their right-click
/// invokes this under `import.meta.env.DEV`. No-op in release, where
/// the frontend never calls it anyway.
#[tauri::command]
#[allow(unused_variables)]
fn open_devtools(webview: tauri::WebviewWindow) {
    #[cfg(debug_assertions)]
    webview.open_devtools();
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
async fn permissions_request_mic(app: AppHandle) -> bool {
    let granted = tauri::async_runtime::spawn_blocking(permissions::mic_request)
        .await
        .unwrap_or(false);
    if granted {
        refresh_speech_setup(&app);
    }
    granted
}

/// `section` is the full privacy pane name (`Privacy_ScreenCapture`,
/// `Privacy_Microphone`, …). Fire-and-forget `open`.
#[tauri::command]
fn permissions_open_prefs(section: String) -> Result<(), String> {
    permissions::open_prefs(&section).map_err(|e| e.to_string())
}

/// `{"running": bool, "frames": ring.len(), "target": CaptureTarget|null}`
/// — read-only; no emit.
#[tauri::command]
fn capture_status(state: State<'_, AppState>) -> serde_json::Value {
    capture_snapshot(&state)
}

/// Idempotent capture start — the same boundary `enter_main` uses.
/// Emits `capture:state`, then resolves to `{"running", "frames", "target"}`.
#[tauri::command]
fn capture_start(app: AppHandle) -> serde_json::Value {
    let state = app.state::<AppState>();
    // Crafted-invoke guard: the shipped UI disables the toggle outside
    // `Main`, but a crafted invoke during onboarding would otherwise
    // light the recorder while capture doesn't exist yet. The command
    // still resolves the (unchanged) status — the toggle resyncs off
    // `running` either way.
    //
    // The check and the start share `gate_transition` with
    // `transition_gate`: a bare gate read could pass just before a
    // transition swaps the gate and `leave_main` tears capture down,
    // letting this start relight capture outside `Main`.
    let _transition = state.gate_transition.lock();
    if *state.gate.lock() != Gate::Main {
        log::warn!("capture_start dropped while gate != Main");
        return capture_snapshot(&state);
    }
    match primary_display_source() {
        Ok((source, w, h)) => start_capture(&app, source, w, h, None),
        Err(e) => {
            log::warn!("capture: display source failed: {e}");
            capture_snapshot(&state)
        }
    }
}

/// Idempotent capture stop — the same boundary `leave_main` and app
/// teardown use. Deliberately ungated: stopping must always be safe.
/// Emits `capture:state`, then resolves to `{"running", "frames", "target"}`.
#[tauri::command]
fn capture_stop(app: AppHandle) -> serde_json::Value {
    stop_capture(&app)
}

/// Idle-state record button: the native macOS content-sharing picker
/// (window / display / application) — same UI Zoom shows. Marvis's own
/// bundle id is excluded so it can never offer itself. Cancel is a
/// silent no-op; `capture:state` reports the picked `target`.
///
/// The picker is a main-thread API (its config setters require it), so
/// the command hops via `run_on_main_thread`; `show` is non-blocking
/// and its `Send` callback fires later with the outcome. On other OSes
/// the in-app picker window (Windows) or the portal dialog (Linux)
/// already covers the flow, so this alias just forwards there.
#[cfg(target_os = "macos")]
#[tauri::command]
fn capture_pick_and_start(app: AppHandle) {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("capture_pick_and_start dropped while gate != Main");
        return;
    }
    let app2 = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        use screencapturekit::content_sharing_picker::*;
        let Some(mut cfg) = SCContentSharingPickerConfiguration::try_new() else {
            log::warn!("capture_pick_and_start: picker unavailable (macOS < 14)");
            return;
        };
        cfg.set_allowed_picker_modes(&[
            SCContentSharingPickerMode::SingleWindow,
            SCContentSharingPickerMode::SingleDisplay,
            SCContentSharingPickerMode::SingleApplication,
        ]);
        cfg.set_excluded_bundle_ids(&["com.getmarvis.marvis"]);
        SCContentSharingPicker::show(&cfg, move |outcome| {
            match outcome {
                SCPickerOutcome::Picked(result) => {
                    // The picker's own filter carries no self-exclusion,
                    // and Marvis's windows are no longer content-protected
                    // — a display pick would composite our bar into the
                    // model's recording. Re-resolve display picks through
                    // `resolve_candidate` (the `d:` path rebuilds the
                    // display filter minus our own windows). Window/app
                    // picks can't be ours (`excluded_bundle_ids`), so
                    // their filters pass through untouched.
                    let (filter, w, h, target) = match result.source() {
                        SCPickedSource::Display(id) => {
                            match capture::resolve_candidate(&format!("d:{id}")) {
                                Ok(res) => (
                                    res.source,
                                    res.w,
                                    res.h,
                                    CaptureTarget {
                                        kind: res.kind,
                                        label: res.label,
                                    },
                                ),
                                // Display vanished between pick and
                                // resolve — degrade to the picker's own
                                // filter rather than dropping the user's
                                // selection entirely.
                                Err(e) => {
                                    log::warn!(
                                        "capture_pick_and_start: display {id} re-resolve failed: {e}"
                                    );
                                    let (w, h) = result.pixel_size();
                                    (
                                        result.filter(),
                                        w,
                                        h,
                                        CaptureTarget {
                                            kind: "display",
                                            label: format!("Display {id}"),
                                        },
                                    )
                                }
                            }
                        }
                        other => {
                            let (w, h) = result.pixel_size();
                            let target = match other {
                                SCPickedSource::Window(t) => CaptureTarget {
                                    kind: "window",
                                    label: t,
                                },
                                SCPickedSource::Application(n) => CaptureTarget {
                                    kind: "app",
                                    label: n,
                                },
                                _ => CaptureTarget {
                                    kind: "app",
                                    label: "Screen".into(),
                                },
                            };
                            (result.filter(), w, h, target)
                        }
                    };
                    // The pick can land after a leave-Main transition ran
                    // `stop_capture` — the invoke-time check can't see it.
                    // Re-check under `gate_transition`, the same critical
                    // section `capture_start` uses (lock order
                    // `gate_transition` → `gate` → `capture`). try_lock:
                    // this callback runs on the main thread, and a worker
                    // holding `gate_transition` blocks on it via
                    // `tray.set_menu` — parking here is an ABBA deadlock.
                    // Contention means a transition is in flight, so
                    // dropping the pick is correct.
                    let state2 = app2.state::<AppState>();
                    let Some(_transition) = state2.gate_transition.try_lock() else {
                        log::warn!(
                            "capture_pick_and_start: pick dropped — gate transition in flight"
                        );
                        return;
                    };
                    if *state2.gate.lock() != Gate::Main {
                        log::warn!("capture_pick_and_start: pick landed after gate left Main");
                        return;
                    }
                    // `start_capture` is idempotent while a capture lives —
                    // a pick made over a running session must retarget, so
                    // stop it first rather than silently keep the old scope.
                    if state2
                        .capture
                        .lock()
                        .as_ref()
                        .is_some_and(PlatformCapture::is_running)
                    {
                        stop_capture(&app2);
                    }
                    start_capture(&app2, filter, w, h, Some(target));
                }
                SCPickerOutcome::Error(e) => {
                    log::warn!("capture_pick_and_start: picker error: {e}");
                }
                SCPickerOutcome::Cancelled => {}
            }
        });
    }) {
        log::warn!("capture_pick_and_start: main-thread hop failed: {e}");
    }
}

/// Non-macOS alias: the record button's "pick and start" is the same
/// custom-picker flow (Windows) or portal dialog (Linux) — whichever
/// `capture_pick_begin` drives on this OS.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn capture_pick_and_start(app: AppHandle) {
    capture_pick_begin(app);
}

/// Linux pick flow, run on a worker thread spawned by
/// `capture_pick_begin`: the XDG screencast portal's own dialog IS the
/// picker (Linux exposes no source enumeration to apps), so the command
/// returns immediately and the pick lands here — same post-pick path
/// the macOS native picker callback takes: gate re-check under
/// `gate_transition`, stop a live capture, start the picked source.
/// The bar comes back in every outcome.
#[cfg(target_os = "linux")]
fn portal_pick_flow(app: AppHandle) {
    match capture::portal_pick_blocking() {
        Ok(Some((source, w, h, kind, label))) => {
            let state = app.state::<AppState>();
            let _transition = state.gate_transition.lock();
            if *state.gate.lock() != Gate::Main {
                log::warn!("capture_pick_begin: portal pick landed after gate left Main");
            } else {
                if state
                    .capture
                    .lock()
                    .as_ref()
                    .is_some_and(PlatformCapture::is_running)
                {
                    stop_capture(&app);
                }
                start_capture(&app, source, w, h, Some(CaptureTarget { kind, label }));
            }
        }
        Ok(None) => {} // user cancelled the portal dialog — silent no-op
        Err(e) => log::warn!("capture: portal pick failed: {e}"),
    }
    if let Some(bar) = app.state::<AppState>().pool.lock().bar() {
        let _ = bar.show();
    }
}

/// Idle record button: hide the bar and open the share-picker window.
/// Gate-guarded like `capture_start` — a crafted invoke outside Main
/// must not surface the picker (or a capture behind it). The bar is
/// re-shown by `capture_pick_select`/`capture_pick_cancel`. On Linux
/// the portal's native dialog replaces the picker window entirely.
#[tauri::command]
fn capture_pick_begin(app: AppHandle) {
    let state = app.state::<AppState>();
    let _transition = state.gate_transition.lock();
    if *state.gate.lock() != Gate::Main {
        log::warn!("capture_pick_begin dropped while gate != Main");
        return;
    }
    #[cfg(target_os = "linux")]
    {
        // Hide the bar under the portal dialog, then let a worker own
        // the (user-paced) pick — the command can't block on it.
        if let Some(bar) = state.pool.lock().bar() {
            let _ = bar.hide();
        }
        drop(_transition);
        let app2 = app.clone();
        std::thread::spawn(move || portal_pick_flow(app2));
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut pool = state.pool.lock();
        if let Some(bar) = pool.bar() {
            let _ = bar.hide();
        }
        if !pool.show_picker(&app) {
            // A failed build can't be cancelled from a picker that never
            // opened — restore the bar so the UI isn't left hidden.
            if let Some(bar) = pool.bar() {
                let _ = bar.show();
            }
        }
    }
}

/// The picker's candidate list — meta only, returned fast; a detached
/// blocking task then thumbs each candidate and emits `picker:thumb`
/// to the picker window (SCK calls must not run on the async
/// executor; `capture_sample_buffer` is a sync Cocoa call).
/// App cards reuse their largest window's thumb — the map fills as
/// windows emit, so apps need no extra capture. Gate-guarded: a
/// crafted invoke outside Main could otherwise enumerate window
/// titles.
#[cfg(not(target_os = "linux"))]
#[tauri::command]
fn capture_pick_list(app: AppHandle) -> Result<Vec<PickCandidate>, String> {
    let state = app.state::<AppState>();
    let _transition = state.gate_transition.lock();
    if *state.gate.lock() != Gate::Main {
        return Err("picker is only available in the main window".into());
    }
    let metas = capture::pick_candidates().map_err(|e| e.to_string())?;
    let app2 = app.clone();
    let list = metas.clone();
    tauri::async_runtime::spawn(async move {
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let mut thumbs: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for m in &list {
                let jpeg = if m.kind == "app" {
                    m.thumb_of.as_ref().and_then(|w| thumbs.get(w).cloned())
                } else {
                    let j = capture::thumb_for(&m.id);
                    if let Some(j) = &j {
                        thumbs.insert(m.id.clone(), j.clone());
                    }
                    j
                };
                if let Some(jpeg) = jpeg {
                    let _ = app2.emit_to(
                        windows::PICKER_LABEL,
                        "picker:thumb",
                        json!({ "id": m.id, "jpeg": jpeg }),
                    );
                }
            }
        })
        .await;
    });
    Ok(metas)
}

/// Picker card click: re-resolve the id against fresh content, stop a
/// live capture if one raced in, start scoped, restore the bar.
/// Errors on stale ids ("no longer available") — the picker shows it
/// and refetches.
#[cfg(not(target_os = "linux"))]
#[tauri::command]
fn capture_pick_select(app: AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _transition = state.gate_transition.lock();
    if *state.gate.lock() != Gate::Main {
        return Err("picker is only available in the main window".into());
    }
    let res = capture::resolve_candidate(&id).map_err(|e| e.to_string())?;
    if state
        .capture
        .lock()
        .as_ref()
        .is_some_and(PlatformCapture::is_running)
    {
        stop_capture(&app);
    }
    let target = CaptureTarget {
        kind: res.kind,
        label: res.label,
    };
    start_capture(&app, res.source, res.w, res.h, Some(target));
    let pool = state.pool.lock();
    pool.hide_picker();
    if let Some(bar) = pool.bar() {
        let _ = bar.show();
    }
    Ok(())
}

/// Linux: no in-app candidate list exists — the portal's own dialog owns
/// selection, so the picker window is never shown and the frontend never
/// calls this. A defensive empty list keeps the command surface uniform.
#[cfg(target_os = "linux")]
#[tauri::command]
fn capture_pick_list(_app: AppHandle) -> Result<Vec<PickCandidate>, String> {
    Ok(vec![])
}

/// Linux: nothing to resolve — see `capture_pick_list`.
#[cfg(target_os = "linux")]
#[tauri::command]
fn capture_pick_select(_app: AppHandle, _id: String) -> Result<(), String> {
    Err("the portal dialog owns source selection on Linux".into())
}

/// Esc / Cancel: drop the picker, restore the bar. No gate check —
/// cancel must always be safe.
#[tauri::command]
fn capture_pick_cancel(app: AppHandle) {
    let state = app.state::<AppState>();
    let pool = state.pool.lock();
    pool.hide_picker();
    if let Some(bar) = pool.bar() {
        let _ = bar.show();
    }
}

// ---------------------------------------------------------------------------
// Commands — sessions
// ---------------------------------------------------------------------------

#[tauri::command]
fn session_list(state: State<'_, AppState>) -> Result<Vec<Session>, String> {
    state.db.session_list().map_err(|e| e.to_string())
}

#[tauri::command]
fn session_get(state: State<'_, AppState>, id: i64) -> Result<Vec<Message>, String> {
    state.db.messages_for(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn transcripts_for(
    state: State<'_, AppState>,
    id: i64,
    limit: Option<usize>,
) -> Result<Vec<Transcript>, String> {
    state
        .db
        .transcripts_for(id, limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn summary_latest(state: State<'_, AppState>, id: i64) -> Result<Option<Summary>, String> {
    state.db.summary_latest(id).map_err(|e| e.to_string())
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
    match state
        .db
        .session_active_id(&kind)
        .map_err(|e| e.to_string())?
    {
        Some(id) => {
            state.db.session_end(id).map_err(|e| e.to_string())?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Resume a past chat: ends the open `ask` session and reopens `id`
/// (`session_reopen` kind-guards — non-ask ids change nothing and
/// return false). The next `ask_send` appends to the reopened session.
#[tauri::command]
fn session_resume(state: State<'_, AppState>, id: i64) -> Result<bool, String> {
    state
        .db
        .session_reopen(id, "ask")
        .map_err(|e| e.to_string())
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
/// (`#rrggbb`, `""` resets to the spec slate), `app.main_language`
/// (`en|zh|ja|ko|fr|es` — chat/summary output; STT auto-detects),
/// `compat.name`,
/// `compat.base_url` (validated http(s) URL; `""` clears),
/// `models.stt_provider` (`deepgram|whisper|sherpa`), and `models.stt_model`
/// (a trimmed non-empty identifier), `recording.auto_screenshots` (bool),
/// `recording.fps` (`8|4|2` — a write during a live capture restarts it
/// so the new rate applies now), `recording.read_interval_secs` (u64
/// ≥1 — minimum seconds between ambient screen reads; applies on the
/// next capture start), and `recording.summary_prompt` (string).
/// Provider order/switches/models have
/// their own commands (`providers_reorder`,
/// `provider_set_enabled`, `model_set_selected`). Persists `config.toml`
/// and returns the updated config. A `hotkeys.*` write delta-swaps the
/// registered set. Every
/// successful write broadcasts `config:changed` so open windows
/// re-render (appearance flips, provider lists, the bar's drag hint).
///
/// `app.onboarding_done` is the wizard's completion write: `true` ends
/// onboarding → `transition_gate` can now reach `Main` (capture, the
/// card) and the bar appears; `false` (a re-run) reverses it.
#[tauri::command]
fn config_set(app: AppHandle, key: String, value: serde_json::Value) -> Result<Config, String> {
    let state = app.state::<AppState>();
    let mut hotkeys_changed = false;
    let mut onboarding_changed = false;
    let mut accent_changed = false;
    let mut stt_changed = false;
    let fps_changed: bool;
    {
        let mut cfg = state.config.lock();
        let prev_fps = cfg.recording.fps;
        match key.as_str() {
            "window.bar_x" => cfg.window.bar_x = window_pref_value(&value)?,
            "window.bar_y" => cfg.window.bar_y = window_pref_value(&value)?,
            "window.bar_locked" => {
                cfg.window.bar_locked =
                    value.as_bool().ok_or("window.bar_locked must be a bool")?;
            }
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
                let next = if v.is_empty() {
                    config::DEFAULT_ACCENT.to_string()
                } else {
                    let ok = v.len() == 7
                        && v.starts_with('#')
                        && v[1..].chars().all(|c| c.is_ascii_hexdigit());
                    if !ok {
                        return Err("app.accent must be a #rrggbb color".to_string());
                    }
                    v
                };
                accent_changed = cfg.app.accent != next;
                cfg.app.accent = next;
            }
            "app.main_language" => {
                cfg.app.main_language = config::validate_main_language(
                    value.as_str().ok_or("app.main_language must be a string")?,
                )?;
            }
            key if config::apply_recording_config(&mut cfg.recording, key, &value)? => {}
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
            key if config::apply_stt_config(&mut cfg.models, key, &value)? => {
                stt_changed = true;
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
                if !config::default_hotkeys().contains_key(name) {
                    return Err(format!("unknown hotkey action {name:?}"));
                }
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
        fps_changed = cfg.recording.fps != prev_fps;
    }
    if hotkeys_changed {
        swap_hotkeys(&app);
    }
    if accent_changed {
        // Re-tint the bar's glass now — otherwise the new accent only
        // reaches the material on the next pill⇄card morph.
        state.pool.lock().refresh_bar_glass(&app);
    }
    if fps_changed
        && state
            .capture
            .lock()
            .as_ref()
            .is_some_and(PlatformCapture::is_running)
    {
        // A live session keeps its old cadence — rebuild it so the new
        // rate applies immediately.
        stop_capture(&app);
        match primary_display_source() {
            Ok((source, w, h)) => {
                start_capture(&app, source, w, h, None);
            }
            Err(e) => log::warn!("capture: display source failed: {e}"),
        }
    }
    if onboarding_changed {
        // Gate first: `enter_main` starts capture while the
        // wizard is still the visible window; then the bar un-hides.
        transition_gate(&app);
        state.sync_bar_visibility();
    }
    let updated = state.config.lock().clone();
    let _ = app.emit("config:changed", &updated);
    if stt_changed {
        refresh_speech_setup(&app);
    }
    // bar_locked/edge/hotkey writes all show up in the menu's
    // labels/checks — rebuild unconditionally (writes are user-driven).
    refresh_tray_menu(&app);
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

/// App-exit teardown for every speech session — meeting Listen and Ask
/// dictation share the microphone, so quitting must release both.
fn stop_speech(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.listen.stop();
    let _ = state.dictation.stop();
}

#[tauri::command]
fn quit_application(app: AppHandle) {
    stop_speech(&app);
    stop_capture(&app);
    app.exit(0);
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Default our crate to info so dev runs show the screen-read
    // diagnostics; `RUST_LOG` still overrides everything.
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("marvis_lib=info"),
    )
    .try_init();
    let builder = tauri::Builder::default()
        // Order matters: the deep-link plugin must be registered before
        // `deeplink::init` resolves `app.deep_link()` inside `setup`.
        .plugin(tauri_plugin_deep_link::init())
        // Required before `app.global_shortcut()` (hotkey registration).
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_opener::init());
    // Windows/Linux: a second `marvis://` launch forwards to the running
    // instance instead of spawning a duplicate (deep-link rides this
    // channel). macOS delivers open-url natively — leave it untouched.
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        let state = app.state::<AppState>();
        let pool = state.pool.lock();
        if let Some(bar) = pool.bar() {
            let _ = bar.show();
            let _ = bar.set_focus();
        }
    }));
    // Liquid glass is macOS-only — other OSes keep the webview's CSS
    // frost (`surface_material` reports "none").
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_plugin_liquid_glass::init());
    builder
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
            let pool = WindowPool::create_bar_only(handle, onboarding_done, &cfg.app.accent)?;
            let voice_models = voice_models::VoiceModelManager::new();
            voice_models.attach_app(handle.clone());
            let sherpa_models = sherpa_models::SherpaModelManager::new();
            sherpa_models.attach_app(handle.clone());
            // Tauri externalBin sidecars are staged beside the executable
            // (`Contents/MacOS` in a macOS app), not under `Contents/Resources`.
            let bundled_whisper = std::env::current_exe().ok().and_then(|executable| {
                executable.parent().map(|executable_dir| {
                    let target_specific = paths::bundled_whisper_cli(executable_dir);
                    if target_specific.is_file() {
                        target_specific
                    } else {
                        // Tauri strips the target suffix when it copies an
                        // externalBin into a packaged app.
                        paths::packaged_whisper_cli(executable_dir)
                    }
                })
            });
            app.manage(AppState {
                keystore: Mutex::new(keystore),
                config: Mutex::new(cfg),
                db: Arc::new(db),
                ring: Arc::new(Mutex::new(RingBuffer::new(RING_MAX_FRAMES, RING_MAX_BYTES))),
                capture: Mutex::new(None),
                screen_reader: Arc::new(screen_read::ScreenReader::new()),
                capture_target: Mutex::new(None),
                ask: Arc::new(AskService::new()),
                listen: Arc::new(ListenService::new()),
                dictation: Arc::new(DictationService::new()),
                bundled_whisper,
                pool: Mutex::new(pool),
                hotkeys: Mutex::new(None),
                gate: Mutex::new(Gate::NeedsPermission),
                gate_transition: Mutex::new(()),
                speech_lifecycle: tokio::sync::Mutex::new(()),
                alert: Mutex::new(None),
                voice_models,
                sherpa_models,
                voice_enroll: voiceprint::VoiceEnroll::new(),
            });
            deeplink::init(handle, deeplink_dispatch(handle))?;
            // Warn-and-continue like hotkeys: a missing tray must never
            // wedge startup.
            if let Err(e) = tray::init(handle) {
                log::warn!("tray init failed: {e}");
            }
            // One global dispatcher for every `menu.*` item — tray menu,
            // bar popup, and the app menubar share both the builders and
            // this listener.
            handle.on_menu_event(menu_dispatch());
            // The app menubar replaces tauri's generated default. macOS
            // only: `Builder::menu`/`app.set_menu` would attach the menu
            // to EVERY menu-less window on Windows/Linux — including the
            // borderless bar — so those platforms hang it on the prefs
            // window in windows/mod.rs instead.
            #[cfg(target_os = "macos")]
            {
                // `live_bar_edge` refreshes from the window's real rect
                // first: a `Moved` frame can lag `position_bar_at_startup`'s
                // `set_rect`, leaving `bar_rect` on the creation frame and
                // the Position checks pinned to the wrong edge at launch.
                let edge = handle.state::<AppState>().pool.lock().live_bar_edge();
                match menubar::build(handle, edge) {
                    Ok(menu) => {
                        if let Err(e) = handle.set_menu(menu) {
                            log::warn!("app menubar install failed: {e}");
                        }
                    }
                    Err(e) => log::warn!("app menubar build failed: {e}"),
                }
            }
            // The one chrome-level binding is gate-independent; kept in
            // state so a `config_set` rebind delta-swaps against it.
            let binds = handle.state::<AppState>().config.lock().hotkeys.clone();
            match hotkey::register_all(handle, &binds, hotkey_dispatch(handle)) {
                Ok(set) => {
                    handle.state::<AppState>().hotkeys.lock().replace(set);
                }
                Err(e) => log::warn!("startup hotkey registration failed: {e}"),
            }
            // Computes the gate, enters `Main` if it's already open, and
            // emits `app:state` either way. Onboarding blocks `Main`, so
            // a first run can never light capture here.
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
            ask_retry,
            ask_close,
            ask_send_screen_only,
            ask_current,
            listen_start,
            listen_stop,
            listen_pause,
            listen_resume,
            listen_status,
            dictation_start,
            dictation_stop,
            dictation_status,
            voice_models_catalog,
            whisper_status,
            whisper_download,
            whisper_cancel_download,
            whisper_remove_model,
            sherpa_status,
            sherpa_download,
            sherpa_cancel_download,
            sherpa_remove_model,
            voiceprint_status,
            voice_enroll_start,
            voice_enroll_stop,
            voice_enroll_cancel,
            voiceprint_remove,
            alert_show,
            alert_current,
            alert_dismiss,
            window_toggle_all,
            window_set_chat_open,
            window_focus_bar,
            window_show_settings,
            window_show_onboarding,
            window_hide_prefs,
            prefs_mode,
            window_adjust_height,
            window_snap_edge,
            window_recenter,
            window_bar_edge,
            window_set_bar_expanded,
            bar_context_menu,
            open_devtools,
            permissions_status,
            permissions_request_screen,
            permissions_request_mic,
            permissions_open_prefs,
            capture_start,
            capture_pick_and_start,
            capture_pick_begin,
            capture_pick_list,
            capture_pick_select,
            capture_pick_cancel,
            capture_stop,
            capture_status,
            session_list,
            session_get,
            transcripts_for,
            summary_latest,
            session_delete,
            session_end_active,
            session_resume,
            config_get,
            config_set,
            surface_material,
            quit_application,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Keep capture + speech teardown centralized at the application
            // boundary: this covers tray/menu quit, window-manager quit, and
            // other native exit paths in addition to the explicit command
            // above.
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                stop_speech(app);
                stop_capture(app);
            }
        });
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

    #[test]
    fn removed_listen_placeholder_is_absent_from_command_contract() {
        // Keep this tied to the actual registration source rather than a
        // second hand-maintained list of command names.
        assert!(!include_str!("lib.rs").contains(concat!("listen_", "stub")));
        let source = include_str!("lib.rs");
        assert!(source.contains("listen_start,"));
        assert!(source.contains("listen_stop,"));
        assert!(source.contains("listen_pause,"));
        assert!(source.contains("listen_resume,"));
        assert!(source.contains("listen_status,"));
        assert!(source.contains("dictation_start,"));
        assert!(source.contains("dictation_stop,"));
        assert!(source.contains("dictation_status,"));
        assert!(source.contains("whisper_status,"));
        assert!(source.contains("sherpa_status,"));
        assert!(source.contains("sherpa_download,"));
        assert!(source.contains("sherpa_cancel_download,"));
        assert!(source.contains("sherpa_remove_model,"));
        assert!(source.contains("session_resume,"));
    }

    /// The punctuation auto-install chain: a completed sherpa download
    /// task clears its own `active` slot then re-enters
    /// `refresh_speech_setup`, which calls `ensure_punct` — the unit
    /// tests drive `ensure_punct` directly, so this asserts both halves
    /// of the re-entrant wiring stay connected.
    #[test]
    fn punct_chain_reenters_refresh_and_refresh_calls_ensure_punct() {
        assert!(include_str!("sherpa_models.rs").contains("crate::refresh_speech_setup(app)"));
        assert!(include_str!("lib.rs").contains("ensure_punct("));
    }

    /// Both speech services must route providers through the shared
    /// curated setup check — a missing model download or key is a
    /// Settings fix surfaced as `needs_setup`, never a raw engine/path
    /// error.
    #[test]
    fn sherpa_setup_check_is_wired_into_both_speech_services() {
        assert!(include_str!("dictation.rs").contains("stt_setup_error("));
        assert!(include_str!("listen.rs").contains("stt_setup_error("));
    }

    #[test]
    fn app_exit_registration_covers_requested_and_completed_tauri_exit() {
        let source = include_str!("lib.rs");
        assert!(source.contains("tauri::RunEvent::ExitRequested { .. }"));
        assert!(source.contains("| tauri::RunEvent::Exit"));
        assert!(source.contains("stop_speech(app);"));
    }

    /// The app menubar keeps its contract: About carries `version
    /// (commit-count build)` + the copyright line, Settings and the
    /// Position items reuse the shared `menu.*` ids so the single
    /// dispatcher covers them, and the persistent menubar's Position
    /// checks are re-synced from the shared refresh funnel (it is never
    /// rebuilt like the tray copy).
    #[test]
    fn app_menubar_keeps_about_settings_and_position_wiring() {
        let menubar = include_str!("menubar.rs");
        assert!(menubar.contains("MARVIS_BUILD_NUMBER"));
        assert!(menubar.contains("© 2026 Marvis AI LLC"));
        assert!(menubar.contains("menus::MENU_SETTINGS"));
        assert!(menubar.contains("position_submenu"));
        // The View menu also carries the Lock check — synced by the
        // same funnel — and dodges AppKit's "Enter Full Screen"
        // injection via a zero-width-space title.
        assert!(menubar.contains("lock_item"));
        assert!(menubar.contains("menus::MENU_LOCK"));
        assert!(menubar.contains("View\\u{200B}"));
        // Help menu: native `HELP_SUBMENU_ID` (macOS adds its search
        // field) + the support mailto through the shared dispatcher.
        assert!(menubar.contains("HELP_SUBMENU_ID"));
        assert!(menubar.contains("menus::MENU_SUPPORT"));
        assert!(menubar.contains("mailto:support@getmarvis.com"));
        let lib = include_str!("lib.rs");
        assert!(lib.contains("menubar::sync_position_checks"));
        assert!(lib.contains("menus::MENU_SUPPORT =>"));
        // The shared menu and the menubar must build their Position
        // submenus from the same helper — two hand-maintained copies of
        // the `menu.pos.*` items would silently drift.
        let menus = include_str!("menus.rs");
        assert!(menus.contains("position_submenu(app, edge)?"));
    }

    /// The speech commands keep their documented guards in source: both
    /// start commands gate on `Main`, and each refuses to run while the
    /// other mode's durable status is `listening` (mutual exclusion must
    /// hold even for a stale/racing invoke).
    #[test]
    fn speech_commands_keep_gate_and_mutual_exclusion_guards() {
        let source = include_str!("lib.rs");
        assert!(source.contains("Dictation is unavailable until setup is complete"));
        assert!(source.contains("state.listen.status().is_listening()"));
        assert!(source.contains("state.dictation.status().is_listening()"));
        assert!(source.contains("dictation:state"));
        assert!(source.contains("dictation:draft"));
        assert!(source.contains("dictation:error"));
    }

    /// Both start commands must hold `speech_lifecycle` across their
    /// peer check, the mic-permission await, and the service start —
    /// that's what makes the mutual exclusion atomic against a
    /// concurrent first-start (the check-then-act gap is what raced).
    #[test]
    fn speech_starts_share_a_lifecycle_lock() {
        let source = include_str!("lib.rs");
        assert!(source.contains("speech_lifecycle: tokio::sync::Mutex<()>"));
        for signature in ["async fn listen_start", "async fn dictation_start"] {
            let body = source
                .split(signature)
                .nth(1)
                .and_then(|rest| rest.split("\n#[tauri::command]").next())
                .unwrap_or_else(|| panic!("{signature} body not found"));
            assert!(
                body.contains("state.speech_lifecycle.lock().await"),
                "{signature} must hold the speech lifecycle lock across the await"
            );
        }
    }

    /// `capture_start` keeps the same crafted-invoke guard as
    /// `ask_send`/`ask_send_screen_only`: it mutates only while the gate
    /// is `Main`, and holds `gate_transition` across the check + start
    /// so a racing `transition_gate` can't interleave between them.
    /// `capture_stop` deliberately stays ungated — stopping must always
    /// be safe.
    #[test]
    fn capture_start_is_gate_guarded_but_capture_stop_is_not() {
        let source = include_str!("lib.rs");
        let start_body = source
            .split("fn capture_start(app: AppHandle)")
            .nth(1)
            .and_then(|rest| rest.split("\n#[tauri::command]").next())
            .expect("capture_start body not found");
        assert!(
            start_body.contains("*state.gate.lock() != Gate::Main"),
            "capture_start must drop the invoke while gate != Main"
        );
        let transition_lock = start_body
            .find("state.gate_transition.lock()")
            .expect("capture_start must hold gate_transition across check + start");
        let gate_check = start_body.find("*state.gate.lock() != Gate::Main").unwrap();
        let start_call = start_body
            .find("start_capture(&app, source, w, h, None)")
            .expect("capture_start must resolve the display filter into start_capture");
        assert!(
            transition_lock < gate_check && transition_lock < start_call,
            "gate_transition must be taken before the gate check and start_capture"
        );
        let stop_body = source
            .split("fn capture_stop(app: AppHandle)")
            .nth(1)
            .and_then(|rest| rest.split("\nfn ").next())
            .expect("capture_stop body not found");
        assert!(
            !stop_body.contains("Gate::Main"),
            "capture_stop must stay callable at any gate"
        );
    }

    /// `capture_pick_and_start` mutates (the picked scope becomes the
    /// live capture), so it keeps the same crafted-invoke gate guard as
    /// `capture_start`, and the `Picked` callback re-checks the gate
    /// under `gate_transition` — a pick must never light capture
    /// outside `Main`. `try_lock`, not `lock`: the callback runs on the
    /// main thread and a `gate_transition` holder blocks on it via
    /// `tray.set_menu`, so parking here deadlocks. The body is bounded
    /// on the next `fn` so the assertions can't leak into neighbouring
    /// tests.
    #[test]
    fn capture_pick_and_start_is_gate_guarded() {
        let src = include_str!("lib.rs");
        let body = src
            .split("fn capture_pick_and_start")
            .nth(1)
            .and_then(|rest| rest.split("\nfn ").next())
            .expect("command exists");
        assert!(
            body.contains("*state.gate.lock() != Gate::Main"),
            "picker start must be Main-gated"
        );
        assert!(
            body.contains("gate_transition.try_lock()"),
            "the picked callback must re-check the gate under gate_transition \
             via try_lock (main-thread callback must never park on it)"
        );
    }

    /// Marvis's windows stay user-capturable on macOS (no
    /// `set_content_protected`), so the picker's own filter — which
    /// carries no self-exclusion — would composite our bar into a
    /// DISPLAY pick's recording. The display arm must re-resolve
    /// through `resolve_candidate` (`d:` = display minus own windows);
    /// window/app picks can't be ours (`excluded_bundle_ids`).
    #[test]
    fn capture_pick_and_start_reresolves_display_picks() {
        let src = include_str!("lib.rs");
        let body = src
            .split("fn capture_pick_and_start")
            .nth(1)
            .and_then(|rest| rest.split("\nfn ").next())
            .expect("command exists");
        assert!(
            body.contains("resolve_candidate(&format!(\"d:{id}\"))"),
            "a display pick must rebuild the filter via resolve_candidate (self-excluding)"
        );
        assert!(
            body.contains("set_excluded_bundle_ids"),
            "Marvis must stay excluded from the picker's offer list"
        );
    }

    /// Windows parity with the macOS rule "visible to the user,
    /// invisible to Marvis's own captures": WGC can't exclude windows
    /// from a monitor grab, so `capture/windows.rs` applies
    /// `WDA_EXCLUDEFROMCAPTURE` dynamically — a refcounted guard holds
    /// it only while a capture session is live, then restores
    /// `WDA_NONE`. `build_window` must consult `protection_engaged`
    /// (not protect unconditionally) so a window born mid-capture
    /// starts protected without hiding Marvis from user screenshots
    /// the rest of the time.
    #[test]
    fn windows_self_protects_only_while_capturing() {
        let backend = include_str!("capture/windows.rs");
        for needle in [
            "struct SelfProtection",
            "SetWindowDisplayAffinity",
            "WDA_EXCLUDEFROMCAPTURE",
            "WDA_NONE",
            "pub(crate) fn protection_engaged()",
        ] {
            assert!(backend.contains(needle), "windows backend must keep {needle}");
        }
        let pool = include_str!("windows/mod.rs");
        assert!(
            pool.contains("set_content_protected(crate::capture::protection_engaged())"),
            "windows builds must consult protection_engaged, not protect unconditionally"
        );
    }

    /// The custom picker's begin/select share `capture_start`'s
    /// crafted-invoke guard: `gate_transition` held across the check +
    /// action so a racing leave-Main can't interleave. `select`
    /// additionally stops a live capture first — `start_capture` is
    /// idempotent and would silently keep the old scope.
    #[test]
    fn capture_pick_commands_are_gate_guarded() {
        let source = include_str!("lib.rs");
        let body = |sig: &str| {
            source
                .split(sig)
                .nth(1)
                .and_then(|rest| rest.split("\n#[tauri::command]").next())
                .unwrap_or_else(|| panic!("{sig} body not found"))
        };
        for sig in [
            "fn capture_pick_begin(app: AppHandle)",
            "fn capture_pick_list(app: AppHandle)",
            "fn capture_pick_select(app: AppHandle, id: String)",
        ] {
            let b = body(sig);
            let lock = b
                .find("state.gate_transition.lock()")
                .unwrap_or_else(|| panic!("{sig} must hold gate_transition"));
            let check = b
                .find("*state.gate.lock() != Gate::Main")
                .unwrap_or_else(|| panic!("{sig} must check Gate::Main"));
            assert!(lock < check, "{sig}: lock must precede the check");
        }
        let select = body("fn capture_pick_select(app: AppHandle, id: String)");
        let stop = select
            .find("stop_capture(&app)")
            .expect("select must stop a live capture before retargeting");
        let start = select
            .find("start_capture(&app")
            .expect("select must start_capture with the resolved filter");
        assert!(stop < start, "select must stop before starting");
    }

    /// `leave_main` must stop dictation: the session is bound to the
    /// ask input that leaving Main hides, so an invisible dictation
    /// must not keep the microphone. Listen is deliberately untouched —
    /// meeting Listen is independent of card visibility.
    #[test]
    fn leave_main_stops_dictation_but_not_listen() {
        let source = include_str!("lib.rs");
        let body = source
            .split("fn leave_main(app: &AppHandle)")
            .nth(1)
            .and_then(|rest| rest.split("\nfn ").next())
            .expect("leave_main body not found");
        assert!(body.contains("state.dictation.stop()"));
        assert!(!body.contains("state.listen.stop()"));
    }

    /// Mutual exclusion keys off the durable `listening` state of each
    /// service — `error`/`idle` must not block the other mode.
    #[test]
    fn speech_status_predicates_only_match_listening() {
        assert!(!listen::ListenStatus {
            state: "idle".into(),
            provider: None,
            session_id: None,
            turns: 0,
            mic: false,
            error: None,
            started_at: None,
            paused_secs: 0,
            paused_since: None,
        }
        .is_listening());
        assert!(listen::ListenStatus {
            state: "listening".into(),
            provider: None,
            session_id: None,
            turns: 0,
            mic: false,
            error: None,
            started_at: None,
            paused_secs: 0,
            paused_since: None,
        }
        .is_listening());
        // A paused Listen still owns the audio sources — mutual exclusion
        // must hold through the pause.
        assert!(listen::ListenStatus {
            state: "paused".into(),
            provider: None,
            session_id: None,
            turns: 0,
            mic: false,
            error: None,
            started_at: None,
            paused_secs: 0,
            paused_since: None,
        }
        .is_listening());

        assert!(!dictation::DictationStatus {
            state: "error".into(),
            provider: None,
            error: None,
        }
        .is_listening());
        assert!(dictation::DictationStatus {
            state: "listening".into(),
            provider: None,
            error: None,
        }
        .is_listening());
    }

    /// The dictation service is wired into `AppState` alongside Listen and
    /// starts idle; stop is safe on a fresh service.
    #[test]
    fn dictation_service_is_wired_into_app_state() {
        let tmp = tmp_dir();
        let state = AppState::for_test(&tmp);
        let status = state.dictation.status();
        assert_eq!(status.state, "idle");
        assert_eq!(status.provider, None);
        assert_eq!(status.error, None);
        let draft = state.dictation.stop();
        assert_eq!(draft.text, "");
        assert!(draft.finality);
        let _ = std::fs::remove_dir_all(&tmp);
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

    /// The `capture:state` contract carries the picker-selected scope:
    /// `target` is `null` on the auto primary-display path and the
    /// `{"kind","label"}` pair after a picker selection — the picker's
    /// status UI (Task 7) keys off exactly this shape.
    #[test]
    fn capture_snapshot_serializes_the_picker_target() {
        let tmp = tmp_dir();
        let state = AppState::for_test(&tmp);
        let snap = capture_snapshot(&state);
        assert_eq!(snap["running"], false);
        assert_eq!(snap["frames"], 0);
        assert!(snap["target"].is_null(), "auto path reports null target");

        *state.capture_target.lock() = Some(CaptureTarget {
            kind: "window",
            label: "Finder".into(),
        });
        let snap = capture_snapshot(&state);
        assert_eq!(snap["target"]["kind"], "window");
        assert_eq!(snap["target"]["label"], "Finder");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The failover chain honours `providers.order`, skips disabled ids,
    /// and drops providers with no key (where required) or no model.
    #[test]
    fn deepgram_keys_are_trimmed_and_unverified() {
        assert_eq!(
            normalize_deepgram_key("  dg-test-key \n").unwrap(),
            "dg-test-key"
        );

        let accepted = deepgram_validation_payload("  dg-test-key \t");
        assert_eq!(accepted["ok"], true);
        assert!(accepted["message"]
            .as_str()
            .unwrap()
            .contains("no live provider probe"));

        let rejected = deepgram_validation_payload(" \t\n");
        assert_eq!(rejected["ok"], false);
        assert!(rejected["error"]
            .as_str()
            .unwrap()
            .contains("after trimming"));
    }

    #[test]
    fn deepgram_is_key_only_and_never_enters_llm_chain() {
        assert!(is_key_management_provider("deepgram"));
        assert!(!is_key_management_provider("not-a-provider"));

        let tmp = tmp_dir();
        let state = AppState::for_test(&tmp);
        {
            let mut ks = state.keystore.lock();
            ks.set_key("deepgram", "dg-test-key").unwrap();
        }
        let (cfg, ks) = (state.config.lock(), state.keystore.lock());
        assert!(provider_candidates(&cfg, &ks)
            .iter()
            .all(|c| c.id != "deepgram"));
        assert_eq!(
            ks.masked_status(),
            vec![("deepgram".into(), Some("…-key".into()))]
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

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
