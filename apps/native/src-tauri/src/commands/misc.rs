use crate::*;

/// `"glass" | "vibrancy" | "none"` — which native material backs the
/// overlay windows. macOS 26+ reports glass; older macOS gets the
/// plugin's NSVisualEffectView fallback; other OSes get none (the
/// webview keeps its CSS frost).
#[tauri::command]
pub(crate) fn surface_material(app: AppHandle) -> &'static str {
    #[cfg(target_os = "macos")]
    {
        if app.liquid_glass().is_supported() {
            "glass"
        } else {
            "vibrancy"
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        "none"
    }
}

/// App-exit teardown for every speech session — meeting Listen and Ask
/// dictation share the microphone, so quitting must release both.
pub(crate) fn stop_speech(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.listen.stop();
    let _ = state.dictation.stop();
}

#[tauri::command]
pub(crate) fn quit_application(app: AppHandle) {
    stop_speech(&app);
    stop_capture(&app);
    app.exit(0);
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

