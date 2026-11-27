//! Shell-UI sync helpers — bar lock/edge/position writes and the tray
//! + menubar rebuild that follows every state change (labels and checks
//! track live state). `refresh_tray_menu` MUST never run while holding
//! `pool`/`config`/`capture`/`hotkeys` locks: `set_menu`/`set_checked`
//! block on the main thread.

use crate::*;

/// The `window.bar_locked` write shared by the Lock menu item and the
/// `toggle_lock` hotkey — config save + `config:changed` broadcast,
/// then the tray menu's check refreshes.
pub(crate) fn set_bar_locked(app: &AppHandle, locked: bool) {
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
pub(crate) fn snap_edge_and_refresh(app: &AppHandle, dir: windows::Dir) {
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
pub(crate) fn refresh_tray_menu(app: &AppHandle) {
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
