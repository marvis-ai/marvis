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
//! - `memory:changed` `{}` — broadcast when a stored fact is added,
//!   edited, or deleted (a background extraction landing rows, or a
//!   Settings → Memory edit/delete); the Memory tab refetches on it.

mod ask;
pub mod audio;
mod capture;
mod commands;
mod config;
mod deeplink;
mod dictation;
mod hotkey;
mod keystore;
mod listen;
mod llm;
mod memory;
mod menubar;
mod menus;
mod paths;
mod permissions;
mod presets;
mod prompts;
mod screen_read;
mod session_lifecycle;
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
pub(crate) use commands::*;
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
use session_lifecycle::SessionLifecycle;
use storage::{Db, Memory, Message, Session, Summary, Transcript};
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
/// `config`, `pool` are plain `Mutex`es) while `db`/`ring`/`ask`/`lifecycle`
/// are `Arc`s shared with spawned tasks and the capture callback.
pub struct AppState {
    keystore: Mutex<Keystore>,
    /// `Arc`d like `db`/`ring`: the scheduled memory extraction
    /// re-reads `[memory].enabled` from inside its spawned task —
    /// the snapshot `prepare_hook` checked can go stale while an ask
    /// streams or the extraction queues on the service gate.
    config: Arc<Mutex<Config>>,
    db: Arc<Db>,
    /// Shared per-session handoff boundary — Ask/compaction leases are
    /// coordinated with the production session deletion command.
    lifecycle: Arc<SessionLifecycle>,
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
    /// Restart inputs, nested under `capture` just like `capture_target`.
    capture_restart: Mutex<Option<CaptureRestart>>,
    ask: Arc<AskService>,
    /// Serialized memory extraction (`[memory]`-gated) — shared with
    /// every ask send's `MemoryHook` schedule. Owned like `ask`/`listen`.
    memory: Arc<memory::MemoryService>,
    /// Serialized session compaction — shared with every ask send's
    /// detached `CompactHook` schedule.
    compact: Arc<ask::CompactService>,
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
    /// command futures stay `Send`. Listen stop also takes it; dictation's
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
    /// The ask pipeline's borrow bundle: `db`/`ring`/`reader`/`memory`/
    /// `lifecycle`/`config` clone their `Arc`s (the spawned stream outlives the
    /// call), the rest are short-lived `&Mutex` borrows used only in
    /// pre-flight. `capture_running` snapshots the capture slot so
    /// `resolve_screen` knows whether ring frames are fresh.
    fn deps(&self) -> ask::Deps<'_> {
        ask::Deps {
            db: Arc::clone(&self.db),
            lifecycle: Arc::clone(&self.lifecycle),
            ring: Arc::clone(&self.ring),
            reader: Arc::clone(&self.screen_reader),
            memory: Arc::clone(&self.memory),
            compact: Arc::clone(&self.compact),
            config: Arc::clone(&self.config),
            capture_running: self
                .capture
                .lock()
                .as_ref()
                .is_some_and(PlatformCapture::is_running),
            keystore: &self.keystore,
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
            config: Arc::new(Mutex::new(
                Config::load_from(root.join("config.toml")).unwrap_or_default(),
            )),
            db: Arc::new(Db::at(root.join("marvis.db")).expect("test db")),
            lifecycle: SessionLifecycle::new(),
            ring: Arc::new(Mutex::new(RingBuffer::new(RING_MAX_FRAMES, RING_MAX_BYTES))),
            capture: Mutex::new(None),
            screen_reader: Arc::new(screen_read::ScreenReader::new()),
            capture_target: Mutex::new(None),
            capture_restart: Mutex::new(None),
            ask: Arc::new(AskService::new()),
            memory: memory::MemoryService::new(),
            compact: ask::CompactService::new(),
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

/// Hop window work onto the main thread — the pool's getters park the
/// caller on the main queue, so running them off a tokio worker is the
/// ABBA deadlock the window-event `try_lock`s exist to avoid. `what`
/// labels the warn; a dispatch failure only logs (fire-and-forget).
fn run_on_main(
    app: &AppHandle,
    what: &'static str,
    work: impl FnOnce(&AppHandle) + Send + 'static,
) {
    let app2 = app.clone();
    if let Err(e) = app.run_on_main_thread(move || work(&app2)) {
        log::warn!("{what}: main-thread hop failed: {e}");
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

/// `Main` exit (onboarding reset / permission revoked): collapse the
/// card, stop dictation, stop + drop capture, hide the alert toast.
fn leave_main(app: &AppHandle) {
    let state = app.state::<AppState>();
    // A gate exit is teardown, not a view leave — retire every live
    // run (each tagged `idle` still emits so mirrors and packet
    // filters settle), then collapse. Listen is independent of card
    // visibility; only `listen_stop` stops it.
    state.ask.retire_all(app);
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

#[derive(Clone)]
struct CaptureRestart {
    source: CaptureSource,
    width: u32,
    height: u32,
    target: Option<CaptureTarget>,
    resolver_id: Option<String>,
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
    start_capture_with_resolver(app, source, width, height, target, None)
}

fn start_capture_with_resolver(
    app: &AppHandle,
    source: CaptureSource,
    width: u32,
    height: u32,
    target: Option<CaptureTarget>,
    resolver_id: Option<String>,
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
            let restart = CaptureRestart {
                source: source.clone(),
                width,
                height,
                target: target.clone(),
                resolver_id,
            };
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
                        *state.capture_restart.lock() = Some(restart);
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
    *state.capture_restart.lock() = None;
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
/// Emitted to the `bar` window only — a preset palette pick
/// (`presets_palette_select`); payload is the full `Preset`.
const EV_PRESET_PICK: &str = "bar:preset-pick";
/// Rust→palette events emitted from `windows/mod.rs` (`pub(crate)`
/// keeps one const per name): `show_palette` announces the open
/// (`{ query }` seeds the filter), `palette_query` pushes the
/// composer's live `/token`, `palette_key` forwards nav keys while
/// the bar keeps focus.
pub(crate) const EV_PALETTE_OPEN: &str = "palette:open";
pub(crate) const EV_PALETTE_QUERY: &str = "palette:query";
pub(crate) const EV_PALETTE_KEY: &str = "palette:key";
/// Emitted to the `bar` window on every palette hide (`windows`'s
/// `hide_palette`) — the composer stops forwarding `/`-mode keys.
/// Emitted from `windows/mod.rs`; `pub(crate)` keeps one const.
pub(crate) const EV_PALETTE_CLOSED: &str = "bar:palette-closed";

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
                state
                    .ask
                    .send(&app, &state.deps(), &text, false, None, None, None, Vec::new());
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
                config: Arc::new(Mutex::new(cfg)),
                db: Arc::new(db),
                lifecycle: SessionLifecycle::new(),
                ring: Arc::new(Mutex::new(RingBuffer::new(RING_MAX_FRAMES, RING_MAX_BYTES))),
                capture: Mutex::new(None),
                screen_reader: Arc::new(screen_read::ScreenReader::new()),
                capture_target: Mutex::new(None),
                capture_restart: Mutex::new(None),
                ask: Arc::new(AskService::new()),
                memory: memory::MemoryService::new(),
                compact: ask::CompactService::new(),
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
            ask_stop,
            ask_send_screen_only,
            ask_runs,
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
            presets_list,
            presets_palette_open,
            presets_palette_select,
            presets_palette_close,
            presets_palette_key,
            presets_palette_query,
            presets_palette_height,
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
            memory_list,
            memory_update,
            memory_delete,
            memory_history,
            save_audio_file,
            save_text_file,
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
    fn audio_export_preserves_original_and_copies_to_other_destinations() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("recording.wav");
        let contents = b"original recording contents";
        std::fs::write(&source, contents).unwrap();
        let source = source.canonicalize().unwrap();
        assert!(copy_audio_export(&source, &source).is_err());
        #[cfg(unix)]
        {
            let alias = dir.join("alias.wav");
            std::os::unix::fs::symlink(&source, &alias).unwrap();
            assert!(copy_audio_export(&source, &alias).is_err());
        }
        assert_eq!(std::fs::read(&source).unwrap(), contents);
        let destination = dir.join("export.wav");
        copy_audio_export(&source, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), contents);
        std::fs::write(&destination, b"old export").unwrap();
        copy_audio_export(&source, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), contents);
        std::fs::remove_dir_all(dir).unwrap();
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

    /// The composer's stop button plus the packet identity that keeps a
    /// stream out of the wrong conversation: `ask_stop` cancels ONE
    /// session's run through `AskService::stop`. Deletion also cancels
    /// its run; ending/resuming sessions only detaches the view, so a
    /// detached run keeps writing to its own session. The emit fold
    /// `run`-tags every `ask:*` packet
    /// (in-flight deliveries of a killed run drop) and `send_chain`
    /// `session_id`-tags them (a detached run's packets drop on any
    /// view that isn't showing that session). `ask_runs` is the
    /// per-session resync replacing the single-run `ask_current`.
    #[test]
    fn ask_stop_and_packet_tagging_are_in_the_contract() {
        let lib = include_str!("lib.rs");
        assert!(lib.contains(concat!("ask", "_stop,")));
        assert!(lib.contains(concat!("ask", "_runs,")));
        let commands = include_str!("commands/ask.rs");
        assert!(commands.contains("pub(crate) fn ask_stop("));
        assert!(commands.contains("state.ask.stop(&app, session_id)"));
        assert!(commands.contains("pub(crate) fn ask_runs("));
        let sessions = include_str!("commands/sessions.rs");
        assert!(sessions.contains("state.ask.stop(&app, id)"));
        let view_boundaries = sessions
            .split("pub(crate) fn session_end_active")
            .nth(1)
            .unwrap();
        assert!(!view_boundaries.contains("state.ask.stop"));
        let ask = include_str!("ask/mod.rs");
        assert!(ask.contains("pub fn stop(&self, app: &AppHandle, session_id: i64)"));
        assert!(ask.contains("payload[\"run\"]"));
        let pipeline = include_str!("ask/pipeline.rs");
        assert!(pipeline.contains("payload[\"session_id\"]"));
    }

    /// The preset surface: `presets_list` plus the palette commands
    /// registered (open/select/close plus the `/`-session plumbing —
    /// key forward, query push, visibility probe), and the palette
    /// event constants declared — all asserted against the
    /// registration source itself. `concat!` keeps each literal out
    /// of this file's text (same trick as the `concat!("listen_",
    /// "stub")` negative assert above) so the asserts can't
    /// self-satisfy.
    #[test]
    fn preset_commands_and_dispatch_are_in_the_contract() {
        let source = include_str!("lib.rs");
        assert!(source.contains(concat!("presets", "_list,")));
        assert!(source.contains(concat!("presets", "_palette_open,")));
        assert!(source.contains(concat!("presets", "_palette_select,")));
        assert!(source.contains(concat!("presets", "_palette_close,")));
        assert!(source.contains(concat!("presets", "_palette_key,")));
        assert!(source.contains(concat!("presets", "_palette_query,")));
        assert!(source.contains(concat!("presets", "_palette_height,")));
        assert!(source.contains(concat!("EV_", "PRESET_PICK")));
        assert!(source.contains(concat!("EV_", "PALETTE_OPEN")));
        assert!(source.contains(concat!("EV_", "PALETTE_QUERY")));
        assert!(source.contains(concat!("EV_", "PALETTE_KEY")));
        assert!(source.contains(concat!("EV_", "PALETTE_CLOSED")));
    }

    /// The export surface: `save_text_file` is registered in
    /// `generate_handler!`, gate-guarded like the palette commands, and
    /// runs its blocking dialog off the async executor. `concat!` keeps
    /// the literal out of this file's text so the assert can't
    /// self-satisfy.
    #[test]
    fn export_command_is_registered_and_gate_guarded() {
        let source = include_str!("lib.rs");
        assert!(source.contains(concat!("save", "_text_file,")));
        let body = include_str!("commands/files.rs")
            .split("async fn save_text_file(")
            .nth(1)
            .and_then(|rest| rest.split("\n/// ").next())
            .expect("save_text_file body not found");
        assert!(
            body.contains("*state.gate.lock() != Gate::Main"),
            "save_text_file must check Gate::Main"
        );
        assert!(
            body.contains("spawn_blocking"),
            "save_text_file's dialog must run off the async executor"
        );
    }

    #[test]
    fn audio_export_command_is_registered_and_guarded() {
        let source = include_str!("lib.rs");
        assert!(source.contains(concat!("save", "_audio_file,")));
        let body = include_str!("commands/files.rs")
            .split("async fn save_audio_file(")
            .nth(1)
            .and_then(|rest| rest.split("\n/// ").next())
            .expect("save_audio_file body not found");
        assert!(body.contains("*state.gate.lock() != Gate::Main"));
        assert!(body.contains("spawn_blocking"));
        assert!(body.contains("audio_file"));
        assert!(!body.contains("source_path"));
    }

    #[test]
    fn suggested_name_sanitizes_separators_controls_and_caps() {
        assert_eq!(sanitize_suggested_name("a/b\\c.md", "marvis-export.md"), "abc.md");
        assert_eq!(sanitize_suggested_name("n\u{0}ame.md", "marvis-export.md"), "name.md");
        assert_eq!(sanitize_suggested_name("   ", "marvis-export.md"), "marvis-export.md");
        assert_eq!(sanitize_suggested_name("", "marvis-export.md"), "marvis-export.md");
        assert_eq!(sanitize_suggested_name(&"x".repeat(200), "marvis-export.md").len(), 80);
        assert_eq!(sanitize_suggested_name("notes.md", "marvis-export.md"), "notes.md");
        assert_eq!(sanitize_suggested_name(" /\\ ", "marvis-export.wav"), "marvis-export.wav");
    }

    /// The punctuation auto-install chain: a completed sherpa download
    /// task clears its own `active` slot then re-enters
    /// `refresh_speech_setup`, which calls `ensure_punct` — the unit
    /// tests drive `ensure_punct` directly, so this asserts both halves
    /// of the re-entrant wiring stay connected.
    #[test]
    fn punct_chain_reenters_refresh_and_refresh_calls_ensure_punct() {
        assert!(include_str!("sherpa_models/manager.rs").contains("crate::refresh_speech_setup(app)"));
        assert!(include_str!("commands/dictation.rs").contains("ensure_punct("));
    }

    /// Both speech services must route providers through the shared
    /// curated setup check — a missing model download or key is a
    /// Settings fix surfaced as `needs_setup`, never a raw engine/path
    /// error.
    #[test]
    fn sherpa_setup_check_is_wired_into_both_speech_services() {
        assert!(include_str!("dictation/mod.rs").contains("stt_setup_error("));
        assert!(include_str!("listen/mod.rs").contains("stt_setup_error("));
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
        let source = concat!(
            include_str!("commands/listen.rs"),
            include_str!("commands/dictation.rs")
        );
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
        for (source, signature) in [
            (include_str!("commands/listen.rs"), "async fn listen_start"),
            (include_str!("commands/dictation.rs"), "async fn dictation_start"),
        ] {
            let body = source
                .split(signature)
                .nth(1)
                .and_then(|rest| rest.split("\n#[tauri::command").next())
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
        let source = include_str!("commands/capture.rs");
        let start_body = source
            .split("fn capture_start(app: AppHandle)")
            .nth(1)
            .and_then(|rest| rest.split("\n#[tauri::command").next())
            .expect("capture_start body not found");
        assert!(
            start_body.contains("*state.gate.lock() != Gate::Main"),
            "capture_start must drop the invoke while gate != Main"
        );
        let transition_lock = start_body
            .find("state.gate_transition.try_lock()")
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
            .and_then(|rest| rest.split("\n#[tauri::command").next())
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
        let src = include_str!("commands/capture.rs");
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
        let src = include_str!("commands/capture.rs");
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
        let pool = include_str!("windows/build.rs");
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
        let source = include_str!("commands/capture.rs");
        let body = |sig: &str| {
            source
                .split(sig)
                .nth(1)
                .and_then(|rest| rest.split("\n#[tauri::command").next())
                .unwrap_or_else(|| panic!("{sig} body not found"))
        };
        for sig in [
            "fn capture_pick_begin(app: AppHandle)",
            "fn capture_pick_list(app: AppHandle)",
            "fn capture_pick_select(app: AppHandle, id: String)",
        ] {
            let b = body(sig);
            let lock = b
                .find("state.gate_transition.try_lock()")
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
            .find("start_capture_with_resolver(&app")
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
            audio_file: None,
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
            audio_file: None,
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
            audio_file: None,
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

    /// `ask_send`'s attachment input is part of the command contract:
    /// the command takes the webview's normalized JPEGs, the input
    /// struct lives in ask.rs, and its serde shape accepts the
    /// camelCase `jpegBase64` key the bridge sends.
    #[test]
    fn ask_attachment_contract_is_registered() {
        let source = include_str!("commands/ask.rs");
        let ask = include_str!("ask/mod.rs");
        assert!(source.contains("attachments: Option<Vec<ask::AskAttachmentInput>>"));
        assert!(ask.contains(concat!("pub struct AskAttachment", "Input")));
        let parsed: crate::ask::AskAttachmentInput = serde_json::from_value(
            json!({"name": "a.png", "jpegBase64": "AA=="}),
        )
        .expect("jpegBase64 must deserialize");
        assert_eq!(parsed.name, "a.png");
        assert_eq!(parsed.jpeg_base64, "AA==");
    }

    /// The memory CRUD surface: all three commands ride
    /// `generate_handler!` and the `memory:changed` broadcast name is
    /// part of this file's contract.
    #[test]
    fn memory_commands_and_event_are_registered() {
        let source = include_str!("lib.rs");
        assert!(source.contains("memory_list,"));
        assert!(source.contains("memory_update,"));
        assert!(source.contains("memory_delete,"));
        assert!(source.contains("memory_history,"));
        assert!(source.contains(concat!("memory", ":changed")));
    }
}
