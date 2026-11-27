//! `lib.rs` — module wiring and the startup sequence.
//!
//! Wires every module together: `AppState` (`state.rs`) is managed via
//! `app.manage(...)` in [`run`]'s `setup`, then reached from commands
//! (`State<'_, AppState>` / `AppHandle`), the hotkey dispatch closure,
//! and the deep-link dispatch closure (both re-resolve state per event
//! so they survive gate transitions). Command handlers live under
//! `commands/`; gate transitions in `gate.rs`; capture start/stop in
//! `capture_control.rs`; tray/menubar sync in `chrome.rs`; event
//! dispatchers in `dispatch.rs`; fatal-error surfacing in `crashlog.rs`.
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
mod capture_control;
mod chrome;
mod commands;
mod config;
mod crashlog;
mod deeplink;
mod dictation;
mod dispatch;
mod gate;
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
mod state;
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
use session_lifecycle::SessionLifecycle;
use storage::{Db, Memory, Message, Session, Summary, Transcript};
use windows::WindowPool;
pub(crate) use capture_control::*;
pub(crate) use chrome::*;
pub(crate) use commands::*;
pub(crate) use crashlog::*;
pub(crate) use dispatch::*;
pub(crate) use gate::*;
pub(crate) use state::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Chain to the default hook so a terminal-launched build still gets
    // its stderr backtrace — the hook adds the crash.log/dialog surfacing.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        report_fatal(&info.to_string());
        default_hook(info);
    }));
    // Default our crate to info so dev runs show the screen-read
    // diagnostics; `RUST_LOG` still overrides everything.
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("marvis_lib=info"),
    )
    .try_init();
    // WebKitGTK's EGL/dmabuf renderer segfaults on exotic Wayland
    // compositors (Niri and friends) — the upstream-documented
    // kill-switch; honored only when the user hasn't set it explicitly.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    // A missing/broken WebView2 runtime is Windows' classic silent
    // killer: every `WebviewWindowBuilder::build` fails, setup returns
    // `Err`, and the process vanishes. Check BEFORE the event loop so the
    // user gets an actionable message instead.
    #[cfg(target_os = "windows")]
    if let Err(e) = tauri::webview_version() {
        report_fatal(&format!(
            "Marvis needs the Microsoft Edge WebView2 Runtime, which is \
             missing or damaged ({e}). Install it from \
             https://developer.microsoft.com/en-us/microsoft-edge/webview2/ \
             then launch Marvis again."
        ));
        std::process::exit(1);
    }
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
        // `?`-propagated setup failures reach here — surface them
        // (crash.log + Windows dialog) instead of aborting silently.
        .unwrap_or_else(|e| {
            report_fatal(&format!("application build failed: {e}"));
            std::process::exit(1);
        })
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
mod tests;
