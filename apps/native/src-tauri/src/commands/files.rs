use crate::*;

#[tauri::command]
pub(crate) async fn save_audio_file(
    app: AppHandle,
    session_id: i64,
    suggested_name: String,
) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("save_audio_file dropped while gate != Main");
        return Ok(None);
    }
    let source = state
        .db
        .session_audio_file(session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No audio recording for this session".to_string())?;
    let source = std::path::PathBuf::from(source);
    let audios = crate::paths::audios_dir();
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let audios = audios.canonicalize().map_err(|e| e.to_string())?;
    if !source.starts_with(&audios) {
        return Err("Audio recording is outside the Marvis audio directory".into());
    }
    if std::fs::metadata(&source).map_err(|e| e.to_string())?.len() <= 44 {
        return Err("This session has no recorded audio".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(sanitize_suggested_name(&suggested_name))
            .add_filter("WAV audio", &["wav"])
            .save_file()
        else {
            return Ok(None);
        };
        copy_audio_export(&source, &path)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) fn copy_audio_export(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), String> {
    match destination.canonicalize() {
        Ok(existing) if existing == source => {
            return Err("Choose a different destination from the original recording".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Could not check the audio export destination".into()),
    }
    std::fs::copy(source, destination)
        .map_err(|_| "Could not export audio recording".to_string())?;
    Ok(())
}

/// Document egress (export): a native save dialog + write. Deliberately
/// dumb — the webview builds the document; this owns the two things a
/// webview can't do without an fs capability. `None` = user cancel
/// (also the gate-drop result: a crafted invoke gets the same nothing).
#[tauri::command]
pub(crate) async fn save_text_file(
    app: AppHandle,
    suggested_name: String,
    contents: String,
) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("save_text_file dropped while gate != Main");
        return Ok(None);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(sanitize_suggested_name(&suggested_name))
            .add_filter("Markdown", &["md"])
            .save_file()
        else {
            return Ok(None);
        };
        std::fs::write(&path, contents).map_err(|e| e.to_string())?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The dialog's suggested name: path separators and control chars can't
/// smuggle a directory choice past the picker; the cap keeps the dialog
/// field sane. Never a path — just a filename.
pub(crate) fn sanitize_suggested_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| !matches!(c, '/' | '\\') && !c.is_control())
        .take(80)
        .collect();
    let clean = clean.trim();
    if clean.is_empty() {
        "marvis-export.md".to_string()
    } else {
        clean.to_string()
    }
}

// ---------------------------------------------------------------------------
// Commands — config / app
// ---------------------------------------------------------------------------

