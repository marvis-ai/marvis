use crate::*;

/// Tray Toggle's behaviour as a command: collapse/expand the unified
/// card. (The global hotkey no longer routes here — it toggles only the
/// bar's input pill via `bar:toggle-input`.)
#[tauri::command]
pub(crate) fn window_toggle_all(app: AppHandle) {
    app.state::<AppState>().pool.lock().toggle_chat(&app);
}

/// Direct card open/close — the mic button's listen mode and the
/// `capture:permission-needed` collapse use it (toggle semantics would
/// close an open card when the user only wants to switch modes, and
/// `ask_close` would cancel an in-flight text-only ask).
#[tauri::command]
pub(crate) fn window_set_chat_open(app: AppHandle, open: bool) {
    app.state::<AppState>()
        .pool
        .lock()
        .set_chat_open(&app, open);
}

/// Focus the bar window — the `bar:toggle-input` show path needs it so
/// a global-hotkey reveal lands the user's typing in the field.
#[tauri::command]
pub(crate) fn window_focus_bar(app: AppHandle) {
    let state = app.state::<AppState>();
    // Clone the handle out so the pool guard drops before `set_focus`.
    let bar = state.pool.lock().bar().cloned();
    if let Some(bar) = bar {
        let _ = bar.set_focus();
    }
}

/// Same entry point as the bar's `Cmd+,` and the tray's Settings item.
/// Main-thread command: `show_prefs` is window work end to end — its
/// getters (`is_visible` in `set_bar_shown`) park the caller on the
/// main queue, which deadlocks if a worker arrives holding `pool`
/// while main waits on it. Never mark `(async)`.
#[tauri::command]
pub(crate) fn window_show_settings(app: AppHandle) {
    show_settings(&app);
}

/// Onboarding mode of the same prefs window — the startup first-run
/// opener and the sidebar's "Re-run setup" both come through here.
/// Main-thread command for the same reason as `window_show_settings`.
#[tauri::command]
pub(crate) fn window_show_onboarding(app: AppHandle) {
    app.state::<AppState>()
        .pool
        .lock()
        .show_prefs(&app, "onboarding");
}

/// Hide the prefs window, then reconcile the bar: if the hidden mode
/// was onboarding and the wizard is done, the bar floats again.
#[tauri::command]
pub(crate) fn window_hide_prefs(state: State<'_, AppState>) {
    state.pool.lock().hide_prefs();
    state.sync_bar_visibility();
}

/// The mode the prefs window was last shown in (`"settings"` |
/// `"onboarding"`, `""` before first use) — read on mount so a
/// `prefs:mode` emit that raced the loading webview still lands.
#[tauri::command]
pub(crate) fn prefs_mode(state: State<'_, AppState>) -> String {
    state.pool.lock().prefs_mode().to_string()
}

// ---------------------------------------------------------------------------
// Commands — alert toast
// ---------------------------------------------------------------------------

/// Raise the alert toast — the webview's only error surface (the bar
/// pill has no room to render one).
#[tauri::command]
pub(crate) fn alert_show(app: AppHandle, message: String) {
    show_alert(&app, &message);
}

/// The live alert payload, or `null` — read by the toast on mount so a
/// show that raced its listener still renders.
#[tauri::command]
pub(crate) fn alert_current(state: State<'_, AppState>) -> serde_json::Value {
    state
        .alert
        .lock()
        .clone()
        .unwrap_or(serde_json::Value::Null)
}

#[tauri::command]
pub(crate) fn alert_dismiss(state: State<'_, AppState>) {
    *state.alert.lock() = None;
    state.pool.lock().hide_alert();
}

/// `height` is the desired TOTAL window height (the frontend measures
/// the whole card) — the pool clamps and animates, anchored edge fixed.
/// Expanded-only.
#[tauri::command]
pub(crate) fn window_adjust_height(state: State<'_, AppState>, height: f64) {
    state.pool.lock().adjust_height(height);
}

/// The webview's pill⇄input morph signal — under liquid glass the
/// capsule IS the window, so the window resizes to match (idle 172,
/// expanded 600, same 64 height and capsule radius).
#[tauri::command]
pub(crate) fn window_set_bar_expanded(state: State<'_, AppState>, expanded: bool) {
    state.pool.lock().set_bar_expanded(expanded);
}

/// Settings → Bar picker: `edge` is `"top"|"bottom"|"left"|"right"`.
/// Snaps (animated) the bar to that work-area edge; the resulting `Moved`
/// event persists `window.bar_x/y` through the debounced write.
#[tauri::command]
pub(crate) fn window_snap_edge(app: AppHandle, edge: String) -> Result<(), String> {
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
pub(crate) fn window_recenter(app: AppHandle) {
    app.state::<AppState>().pool.lock().recenter_bar();
    refresh_tray_menu(&app);
}

/// The edge the bar is currently nearest (`"top"` | `"bottom"` |
/// `"left"` | `"right"`) — the Bar picker's selected value. Recomputed
/// from the live rect so a just-finished drag reads correctly.
#[tauri::command]
pub(crate) fn window_bar_edge(state: State<'_, AppState>) -> String {
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
pub(crate) fn bar_context_menu(app: AppHandle) -> Result<(), String> {
    let menu = menus::build(&app).map_err(|e| e.to_string())?;
    let bar = app
        .get_webview_window(windows::BAR_LABEL)
        .ok_or("bar window missing")?;
    bar.popup_menu(&menu).map_err(|e| e.to_string())
}

