use crate::*;

/// The merged preset list — built-ins first, then `prompts.custom`.
/// The palette view, the bar's chip matcher, and the prefs tab read
/// it; `ask_send` resolves ids against the same order server-side.
#[tauri::command]
pub(crate) fn presets_list(state: State<'_, AppState>) -> Vec<presets::Preset> {
    presets::all(&state.config.lock().prompts.custom)
}

/// The composer wand's popup (and the input-row right-click, the bare
/// `/` send, a leading `/` keystroke): the preset palette — a small
/// overlay left-aligned to the composer caret (`anchor_x` is the
/// caret's x in bar-viewport px, resolved against the window's outer
/// position; the pointer, then the bar's center, are the fallbacks)
/// and announced as `palette:open { query }` — `query` seeds the
/// palette's filter when the open rides a typed `/token`. `anchor_y`
/// is the composer row's top edge in bar-viewport px — a card-mode
/// open pops the palette above it (the row is the card's bottom
/// footer; the pill edge only coincides with it growing up).
/// `focused` selects the interaction model: the wand/right-click open
/// key-focus it (own nav, click-away dismiss); a `/`-typed open
/// surfaces it unfocused so the composer keeps the `/token`
/// (`palette:query` streams the filter, `palette:key` forwards nav
/// keys). Gate-guarded like `ask_send` — a crafted invoke during
/// onboarding must not pop chrome over the wizard.
#[tauri::command(async)]
pub(crate) fn presets_palette_open(
    app: AppHandle,
    anchor_x: Option<f64>,
    anchor_y: Option<f64>,
    query: Option<String>,
    focused: Option<bool>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("presets_palette_open dropped while gate != Main");
        return Ok(());
    }
    state
        .pool
        .lock()
        .show_palette(&app, anchor_x, anchor_y, query, focused.unwrap_or(true));
    Ok(())
}

/// A palette row pick: hide the palette, hand focus back to the bar,
/// then emit the preset as `bar:preset-pick` — the same event the
/// composer's apply path consumes for the `/` shorthand.
#[tauri::command]
pub(crate) fn presets_palette_select(app: AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("presets_palette_select dropped while gate != Main");
        return Ok(());
    }
    let preset = presets::find(&id, &state.config.lock().prompts.custom);
    let bar = {
        let mut pool = state.pool.lock();
        pool.hide_palette(&app);
        pool.bar().cloned()
    };
    if let Some(bar) = bar {
        let _ = bar.set_focus();
    }
    if let Some(p) = preset {
        let _ = app.emit_to(windows::BAR_LABEL, EV_PRESET_PICK, p);
    }
    Ok(())
}

/// Palette dismissed by key — Esc in either focus mode, or a
/// forwarded Esc (click-away blur and the bar's own blur hide it
/// without this call). Hides + refocuses the bar; the composer's
/// `/token` text needs no reconciliation — it never left the field.
/// `hide_palette` announces `bar:palette-closed` for the bar's
/// key-forwarding gate. Gate-guarded like `open` — a crafted invoke
/// during onboarding must not refocus the (hidden) bar.
#[tauri::command]
pub(crate) fn presets_palette_close(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("presets_palette_close dropped while gate != Main");
        return Ok(());
    }
    let bar = {
        let mut pool = state.pool.lock();
        pool.hide_palette(&app);
        pool.bar().cloned()
    };
    if let Some(bar) = bar {
        let _ = bar.set_focus();
    }
    Ok(())
}

/// A composer key forwarded to the open palette (`palette:key`) —
/// the `/`-typed palette is unfocused, so the bar pipes `↑↓`, `Enter`,
/// `Tab`, `Esc`, `Home`, `End` through here for the view to run.
/// Emits to the palette window only; harmless when it's hidden.
/// Gate-guarded like `open`.
#[tauri::command]
pub(crate) fn presets_palette_key(app: AppHandle, key: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("presets_palette_key dropped while gate != Main");
        return Ok(());
    }
    state.pool.lock().palette_key(&app, &key);
    Ok(())
}

/// The composer's `/token` pushed as the palette's filter
/// (`palette:query`) on every composer edit while an unfocused
/// palette is up — the field that once lived in the palette now lives
/// in the composer. Gate-guarded like `open`.
#[tauri::command]
pub(crate) fn presets_palette_query(app: AppHandle, query: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("presets_palette_query dropped while gate != Main");
        return Ok(());
    }
    state.pool.lock().palette_query(&app, &query);
    Ok(())
}

/// The palette view's content-height report — the window hugs its
/// list: clamped to `[PALETTE_MIN_H, PALETTE_MAX_H]` (past max the
/// list scrolls) and re-anchored to the open-time caret x, so a
/// filter-shrink keeps the same gap off the bar. Gate-guarded like
/// `open`.
#[tauri::command]
pub(crate) fn presets_palette_height(app: AppHandle, height: f64) -> Result<(), String> {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("presets_palette_height dropped while gate != Main");
        return Ok(());
    }
    state.pool.lock().set_palette_height(height);
    Ok(())
}

