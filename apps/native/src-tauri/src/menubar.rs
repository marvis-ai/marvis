//! `menubar.rs` — the application menubar (`About`/`Settings`/`View`
//! surfaces, distinct from the tray + popup menu in `menus.rs`).
//!
//! On macOS [`build`] produces the whole app row — `Marvis` (About,
//! Settings, Services, Hide, Quit), `Edit`, `View` (Position, Lock Bar
//! Position), `Window`, `Help` (Email Support) — installed
//! via `app.set_menu` in `run`'s `setup`, replacing tauri's generated
//! default. Windows and Linux have no app menubar: the same builder's
//! reduced `File`/`View`/`Help` menu is attached to the prefs window in
//! `windows/mod.rs` (the app's one decorated surface — the borderless
//! bar/alert/picker must never grow a menubar strip).
//!
//! Items reuse the `menu.*` ids from `menus.rs`, so the single global
//! `menu_dispatch` handles them identically wherever they live. Unlike
//! the tray copy the menubar persists (built once, never rebuilt), so
//! [`sync_position_checks`] pokes its `menu.pos.*` checks in place from
//! every state-change funnel.

use tauri::menu::{
    AboutMetadata, IsMenuItem, Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu,
};
use tauri::{AppHandle, Manager, Wry};

use crate::windows::{self, Dir};
use crate::{menus, AppState};

/// The About panel's `(build)` half of `version (build)` — the git
/// commit count baked in by build.rs; absent for non-git builds.
const BUILD_NUMBER: Option<&'static str> = option_env!("MARVIS_BUILD_NUMBER");

/// `View` on macOS carries a trailing zero-width space: AppKit's
/// View-menu heuristic is a literal title match, and a submenu titled
/// exactly "View" without a `toggleFullScreen:` item gets an injected
/// "Enter Full Screen" — created by the OS, so no menu API can remove
/// it. The ZWSP renders identically but fails the `isEqualToString:`
/// check, so nothing is injected. Other platforms keep the plain title.
#[cfg(target_os = "macos")]
const VIEW_TITLE: &str = "View\u{200B}";
#[cfg(not(target_os = "macos"))]
const VIEW_TITLE: &str = "View";

/// Support address opened by the Help menu's `menu.support` item —
/// `mailto:` via the opener plugin, so it lands in the user's default
/// mail client.
pub(crate) const SUPPORT_MAILTO: &str = "mailto:support@getmarvis.com";

/// About metadata shared by the macOS App-menu item and the
/// Windows/Linux Help-menu one: `version (build)` + copyright.
fn about_metadata(app: &AppHandle) -> AboutMetadata<'static> {
    let pkg = app.package_info();
    AboutMetadata {
        name: Some(pkg.name.clone()),
        version: Some(pkg.version.to_string()),
        short_version: BUILD_NUMBER.map(str::to_string),
        copyright: Some("© 2026 Marvis AI LLC".to_string()),
        ..Default::default()
    }
}

