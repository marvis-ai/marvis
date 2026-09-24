//! `tray.rs` — the menu-bar extra (macOS) / notification-area icon (Windows).
//!
//! Assets differ per platform: macOS renders a monochrome *template*
//! glyph (`icon_as_template` — AppKit auto-inverts it for the menu bar's
//! light/dark state) while Windows gets the full-colour mark packed as a
//! multi-size `.ico`. Both are embedded via `include_bytes!`, so no
//! `bundle.resources` entry is needed. Sources live in `assets/` at the
//! repo root (`marvis-mark-template.svg` is the template variant of
//! `marvis-mark.svg` — the same iris and text lines as a flat
//! silhouette; the mark's rounded square would template-render as a
//! solid blob).

use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::AppHandle;

/// `MenuItem` ids — matched by the dispatch closure in `lib.rs`.
pub const MENU_TOGGLE: &str = "tray.toggle";
pub const MENU_SETTINGS: &str = "tray.settings";
pub const MENU_QUIT: &str = "tray.quit";

#[cfg(target_os = "macos")]
const ICON_BYTES: &[u8] = include_bytes!("../icons/tray-macos.png");
#[cfg(not(target_os = "macos"))]
const ICON_BYTES: &[u8] = include_bytes!("../icons/tray-windows.ico");

/// Build the tray icon and its menu. `on_menu` is the dispatch closure
/// (same shape as `hotkey_dispatch`/`deeplink_dispatch`): it re-resolves
/// `AppState` per event so the handler survives gate transitions.
pub fn init(
    app: &AppHandle,
    on_menu: impl Fn(&AppHandle, MenuEvent) + Send + Sync + 'static,
) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, MENU_TOGGLE, "Show / Hide", true, None::<&str>)?;
    // The accelerator is display-only here (a tray menu isn't the app
    // menu, so AppKit never fires it) — `Cmd+,` is bound for real as a
    // keydown handler in the bar webview (fires only while it's active).
    let settings = MenuItem::with_id(app, MENU_SETTINGS, "Settings", true, Some("CmdOrCtrl+,"))?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit Marvis", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &settings, &quit])?;
    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(ICON_BYTES)?)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Marvis")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu)
        .build(app)?;
    Ok(())
}
