use crate::*;

#[tauri::command]
pub(crate) fn voiceprint_status(state: State<'_, AppState>) -> voiceprint::VoiceprintStatus {
    state.voice_enroll.status()
}

#[tauri::command]
pub(crate) fn voice_enroll_start(state: State<'_, AppState>) -> Result<(), String> {
    state.voice_enroll.start()
}

/// Stop returns the saved take's seconds so the UI can confirm "Ns saved".
#[tauri::command]
pub(crate) fn voice_enroll_stop(state: State<'_, AppState>) -> Result<voiceprint::VoiceEnrollResult, String> {
    state.voice_enroll.stop()
}

#[tauri::command]
pub(crate) fn voice_enroll_cancel(state: State<'_, AppState>) {
    state.voice_enroll.cancel();
}

#[tauri::command]
pub(crate) fn voiceprint_remove(state: State<'_, AppState>) -> Result<(), String> {
    state.voice_enroll.remove()
}

// ---------------------------------------------------------------------------
// Commands — windows
// ---------------------------------------------------------------------------

