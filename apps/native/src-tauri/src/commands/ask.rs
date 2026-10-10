use crate::*;

/// Fire an ask; returns after synchronous pre-flight — tokens stream to
/// the `bar` window as `ask:*` events on a spawned task. `withScreen`
/// (optional) is the explicit attach flag — a screen read runs even when
/// the text shows no intent. `listenId` (optional) binds the send to a
/// listen doc — its own ask session, its summary+transcript as context.
/// `presetId` (optional) arms a preset — a resolvable id persists on
/// the user row so `ask_retry` re-applies it; its instruction text
/// (everything not carrying `{input}`) appends to the system prompt
/// for this send. `presetLang` (optional) is the `{lang}` param
/// badge's edited value — `None` resolves `{lang}` to the configured
/// main language. `attachments` (optional) carries the composer's
/// normalized JPEGs (`{ name, jpegBase64 }`). Returns `false` only when
/// the target session's own run is still live and the send was refused
/// — the composer keeps the draft so a refused send loses nothing.
#[tauri::command]
pub(crate) fn ask_send(
    app: AppHandle,
    text: String,
    with_screen: Option<bool>,
    listen_id: Option<i64>,
    preset_id: Option<String>,
    preset_lang: Option<String>,
    attachments: Option<Vec<ask::AskAttachmentInput>>,
) -> bool {
    let state = app.state::<AppState>();
    // Crafted-invoke guard: the shipped UI gates sends behind `Main`,
    // but a crafted invoke during onboarding would otherwise proceed —
    // DB writes, network, and emits to a window that doesn't exist yet.
    if *state.gate.lock() != Gate::Main {
        log::warn!("ask_send dropped while gate != Main");
        return false;
    }
    state.ask.send(
        &app,
        &state.deps(),
        &text,
        with_screen.unwrap_or(false),
        listen_id,
        preset_id,
        preset_lang,
        attachments.unwrap_or_default(),
    )
}

/// Regenerate the last answer — re-runs the active session's last user
/// turn (ask.rs `AskService::retry`). Same gate guard as `ask_send`.
#[tauri::command]
pub(crate) fn ask_retry(app: AppHandle) {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("ask_retry dropped while gate != Main");
        return;
    }
    state.ask.retry(&app, &state.deps());
}

/// Collapse the card. Deliberately does NOT cancel anything — a run
/// keeps streaming into its own session (stop is `ask_stop`'s job).
#[tauri::command]
pub(crate) fn ask_close(app: AppHandle) {
    let state = app.state::<AppState>();
    state.ask.close(&app, &state.pool);
}

/// The composer's stop button: cancel ONE session's in-flight run —
/// other sessions' runs stream on untouched. No gate guard —
/// cancelling a run is safe and idempotent anywhere.
#[tauri::command]
pub(crate) fn ask_stop(app: AppHandle, session_id: i64) {
    let state = app.state::<AppState>();
    state.ask.stop(&app, session_id);
}

/// The bar's camera affordance — a screen-only ask (fixed prompt,
/// frame required). Same gate guard as `ask_send`.
#[tauri::command]
pub(crate) fn ask_send_screen_only(app: AppHandle) {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("ask_send_screen_only dropped while gate != Main");
        return;
    }
    state.ask.send_screen_only(&app, &state.deps());
}

/// Every run's snapshot — `[{session_id, run, state, question,
/// response, error, attachments}]` — the live tails a re-mounted chat
/// or activity mirror resyncs from (the persisted session already
/// carries every completed turn). `error` re-delivers the last
/// `ask:error`, which can fire before the webview's `listen()` is up;
/// a `null` session_id entry is the sessionless pre-flight orphan.
#[tauri::command]
pub(crate) fn ask_runs(state: State<'_, AppState>) -> serde_json::Value {
    json!(state.ask.runs_payload())
}
