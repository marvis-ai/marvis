use crate::*;

pub(crate) const EV_DICTATION_STATE: &str = "dictation:state";
pub(crate) const EV_DICTATION_DRAFT: &str = "dictation:draft";
pub(crate) const EV_DICTATION_ERROR: &str = "dictation:error";

/// `dictation:state` carries the whole durable status — `DictationStatus`'s
/// wire fields (`state`/`provider`/`error`) already are the payload.
pub(crate) fn emit_dictation_state(app: &AppHandle, status: &dictation::DictationStatus) {
    let _ = app.emit_to(windows::BAR_LABEL, EV_DICTATION_STATE, status);
}

pub(crate) fn emit_dictation_event(app: &AppHandle, event: DictationEvent) {
    match event {
        DictationEvent::Draft(draft) => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_DICTATION_DRAFT, draft);
        }
        DictationEvent::Error {
            message,
            needs_setup,
        } => {
            let _ = app.emit_to(
                windows::BAR_LABEL,
                EV_DICTATION_ERROR,
                json!({ "message": message, "needs_setup": needs_setup }),
            );
            // Re-emit the durable status snapshot so a bar that missed the
            // failure resynchronizes — same contract as `listen:error`.
            let status = app.state::<AppState>().dictation.status();
            emit_dictation_state(app, &status);
        }
    }
}

/// A durable setup error should live exactly as long as its cause. STT
/// config writes, Deepgram key changes, a mic grant, and completed model
/// downloads each re-run the speech services' pre-flight checks through
/// here — a resolved error resets to `idle` and a still-broken one
/// rewrites to the current reason, both via the usual `*:state` resync,
/// so no webview keeps showing a problem the user already fixed.
pub(crate) fn refresh_speech_setup(app: &AppHandle) {
    let state = app.state::<AppState>();
    let config = state.config.lock().clone();
    let keystore = state.keystore.lock().clone();
    let bundled = state.bundled_whisper.as_deref();
    let sherpa_root = paths::sherpa_models_dir();
    state
        .sherpa_models
        .ensure_punct(&config.models.stt_provider, &config.models.stt_model);
    if let Some(status) = state
        .listen
        .revalidate_setup(&keystore, &config, bundled, &sherpa_root)
    {
        emit_listen_state(app, &status);
    }
    let mic_allowed = permissions::mic_status() == permissions::PermissionState::Authorized;
    if let Some(status) =
        state
            .dictation
            .revalidate_setup(&keystore, &config, bundled, &sherpa_root, mic_allowed)
    {
        emit_dictation_state(app, &status);
    }
}

/// Mic-only dictation into the Ask input: requests mic permission, never
/// opens `SystemAudioSource`, and persists nothing. Mutually exclusive with
/// meeting Listen — the mic button normally stops the other mode first, so
/// a conflict here means a stale/racing invoke and must fail safely.
#[tauri::command]
pub(crate) async fn dictation_start(app: AppHandle) -> Result<dictation::DictationStatus, String> {
    let state = app.state::<AppState>();
    // Serialized with `listen_start` end-to-end: the peer-status check,
    // the mic-permission await, and the service start are one critical
    // section — two concurrent first-starts can no longer both see the
    // peer idle and double-open the microphone.
    let _lifecycle = state.speech_lifecycle.lock().await;
    if *state.gate.lock() != Gate::Main {
        return Err("Dictation is unavailable until setup is complete".into());
    }
    if state.listen.status().is_listening() {
        return Err("Stop listening before dictating".into());
    }
    let mic_allowed = tauri::async_runtime::spawn_blocking(permissions::mic_request)
        .await
        .unwrap_or(false);
    let config = state.config.lock().clone();
    let keystore = state.keystore.lock().clone();
    let app_for_emit = app.clone();
    let emit = Arc::new(move |event| emit_dictation_event(&app_for_emit, event));
    if let Err(error) = state.dictation.start(
        &keystore,
        &config,
        mic_allowed,
        state.bundled_whisper.as_deref(),
        emit,
    ) {
        // DictationService owns the event contract for failures it emits:
        // setup failures have already emitted needs_setup:true, so
        // re-emitting here would produce a contradictory second event.
        return Err(error.to_string());
    }
    // `leave_main` may have run while the session was being built —
    // dictation is bound to the (now hidden) ask input, so a start that
    // outlived `Main` stops itself instead of running invisibly.
    if *state.gate.lock() != Gate::Main {
        let _ = state.dictation.stop();
        emit_dictation_state(&app, &state.dictation.status());
        return Err("Dictation is unavailable until setup is complete".into());
    }
    let status = state.dictation.status();
    emit_dictation_state(&app, &status);
    Ok(status)
}

/// Stop dictation and return the authoritative final draft. Idempotent —
/// repeated calls return an empty final draft and leave the input alone.
#[tauri::command]
pub(crate) async fn dictation_stop(app: AppHandle) -> Result<dictation::DictationDraft, String> {
    let app_for_stop = app.clone();
    let draft = tauri::async_runtime::spawn_blocking(move || {
        app_for_stop.state::<AppState>().dictation.stop()
    })
    .await
    .map_err(|_| "Could not stop dictation".to_string())?;
    emit_dictation_state(&app, &app.state::<AppState>().dictation.status());
    Ok(draft)
}

/// Live dictation status for bar resync (`idle` | `listening` | `error`).
#[tauri::command]
pub(crate) fn dictation_status(app: AppHandle) -> dictation::DictationStatus {
    refresh_speech_setup(&app);
    app.state::<AppState>().dictation.status()
}

