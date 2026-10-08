//! `tray.rs` — the menu-bar extra (macOS) / notification-area icon (Windows).
//!
//! Assets differ per platform: macOS renders a monochrome *template*
//! (`icon_as_template` — AppKit auto-inverts it for the menu bar's
//! light/dark state, so the mark must be an alpha knockout inside the
//! dark tile) while Windows/Linux render real colour, so they get the
//! same badge painted light-on-dark — a multi-size `.ico` / `.png`.
//! All are embedded via `include_bytes!`, so no `bundle.resources`
//! entry is needed. Sources live in `assets/` at the repo root
//! (`marvis-mark-template.svg` for macOS, `marvis-mark-tray.svg` for
//! Windows/Linux).
//!
//! The icon's menu is [`crate::menus::build`]'s output — the same items
//! the bar's idle-state right-click popup shows. `lib.rs` registers the
//! ONE global `on_menu_event` dispatcher; per-state refreshes go through
//! `refresh_tray_menu` (a `set_menu` swap on this icon's `"main"` id).

use tauri::image::Image;
use tauri::tray::TrayIconBuilder;
use tauri::AppHandle;

use crate::menus;

#[cfg(target_os = "macos")]
const ICON_BYTES: &[u8] = include_bytes!("../icons/tray-macos.png");
#[cfg(target_os = "windows")]
const ICON_BYTES: &[u8] = include_bytes!("../icons/tray-windows.ico");
#[cfg(target_os = "linux")]
const ICON_BYTES: &[u8] = include_bytes!("../icons/tray-linux.png");

/// Build the tray icon and attach the shared menu.
pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let menu = menus::build(app)?;
    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(ICON_BYTES)?)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Marvis")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .build(app)?;
    Ok(())
}
