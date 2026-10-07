//! `menus.rs` — the ONE menu shared by the tray icon and the bar's
//! idle-state right-click popup, plus the item builders the app
//! menubar (menubar.rs) reuses.
//!
//! Items live under the `menu.*` id namespace — except the preset
//! picker's `preset.<id>` rows ([`build_preset_menu`], matched inside
//! `menu_dispatch` before the `menu.*` arms). Everything dispatches
//! through the single global `on_menu_event` listener registered in
//! lib.rs (`menu_dispatch`). Menu events broadcast to EVERY registered
//! listener (global + per-window), so the prefix match is what keeps
//! dispatch single-fire.
//!
//! The menu is rebuilt rather than mutated: [`build`] reads live state
//! (capture running, listen live, nearest edge, `window.bar_locked`, and
//! `cfg.hotkeys` for the accelerator labels) so a popup is always
//! current, and `refresh_tray_menu` swaps the tray's copy at each
//! state-change funnel.

use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Wry};

use crate::capture::{FrameSource, PlatformCapture};
use crate::windows::Dir;
use crate::AppState;

pub const MENU_ASK: &str = "menu.ask";
pub const MENU_CAPTURE: &str = "menu.capture";
pub const MENU_LISTEN: &str = "menu.listen";
pub const MENU_HISTORY: &str = "menu.history";
pub const MENU_POSITION: &str = "menu.position";
pub const MENU_POS_TOP: &str = "menu.pos.top";
pub const MENU_POS_BOTTOM: &str = "menu.pos.bottom";
pub const MENU_POS_LEFT: &str = "menu.pos.left";
pub const MENU_POS_RIGHT: &str = "menu.pos.right";
pub const MENU_POS_CENTER: &str = "menu.pos.center";
/// The four `menu.pos.*` edge items — the one id→label→`Dir` table
/// behind `position_submenu`, `menu_dispatch`, and the menubar's
/// `sync_checks`, so an edge can't drift between the three copies.
/// `menu.pos.center` is absent: it's a point action, not an edge.
pub(crate) const POS_EDGES: [(&str, &str, Dir); 4] = [
    (MENU_POS_TOP, "Top", Dir::Up),
    (MENU_POS_BOTTOM, "Bottom", Dir::Down),
    (MENU_POS_LEFT, "Left", Dir::Left),
    (MENU_POS_RIGHT, "Right", Dir::Right),
];
pub const MENU_LOCK: &str = "menu.lock";
pub const MENU_SETTINGS: &str = "menu.settings";
pub const MENU_SUPPORT: &str = "menu.support";
/// Debug builds only — `menu_dispatch` and this item both compile out
/// in release.
#[cfg(debug_assertions)]
pub const MENU_DEVTOOLS: &str = "menu.devtools";
pub const MENU_QUIT: &str = "menu.quit";

/// `preset.<id>` — one item per preset; dispatch emits the picked
/// preset to the bar as `bar:preset-pick`. Kept OUTSIDE the `menu.*`
/// namespace so the shared dispatcher's prefix rules stay untouched.
pub const PRESET_ITEM_PREFIX: &str = "preset.";
const MENU_PRESET_TEMPLATES: &str = "menu.presets.templates";
const MENU_PRESET_INSTRUCT: &str = "menu.presets.instruct";

/// The preset id a `preset.*` menu event names, else `None`.
pub fn preset_item_id(item_id: &str) -> Option<&str> {
    item_id.strip_prefix(PRESET_ITEM_PREFIX)
}

/// The wand button's popup (and the input-row right-click): two
/// submenus — Templates, then Instructions — built fresh so custom
/// presets always show; customs follow built-ins within each kind.
pub fn build_preset_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let custom = app.state::<AppState>().config.lock().prompts.custom.clone();
    let presets = crate::presets::all(&custom);
    let mut subs: Vec<Submenu<Wry>> = Vec::new();
    for (menu_id, label, kind) in [
        (
            MENU_PRESET_TEMPLATES,
            "Templates",
            crate::presets::PresetKind::Template,
        ),
        (
            MENU_PRESET_INSTRUCT,
            "Instructions",
            crate::presets::PresetKind::Instruct,
        ),
    ] {
        let mut items: Vec<MenuItem<Wry>> = Vec::new();
        for p in presets.iter().filter(|p| p.kind == kind) {
            items.push(MenuItem::with_id(
                app,
                format!("{PRESET_ITEM_PREFIX}{}", p.id),
                p.name.as_str(),
                true,
                None::<&str>,
            )?);
        }
        let sub = Submenu::with_id(app, menu_id, label, true)?;
        let refs: Vec<&dyn IsMenuItem<Wry>> =
            items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
        sub.append_items(&refs)?;
        subs.push(sub);
    }
    let refs: Vec<&dyn IsMenuItem<Wry>> = subs.iter().map(|s| s as &dyn IsMenuItem<Wry>).collect();
    Menu::with_items(app, &refs)
}

