use crate::*;

#[tauri::command]
pub(crate) fn voice_models_catalog(state: State<'_, AppState>) -> Vec<voice_models::VoiceModelCatalogPayload> {
    state.voice_models.catalog_payload()
}

#[tauri::command]
pub(crate) fn whisper_status(app: AppHandle) -> voice_models::WhisperDownloadStatus {
    let state = app.state::<AppState>();
    state.voice_models.status(state.bundled_whisper.as_deref())
}

pub(crate) fn safe_voice_error(error: voice_models::VoiceDownloadError) -> String {
    match error {
        voice_models::VoiceDownloadError::Busy => "A voice model download is already active".into(),
        voice_models::VoiceDownloadError::UnknownModel(_) => "Unknown voice model".into(),
        voice_models::VoiceDownloadError::ActiveModel => {
            "Cannot remove the selected voice model".into()
        }
        voice_models::VoiceDownloadError::Cancelled => "Download cancelled".into(),
        voice_models::VoiceDownloadError::Verification => {
            "Downloaded model verification failed".into()
        }
        voice_models::VoiceDownloadError::Download(_) => "Voice model download failed".into(),
    }
}

#[tauri::command]
pub(crate) fn whisper_download(state: State<'_, AppState>, model: String) -> Result<(), String> {
    let entry =
        voice_models::entry_for_id(&model).ok_or_else(|| "Unknown voice model".to_string())?;
    state
        .voice_models
        .start_download(entry.id)
        .map_err(safe_voice_error)
}

#[tauri::command]
pub(crate) async fn whisper_cancel_download(state: State<'_, AppState>) -> Result<(), String> {
    state
        .voice_models
        .cancel_download()
        .await
        .map_err(safe_voice_error)
}

#[tauri::command]
pub(crate) fn whisper_remove_model(
    state: State<'_, AppState>,
    model: String,
) -> Result<voice_models::WhisperDownloadStatus, String> {
    let entry =
        voice_models::entry_for_id(&model).ok_or_else(|| "Unknown voice model".to_string())?;
    let selected = {
        let config = state.config.lock();
        if config.models.stt_provider == "whisper" {
            voice_models::entry_for_id(&config.models.stt_model).map(|entry| entry.id)
        } else {
            None
        }
    };
    state.voice_models.set_selected_model(selected);
    state
        .voice_models
        .remove_model(entry.id)
        .map_err(safe_voice_error)?;
    Ok(state.voice_models.status(state.bundled_whisper.as_deref()))
}

#[tauri::command]
pub(crate) fn sherpa_status(state: State<'_, AppState>) -> sherpa_models::SherpaStatus {
    state.sherpa_models.status()
}

#[tauri::command]
pub(crate) fn sherpa_download(state: State<'_, AppState>, model: String) -> Result<(), String> {
    let entry =
        sherpa_models::entry_for_id(&model).ok_or_else(|| "Unknown voice model".to_string())?;
    state
        .sherpa_models
        .start_download(entry.id)
        .map_err(safe_voice_error)
}

#[tauri::command]
pub(crate) async fn sherpa_cancel_download(state: State<'_, AppState>) -> Result<(), String> {
    state
        .sherpa_models
        .cancel_download()
        .await
        .map_err(safe_voice_error)
}

#[tauri::command]
pub(crate) fn sherpa_remove_model(
    state: State<'_, AppState>,
    model: String,
) -> Result<sherpa_models::SherpaStatus, String> {
    let entry =
        sherpa_models::entry_for_id(&model).ok_or_else(|| "Unknown voice model".to_string())?;
    let selected = {
        let config = state.config.lock();
        if config.models.stt_provider == "sherpa" {
            sherpa_models::stt_entry_for_value(&config.models.stt_model).map(|e| e.id)
        } else {
            None
        }
    };
    state.sherpa_models.set_selected_model(selected);
    state
        .sherpa_models
        .remove_model(entry.id)
        .map_err(safe_voice_error)?;
    Ok(state.sherpa_models.status())
}

// ---------------------------------------------------------------------------
// Commands — voice enrollment
// ---------------------------------------------------------------------------

