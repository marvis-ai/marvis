use crate::*;

pub(crate) const EV_LISTEN_STATE: &str = "listen:state";
pub(crate) const EV_LISTEN_TURN: &str = "listen:turn";
pub(crate) const EV_LISTEN_SUMMARY: &str = "listen:summary";
pub(crate) const EV_LISTEN_ERROR: &str = "listen:error";

pub(crate) fn emit_listen_state(app: &AppHandle, state: &listen::ListenStatus) {
    let _ = app.emit_to(
        windows::BAR_LABEL,
        EV_LISTEN_STATE,
        json!({
            "state": state.state,
            "provider": state.provider,
            "session_id": state.session_id,
            "audio_file": state.audio_file,
            "mic": state.mic,
            "error": state.error,
            "started_at": state.started_at,
            "paused_secs": state.paused_secs,
            "paused_since": state.paused_since,
        }),
    );
    // The shared menu's Start Listening item is disabled while live.
    refresh_tray_menu(app);
}

pub(crate) fn emit_listen_event(app: &AppHandle, event: ListenEvent) {
    match event {
        ListenEvent::Turn(turn) => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_LISTEN_TURN, turn);
        }
        ListenEvent::Summary(summary) => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_LISTEN_SUMMARY, summary);
        }
        ListenEvent::Error {
            message,
            needs_setup,
        } => {
            let _ = app.emit_to(
                windows::BAR_LABEL,
                EV_LISTEN_ERROR,
                json!({ "message": message, "needs_setup": needs_setup }),
            );
            let status = app.state::<AppState>().listen.status();
            // Re-emit the durable status snapshot so a bar opened after the
            // setup failure can resynchronize through the same payload as a
            // normal listen state update.
            emit_listen_state(app, &status);
        }
    }
}

#[tauri::command]
pub(crate) async fn listen_start(app: AppHandle) -> Result<listen::ListenStatus, String> {
    let state = app.state::<AppState>();
    // Serialized with `dictation_start` end-to-end: the peer-status
    // check, the mic-permission await, and the service start are one
    // critical section — two concurrent first-starts can no longer both
    // see the peer idle and double-open the microphone.
    let _lifecycle = state.speech_lifecycle.lock().await;
    if *state.gate.lock() != Gate::Main {
        return Err("Listen is unavailable until setup is complete".into());
    }
    // Mutual exclusion: a live dictation session owns the microphone —
    // a stale/racing `listen_start` must fail instead of stealing it.
    if state.dictation.status().is_listening() {
        return Err("Stop dictation before listening".into());
    }
    let mic_allowed = tauri::async_runtime::spawn_blocking(permissions::mic_request)
        .await
        .unwrap_or(false);
    let config = state.config.lock().clone();
    let keystore = state.keystore.lock().clone();
    let app_for_emit = app.clone();
    let emit = Arc::new(move |event| emit_listen_event(&app_for_emit, event));
    if let Err(error) = state.listen.start(
        Arc::clone(&state.db),
        &keystore,
        &config,
        mic_allowed,
        state.bundled_whisper.as_deref(),
        emit,
    ) {
        // ListenService owns the event contract for failures it emits. In
        // particular, setup failures have already emitted needs_setup:true;
        // re-emitting here would produce a contradictory second event.
        return Err(error.to_string());
    }
    let status = state.listen.status();
    emit_listen_state(&app, &status);
    Ok(status)
}

#[tauri::command]
pub(crate) async fn listen_stop(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _lifecycle = state.speech_lifecycle.lock().await;
    let app_for_stop = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app_for_stop.state::<AppState>().listen.stop();
    })
    .await
    .map_err(|_| "Could not stop listening".to_string())?;
    emit_listen_state(&app, &state.listen.status());
    Ok(())
}

#[tauri::command]
pub(crate) fn listen_pause(app: AppHandle) {
    let state = app.state::<AppState>();
    if let Some(status) = state.listen.pause() {
        emit_listen_state(&app, &status);
    }
}

#[tauri::command]
pub(crate) fn listen_resume(app: AppHandle) {
    let state = app.state::<AppState>();
    if let Some(status) = state.listen.resume() {
        emit_listen_state(&app, &status);
    }
}

#[tauri::command]
pub(crate) fn listen_status(app: AppHandle) -> listen::ListenStatus {
    // Self-healing read: a cause fixed outside the tracked triggers (e.g.
    // an externally installed whisper-cli) clears on the next resync.
    refresh_speech_setup(&app);
    app.state::<AppState>().listen.status()
}