/// Build the platform menubar against the live `edge` (the Position
/// checks' initial state).
pub fn build(app: &AppHandle, edge: Dir) -> tauri::Result<Menu<Wry>> {
    let name = app.package_info().name.clone();
    // Read before any item creation — `with_id`/`with_items` can block
    // on the main thread, and the non-macOS call site (`show_prefs`)
    // already holds `pool`, so the config guard must not live across
    // them. (`pool → config` here is the only direction either lock is
    // nested — `menus::build` takes them strictly sequentially.)
    let locked = app.state::<AppState>().config.lock().window.bar_locked;
    let about = PredefinedMenuItem::about(
        app,
        Some(&format!("About {name}")),
        Some(about_metadata(app)),
    )?;
    // Unlike the tray item's display-only label this accelerator is
    // real — the menubar owns `Cmd/Ctrl+,` wherever focus is.
    let settings = MenuItem::with_id(
        app,
        menus::MENU_SETTINGS,
        "Settings…",
        true,
        Some("CmdOrCtrl+,"),
    )?;
    let position = menus::position_submenu(app, edge)?;
    // Menubar accelerators are real key-equivalents, unlike the tray's
    // display-only labels — `None` so the global `toggle_lock` hotkey
    // stays the single binding and the chord can't double-fire.
    let lock = menus::lock_item(app, locked, None)?;
    let view = Submenu::with_items(
        app,
        VIEW_TITLE,
        true,
        &[
            &position as &dyn IsMenuItem<Wry>,
            &PredefinedMenuItem::separator(app)?,
            &lock,
        ],
    )?;
    let support = MenuItem::with_id(
        app,
        menus::MENU_SUPPORT,
        "Email Support…",
        true,
        None::<&str>,
    )?;

    #[cfg(target_os = "macos")]
    {
        let app_menu = Submenu::with_items(
            app,
            &name,
            true,
            &[
                &about as &dyn IsMenuItem<Wry>,
                &PredefinedMenuItem::separator(app)?,
                &settings,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::services(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, Some(&format!("Hide {name}")))?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::show_all(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::quit(app, Some(&format!("Quit {name}")))?,
            ],
        )?;
        let edit = Submenu::with_items(
            app,
            "Edit",
            true,
            &[
                &PredefinedMenuItem::undo(app, None)? as &dyn IsMenuItem<Wry>,
                &PredefinedMenuItem::redo(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::cut(app, None)?,
                &PredefinedMenuItem::copy(app, None)?,
                &PredefinedMenuItem::paste(app, None)?,
                &PredefinedMenuItem::select_all(app, None)?,
            ],
        )?;
        // WINDOW_SUBMENU_ID registers this as the app's window menu, so
        // macOS appends the live window list itself; HELP_SUBMENU_ID
        // likewise marks the trailing menu as the app Help menu, which
        // adds the standard Help search field.
        let window = Submenu::with_id_and_items(
            app,
            tauri::menu::WINDOW_SUBMENU_ID,
            "Window",
            true,
            &[
                &PredefinedMenuItem::minimize(app, None)? as &dyn IsMenuItem<Wry>,
                &PredefinedMenuItem::maximize(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::close_window(app, None)?,
            ],
        )?;
        let help = Submenu::with_id_and_items(
            app,
            tauri::menu::HELP_SUBMENU_ID,
            "Help",
            true,
            &[&support as &dyn IsMenuItem<Wry>],
        )?;
        Menu::with_items(
            app,
            &[&app_menu, &edit, &view, &window, &help].map(|s| s as &dyn IsMenuItem<Wry>),
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Predefined quit/hide/etc. are silently dropped on GTK, so the
        // non-macOS menu keeps to custom items + the supported
        // predefined set (About, separators).
        let quit = MenuItem::with_id(
            app,
            menus::MENU_QUIT,
            &format!("Quit {name}"),
            true,
            None::<&str>,
        )?;
        let file = Submenu::with_items(
            app,
            "File",
            true,
            &[
                &settings as &dyn IsMenuItem<Wry>,
                &PredefinedMenuItem::separator(app)?,
                &quit,
            ],
        )?;
        let help = Submenu::with_items(
            app,
            "Help",
            true,
            &[
                &support as &dyn IsMenuItem<Wry>,
                &PredefinedMenuItem::separator(app)?,
                &about,
            ],
        )?;
        Menu::with_items(
            app,
            &[&file, &view, &help].map(|s| s as &dyn IsMenuItem<Wry>),
        )
    }
}

/// Mirror the live edge onto the persistent menubar's Position checks —
/// the tray copy is rebuilt per state change, the menubar is not, so
/// its `menu.pos.*` items must be poked in place. Covers both homes:
/// `app.menu()` (the macOS app row) and the prefs window's menu
/// (Windows/Linux). Reads the edge itself; callers must not hold
/// `pool`/`config`/`capture` locks — `set_checked` can block on the
/// main thread, the same rule as `refresh_tray_menu`.
pub(crate) fn sync_position_checks(app: &AppHandle) {
    let state = app.state::<AppState>();
    // `refresh_bar_rect` before `bar_edge` — same as `menus::build`; the
    // committed rect can trail the window's real `Moved` frame.
    let edge = {
        let mut pool = state.pool.lock();
        pool.refresh_bar_rect();
        pool.bar_edge()
    };
    let locked = state.config.lock().window.bar_locked;
    for menu in [
        app.menu(),
        app.get_webview_window(windows::PREFS_LABEL)
            .and_then(|win| win.menu()),
    ]
    .into_iter()
    .flatten()
    {
        let Ok(items) = menu.items() else { continue };
        sync_checks(&items, edge, locked);
    }
}

/// Recursively `set_checked` on every stateful check item in `items` —
/// the `menu.pos.*` edge items and the `menu.lock` item the menubar's
/// `View` carries.
fn sync_checks(items: &[MenuItemKind<Wry>], edge: Dir, locked: bool) {
    for kind in items {
        match kind {
            MenuItemKind::Submenu(submenu) => {
                if let Ok(children) = submenu.items() {
                    sync_checks(&children, edge, locked);
                }
            }
            MenuItemKind::Check(check) => {
                let checked = match check.id().as_ref() {
                    menus::MENU_POS_TOP => Some(edge == Dir::Up),
                    menus::MENU_POS_BOTTOM => Some(edge == Dir::Down),
                    menus::MENU_POS_LEFT => Some(edge == Dir::Left),
                    menus::MENU_POS_RIGHT => Some(edge == Dir::Right),
                    menus::MENU_LOCK => Some(locked),
                    _ => None,
                };
                if let Some(checked) = checked {
                    let _ = check.set_checked(checked);
                }
            }
            _ => {}
        }
    }
}
