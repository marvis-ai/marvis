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
/// normalized JPEGs (`{ name, jpegBase64 }`).
#[tauri::command]
pub(crate) fn ask_send(
    app: AppHandle,
    text: String,
    with_screen: Option<bool>,
    listen_id: Option<i64>,
    preset_id: Option<String>,
    preset_lang: Option<String>,
    attachments: Option<Vec<ask::AskAttachmentInput>>,
) {
    let state = app.state::<AppState>();
    // Crafted-invoke guard: the shipped UI gates sends behind `Main`,
    // but a crafted invoke during onboarding would otherwise proceed —
    // DB writes, network, and emits to a window that doesn't exist yet.
    if *state.gate.lock() != Gate::Main {
        log::warn!("ask_send dropped while gate != Main");
        return;
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
    );
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

/// Cancel the in-flight stream and collapse the card.
#[tauri::command]
pub(crate) fn ask_close(app: AppHandle) {
    let state = app.state::<AppState>();
    state.ask.close(&app, &state.pool);
}

/// The composer's stop button: cancel the in-flight run WITHOUT
/// collapsing the card (`ask_close` is this plus the collapse). No
/// gate guard — cancelling a run is safe and idempotent anywhere.
#[tauri::command]
pub(crate) fn ask_stop(app: AppHandle) {
    let state = app.state::<AppState>();
    state.ask.abort(&app);
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

/// `{"state": "idle"|"loading"|"streaming", "question": ..., "response":
/// ..., "error": {...}|null}` — the live tail a re-expanded chat
/// resyncs from (the persisted session already carries every completed
/// turn); `error` re-delivers the last `ask:error`, which can fire
/// before the webview's `listen()` is up (cold-open pre-flight errors).
#[tauri::command]
pub(crate) fn ask_current(state: State<'_, AppState>) -> serde_json::Value {
    state.ask.current_payload()
}
