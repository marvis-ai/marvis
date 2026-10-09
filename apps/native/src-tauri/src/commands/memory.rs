//! Memory CRUD — the settings Memory tab's IPC surface.
//!
//! `memory_list` hands the webview every stored fact for display;
//! `memory_update` rewrites a fact's value as a manual edit (a
//! `"manual"` row automatic extraction can never overwrite);
//! `memory_delete` removes it outright. Errors map to short SAFE
//! strings — sqlite detail (paths, SQL text) stays server-side — and
//! every successful mutation broadcasts `memory:changed` so open
//! windows refetch.

use crate::*;

/// Broadcast when a stored fact is added, edited, or deleted —
/// extraction landing new rows, or a settings edit/delete. An empty
/// ping: the tab re-reads via `memory_list`.
pub(crate) const EV_MEMORY_CHANGED: &str = "memory:changed";

/// Shared mapping so commands and tests assert the same contract:
/// internal errors collapse to a safe string; a missing row is a
/// distinct "not found" the UI can render.
fn map_update(result: anyhow::Result<Option<Memory>>) -> Result<Memory, String> {
    result
        .map_err(|_| "Memory could not be updated".to_string())?
        .ok_or_else(|| "Memory not found".to_string())
}

fn map_delete(result: anyhow::Result<()>) -> Result<(), String> {
    result.map_err(|_| "Memory could not be deleted".to_string())
}

#[tauri::command]
pub(crate) fn memory_list(state: State<'_, AppState>) -> Result<Vec<Memory>, String> {
    state.db.memory_profile().map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn memory_update(app: AppHandle, id: i64, value: String) -> Result<Memory, String> {
    let state = app.state::<AppState>();
    let memory = map_update(state.db.memory_update(id, &value))?;
    let _ = app.emit(EV_MEMORY_CHANGED, json!({}));
    Ok(memory)
}

#[tauri::command]
pub(crate) fn memory_delete(app: AppHandle, id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    map_delete(state.db.memory_delete(id))?;
    let _ = app.emit(EV_MEMORY_CHANGED, json!({}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A crafted/deleted id maps to the safe "not found" string, and an
    /// internal failure collapses — sqlite text (paths, SQL detail) is
    /// never a command error.
    #[test]
    fn update_maps_missing_id_and_internal_errors_safely() {
        assert_eq!(map_update(Ok(None)).unwrap_err(), "Memory not found");
        assert_eq!(
            map_update(Err(anyhow::anyhow!(
                "sqlite error near /Users/x/.marvis/marvis.db: no such column"
            )))
            .unwrap_err(),
            "Memory could not be updated"
        );
    }

    #[test]
    fn delete_never_exposes_database_text() {
        assert_eq!(
            map_delete(Err(anyhow::anyhow!(
                "sqlite: disk I/O error at /Users/x/.marvis/marvis.db"
            )))
            .unwrap_err(),
            "Memory could not be deleted"
        );
    }
}
