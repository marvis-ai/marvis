use crate::*;

#[tauri::command]
pub(crate) fn session_list(state: State<'_, AppState>) -> Result<Vec<Session>, String> {
    state.db.session_list().map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn session_get(state: State<'_, AppState>, id: i64) -> Result<Vec<Message>, String> {
    state.db.messages_for(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn transcripts_for(
    state: State<'_, AppState>,
    id: i64,
    limit: Option<usize>,
) -> Result<Vec<Transcript>, String> {
    state
        .db
        .transcripts_for(id, limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn summary_latest(
    state: State<'_, AppState>,
    id: i64,
) -> Result<Option<Summary>, String> {
    state.db.summary_latest(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn session_delete(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let lifecycle = state.lifecycle.clone();
    let db = state.db.clone();
    lifecycle
        .delete(db.as_ref(), id)
        .await
        .map_err(|e| e.to_string())
}

/// "New chat": end the active session of `kind` (`"ask"`) so the next
/// send starts a fresh conversation. `true` when one was ended, `false`
/// when none was open (no junk row created). An in-flight ask run is
/// NOT killed: it keeps writing to this session's rows (the run's
/// `session_id` is bound at send) while its `ask:*` packets carry that
/// id, so the webview's cleared view drops them — leaving the
/// conversation detaches the view, never the stream.
#[tauri::command]
pub(crate) fn session_end_active(state: State<'_, AppState>, kind: String) -> Result<bool, String> {
    match state
        .db
        .session_active_id(&kind)
        .map_err(|e| e.to_string())?
    {
        Some(id) => {
            state.db.session_end(id).map_err(|e| e.to_string())?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Resume a past chat: ends the open `ask` session and reopens `id`
/// (`session_reopen` kind-guards — non-ask ids change nothing and
/// return false). The next `ask_send` appends to the reopened session.
/// No run is aborted: resuming the session a live run still writes to
/// re-attaches its tail via `ask_runs`; resuming a different one
/// leaves that run streaming into its own session, `session_id`-tagged
/// packets filtered out of this view.
#[tauri::command]
pub(crate) fn session_resume(state: State<'_, AppState>, id: i64) -> Result<bool, String> {
    state
        .db
        .session_reopen(id, "ask")
        .map_err(|e| e.to_string())
}