/// Build the shared menu against live state. The pill rect is refreshed
/// from the live window first — a just-finished drag must check the
/// edge it actually landed on (the same read `window_bar_edge` makes).
/// Each lock is taken and released inside this function; callers must
/// not be holding `pool`/`config`/`capture` when they call.
pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let state = app.state::<AppState>();
    let (hotkeys, locked) = {
        let cfg = state.config.lock();
        (cfg.hotkeys.clone(), cfg.window.bar_locked)
    };
    // The item's displayed accelerator is the configured binding — a
    // rebind rewrites the label on the next build. It is display-only:
    // the real chords are the global-shortcut registrations.
    let accel = |action: &str| hotkeys.get(action).cloned();
    let capture_running = state
        .capture
        .lock()
        .as_ref()
        .is_some_and(PlatformCapture::is_running);
    let listen_live = state.listen.status().is_listening();
    let edge = state.pool.lock().live_bar_edge();

    let ask = MenuItem::with_id(
        app,
        MENU_ASK,
        "Start Conversation",
        true,
        accel("toggle_input"),
    )?;
    let capture = MenuItem::with_id(
        app,
        MENU_CAPTURE,
        if capture_running {
            "Stop Screen Recording"
        } else {
            "Start Screen Recording"
        },
        true,
        accel("toggle_capture"),
    )?;
    // Start-only per spec — a live session disables the item rather
    // than toggling it off.
    let listen = MenuItem::with_id(
        app,
        MENU_LISTEN,
        "Start Listening",
        !listen_live,
        accel("start_listen"),
    )?;
    let history = MenuItem::with_id(app, MENU_HISTORY, "History", true, accel("show_history"))?;
    let sep1 = PredefinedMenuItem::separator(app)?;

    let position = position_submenu(app, edge)?;

    let lock = lock_item(app, locked, accel("toggle_lock"))?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    // Display-only accelerator (same caveat as the retired tray item) —
    // the real `Cmd+,` binding is the bar webview's keydown handler.
    let settings = MenuItem::with_id(app, MENU_SETTINGS, "Settings", true, Some("CmdOrCtrl+,"))?;
    // Debug builds only: web-inspector entry in the bar's right-click
    // popup (and the tray copy). No accelerator — popup/tray menus
    // never fire one, so a shortcut label would lie.
    #[cfg(debug_assertions)]
    let dev_sep = PredefinedMenuItem::separator(app)?;
    #[cfg(debug_assertions)]
    let inspect = MenuItem::with_id(app, MENU_DEVTOOLS, "Inspect Element", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit Marvis", true, None::<&str>)?;

    let mut items: Vec<&dyn IsMenuItem<Wry>> = vec![
        &ask as &dyn IsMenuItem<Wry>,
        &capture,
        &listen,
        &history,
        &sep1,
        &position,
        &lock,
        &sep2,
        &settings,
    ];
    #[cfg(debug_assertions)]
    {
        items.push(&dev_sep);
        items.push(&inspect);
    }
    items.push(&quit);
    Menu::with_items(app, &items)
}

/// The `Lock Bar Position` check shared by the shared menu and the app
/// menubar's `View`. The menubar passes `accel = None` — there the
/// accelerator would be real, and the configured `toggle_lock` chord is
/// already a global hotkey, so a menu key-equivalent would double-fire.
pub(crate) fn lock_item(
    app: &AppHandle,
    locked: bool,
    accel: Option<String>,
) -> tauri::Result<CheckMenuItem<Wry>> {
    CheckMenuItem::with_id(app, MENU_LOCK, "Lock Bar Position", true, locked, accel)
}

/// The `Position` submenu shared by the shared menu and the app
/// menubar's `View` — edge items are checks against the live `edge`;
/// `Center` is a plain action (it's a point, not an edge). The menubar
/// copy is built once and kept fresh by
/// `menubar::sync_position_checks`, not rebuilt like the tray's.
/// The edge a `menu.pos.*` id snaps to — `None` for `menu.pos.center`
/// and every non-position id.
pub(crate) fn pos_edge_dir(id: &str) -> Option<Dir> {
    POS_EDGES
        .iter()
        .find_map(|&(item_id, _, dir)| (item_id == id).then_some(dir))
}

pub(crate) fn position_submenu(app: &AppHandle, edge: Dir) -> tauri::Result<Submenu<Wry>> {
    let mut edge_items: Vec<CheckMenuItem<Wry>> = Vec::new();
    for &(id, label, dir) in POS_EDGES.iter() {
        edge_items.push(CheckMenuItem::with_id(
            app,
            id,
            label,
            true,
            edge == dir,
            None::<&str>,
        )?);
    }
    let pos_sep = PredefinedMenuItem::separator(app)?;
    let center = MenuItem::with_id(app, MENU_POS_CENTER, "Center", true, None::<&str>)?;
    let position = Submenu::with_id(app, MENU_POSITION, "Position", true)?;
    {
        let mut items: Vec<&dyn IsMenuItem<Wry>> = edge_items
            .iter()
            .map(|i| i as &dyn IsMenuItem<Wry>)
            .collect();
        items.push(&pos_sep);
        items.push(&center);
        position.append_items(&items)?;
    }
    Ok(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_item_id_strips_the_prefix() {
        assert_eq!(preset_item_id("preset.b:concise"), Some("b:concise"));
        assert_eq!(preset_item_id("menu.ask"), None);
        assert_eq!(preset_item_id("preset."), Some(""));
    }
}
