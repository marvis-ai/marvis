use crate::*;

/// Dev-only inspector for webviews without the shared menu (prefs,
/// alert): opens the CALLING window's devtools — their right-click
/// invokes this under `import.meta.env.DEV`. No-op in release, where
/// the frontend never calls it anyway.
#[tauri::command]
#[allow(unused_variables)]
pub(crate) fn open_devtools(webview: tauri::WebviewWindow) {
    #[cfg(debug_assertions)]
    webview.open_devtools();
}

// ---------------------------------------------------------------------------
// Commands — permissions / capture
// ---------------------------------------------------------------------------

/// `{"screen": bool, "mic": "notDetermined"|"restricted"|"denied"|"authorized"}`.
#[tauri::command]
pub(crate) fn permissions_status() -> serde_json::Value {
    json!({
        "screen": permissions::screen_status(),
        "mic": permissions::mic_status(),
    })
}

/// `CGRequestScreenCaptureAccess` may show the system prompt, so it runs
/// on a blocking thread; the gate is re-evaluated afterwards.
#[tauri::command]
pub(crate) async fn permissions_request_screen(app: AppHandle) -> bool {
    let granted = tauri::async_runtime::spawn_blocking(permissions::screen_request)
        .await
        .unwrap_or(false);
    // `transition_gate` → `enter_main`/`leave_main` is window work —
    // this command resumes on a tokio worker after the await, so hop
    // to the main thread (the pool's getters park the caller on the
    // main queue, which is the ABBA deadlock the window-event
    // `try_lock`s exist to avoid).
    let app2 = app.clone();
    if let Err(e) = app.run_on_main_thread(move || transition_gate(&app2)) {
        log::warn!("permissions_request_screen: gate transition hop failed: {e}");
    }
    granted
}

/// `AVCaptureDevice.requestAccess` MUST NOT run on the main thread — its
/// completion can dispatch to the main queue and deadlock a blocked main
/// thread — so the call is explicitly `spawn_blocking`'d.
#[tauri::command]
pub(crate) async fn permissions_request_mic(app: AppHandle) -> bool {
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
pub(crate) fn permissions_open_prefs(section: String) -> Result<(), String> {
    permissions::open_prefs(&section).map_err(|e| e.to_string())
}

