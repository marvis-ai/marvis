use super::*;

/// Shared builder flags for every Marvis overlay window (spec): frameless,
/// transparent, always-on-top, non-resizable, skip-taskbar, no shadow —
/// then `set_visible_on_all_workspaces` and the liquid-glass material.
/// `corner_radius` matches the surface's CSS radius —
/// the glass view fills the window, so its shape IS the surface shape.
/// `app.accent` (`#rrggbb`) at 8% alpha (`{accent}15`) → the bar's glass
/// `tint_color`.
/// The alpha is load-bearing: the pre-26 `NSVisualEffectView` fallback
/// paints the tint as an overlay fill, so an opaque value would bury
/// the vibrancy entirely — and even on glass, a stronger tint reads
/// heavy over the capsule.
pub(super) fn accent_glass_tint(accent: &str) -> Option<String> {
    let accent = accent.trim();
    (accent.len() == 7 && accent.starts_with('#')).then(|| format!("{accent}15"))
}

/// Order `win` front WITHOUT taking key status — `show()` calls
/// `makeKeyAndOrderFront` on macOS, which would steal the composer's
/// focus mid-`/` typing. `orderFront:` surfaces the palette within the
/// app's window level, unfocused. Other platforms fall back to
/// `show()` — if it activates, the palette degrades to focused-mode
/// (its own keydown + click-away still work).
#[cfg(target_os = "macos")]
pub(super) fn order_front_unfocused(win: &WebviewWindow) {
    use objc2_app_kit::NSWindow;
    match win.ns_window() {
        // `orderFront:` is a main-thread-only AppKit write — off the main
        // thread the window manager asserts ("Must only be used from the
        // main thread", SIGTRAP). Hop: inline on the main thread, queued
        // otherwise — the call stays safe wherever a caller lands.
        Ok(ptr) => {
            let ptr = ptr as usize;
            if let Err(e) = win.run_on_main_thread(move || unsafe {
                // SAFETY: tauri hands us the live NSWindow for `win`; the
                // pool owns the window, so it outlives this dispatch.
                let ns_win = &*(ptr as *const NSWindow);
                ns_win.orderFront(None);
            }) {
                log::warn!("windows: palette orderFront hop failed ({e}) — falling back to show");
                let _ = win.show();
            }
        }
        Err(e) => {
            log::warn!("windows: palette orderFront failed ({e}) — falling back to show");
            let _ = win.show();
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn order_front_unfocused(win: &WebviewWindow) {
    let _ = win.show();
}

/// Build a hidden overlay for `index.html?view={label}`, sized in logical
/// pixels by `w` and `h`. On macOS, `corner_radius` and `tint_color` configure
/// the glass surface. On Windows, request capture protection when a Marvis
/// capture guard is held. Window creation errors propagate; workspace,
/// capture-protection, and material failures do not prevent returning the window.
pub(super) fn build_window(
    app: &AppHandle,
    label: &str,
    w: f64,
    h: f64,
    corner_radius: f64,
    tint_color: Option<String>,
    accept_first_mouse: bool,
) -> anyhow::Result<WebviewWindow> {
    let url = WebviewUrl::App(format!("index.html?view={label}").into());
    let win = WebviewWindowBuilder::new(app, label, url)
        .inner_size(w, h)
        .accept_first_mouse(accept_first_mouse)
        // Pin user-resize to the built size: Tahoe edge-drags borderless
        // windows even with `resizable(false)`, and min == max drops the
        // affordance/cursor. Programmatic `set_size` is not limited, so
        // the bar's morphs still animate — `sync_bar_size_limits`
        // re-pins to each canonical width afterward.
        .min_inner_size(w, h)
        .max_inner_size(w, h)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .shadow(false)
        .visible(false)
        .build()?;
    if let Err(e) = win.set_visible_on_all_workspaces(true) {
        log::warn!("windows: set_visible_on_all_workspaces failed for {label}: {e}");
    }
    // WGC can't exclude windows from a monitor grab, so Windows applies
    // display affinity dynamically: `capture::windows` protects our
    // HWNDs only while a capture session is live, and this flag read
    // makes a window born mid-capture start protected. The rest of the
    // time the user CAN screenshot/record Marvis — the macOS asymmetry
    // (visible to the user, invisible to our own captures) without
    // SCContentFilter. macOS itself needs nothing here (its filters do
    // the exclusion) and Linux's tao backend no-ops the call anyway —
    // the portal can't self-exclude either.
    #[cfg(target_os = "windows")]
    if let Err(e) = win.set_content_protected(crate::capture::protection_engaged()) {
        log::warn!("windows: set_content_protected failed for {label}: {e}");
    }
    apply_surface_material(app, &win, corner_radius, tint_color, label);
    Ok(win)
}

/// Surface material application. On macOS this is the liquid-glass
/// `set_effect`, which dispatches to the main queue and blocks on it;
/// callers hold the pool lock, which main-thread callbacks also take —
/// apply on a detached thread so the lock is never held across the
/// wait. Warn-only on failure, same as the calls above. Off macOS the
/// webview's own CSS frost/radius draws the surface, so this is a no-op.
pub(super) fn apply_surface_material(
    app: &AppHandle,
    win: &WebviewWindow,
    corner_radius: f64,
    tint_color: Option<String>,
    label: &str,
) {
    #[cfg(target_os = "macos")]
    std::thread::spawn({
        let app = app.clone();
        let win = win.clone();
        let label = label.to_string();
        move || {
            if let Err(e) = app.liquid_glass().set_effect(
                &win,
                LiquidGlassConfig {
                    corner_radius,
                    tint_color,
                    ..Default::default()
                },
            ) {
                log::warn!("windows: liquid glass failed for {label}: {e}");
            }
        }
    });
    #[cfg(not(target_os = "macos"))]
    let _ = (app, win, corner_radius, tint_color, label);
}

/// Re-apply the liquid-glass corner radius on a live window — capsule
/// (`BAR_H/2`) ⇄ card (`CARD_RADIUS`). Shares `apply_surface_material`'s
/// warn-only detached-thread pattern; a no-op off macOS.
pub(super) fn set_glass_radius(app: &AppHandle, win: &WebviewWindow, corner_radius: f64) {
    // Re-supply the accent tint — `set_effect` CLEARS the tint when
    // `tint_color` is `None`, so a radius-only re-apply would strip
    // it on every pill⇄card morph.
    let tint_color = accent_glass_tint(&app.state::<crate::AppState>().accent());
    apply_surface_material(app, win, corner_radius, tint_color, "bar");
}

/// The prefs window is deliberately NOT built by [`build_window`]: it's a
/// real macOS window, not overlay chrome — native decorations, normal
/// focus — but it IS a liquid-glass surface: `TitleBarStyle::Overlay` +
/// `hidden_title` make the content full-size so the traffic lights
/// land inside the sidebar (the Settings.app look) and
/// `GlassMaterialVariant::AbuttedSidebar` paints the whole window in
/// the edge-abutting sidebar material — the same variant Settings.app
/// uses. It joins the bar's floating
/// level ONLY while focused (so it can overlap the bar the user keeps
/// on top) and drops back on blur — focus events keep `always_on_top`
/// mirroring the window's active state. It follows `build_window`'s
/// protection rule (Windows: display affinity only while a Marvis
/// capture is live) and joinable on all workspaces so it can be
/// summoned over any space.
/// `CloseRequested` is intercepted into a hide: the window is owned by
/// the pool for the app's lifetime, so the red light must not destroy
/// the webview (a fresh build would lose scroll/tab state).
/// Returns the window hidden. Window creation errors propagate; workspace,
/// capture-protection, and material failures do not prevent returning the window.
pub(super) fn build_prefs_window(app: &AppHandle) -> anyhow::Result<WebviewWindow> {
    let url = WebviewUrl::App(format!("index.html?view={PREFS_LABEL}").into());
    let builder = WebviewWindowBuilder::new(app, PREFS_LABEL, url)
        .inner_size(PREFS_W, PREFS_H)
        .title("Marvis — Settings")
        .decorations(true);
    // Overlay titlebar + hidden title are macOS-only builder methods —
    // they put the traffic lights inside the sidebar surface; other
    // platforms keep native decorations (the builder above).
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    let win = builder
        .transparent(true)
        .resizable(false)
        .visible(false)
        // Tauri's file-drop handler swallows element drags inside the
        // webview — the Providers list's HTML5 reorder needs the native
        // path. Marvis consumes no OS file drops, so nothing is lost.
        .disable_drag_drop_handler()
        .build()?;
    {
        let handle = win.clone();
        let app = app.clone();
        // `match event.clone()` binds arm fields by value, so `focused`
        // is `bool` regardless of the handler's `&WindowEvent` signature.
        win.on_window_event(move |event| match event.clone() {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = handle.set_always_on_top(false);
                let _ = handle.hide();
                // Closing a wizard re-exposes the bar if onboarding is
                // already done (a re-run); on a true first run the flag
                // is still false and the bar correctly stays hidden.
                app.state::<crate::AppState>().sync_bar_visibility();
            }
            // Float only while active: focused prefs may overlap the
            // always-on-top bar; on blur it drops to the normal level.
            tauri::WindowEvent::Focused(focused) => {
                let _ = handle.set_always_on_top(focused);
            }
            _ => {}
        });
    }
    if let Err(e) = win.set_visible_on_all_workspaces(true) {
        log::warn!("windows: set_visible_on_all_workspaces failed for prefs: {e}");
    }
    #[cfg(target_os = "windows")]
    if let Err(e) = win.set_content_protected(crate::capture::protection_engaged()) {
        log::warn!("windows: set_content_protected failed for prefs: {e}");
    }
    // Same detached warn-only pattern as `apply_surface_material`; the
    // sidebar variant is macOS-only chrome — other platforms keep the
    // webview's own surface.
    #[cfg(target_os = "macos")]
    std::thread::spawn({
        let app = app.clone();
        let win = win.clone();
        move || {
            if let Err(e) = app.liquid_glass().set_effect(
                &win,
                LiquidGlassConfig {
                    corner_radius: 0.0,
                    tint_color: None,
                    variant: GlassMaterialVariant::AbuttedSidebar,
                    ..Default::default()
                },
            ) {
                log::warn!("windows: liquid glass failed for prefs: {e}");
            }
        }
    });
    Ok(win)
}

