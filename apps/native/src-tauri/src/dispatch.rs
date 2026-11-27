//! Event dispatch — the closures `run()` hands to the hotkey plugin,
//! the deep-link plugin, and `on_menu_event` map their actions onto the
//! same boundaries commands use (`toggle_capture`, `set_bar_locked`, …).
//! Each owns an `AppHandle` and re-resolves `AppState` per event so it
//! survives gate transitions and webview reloads.

use crate::*;

/// Delta-swap to the config's current binding set via
/// [`hotkey::swap_hotkey_set`]: pairs shared with the live set stay
/// registered untouched (macOS Carbon refuses duplicate registration of
/// a combo, so a register-everything-then-unregister swap can never
/// succeed). On failure the swap has already rolled back; the returned
/// `restored` set is what the OS still has bound, so it's stored
/// either way.
pub(crate) fn swap_hotkeys(app: &AppHandle) {
    let state = app.state::<AppState>();
    let binds = state.config.lock().hotkeys.clone();
    let dispatch = hotkey_dispatch(app);
    // Hold the guard across the swap: a concurrent swap seeing `None`
    // would compute a full `add` set against still-live OS bindings and
    // wedge every future swap. `state.hotkeys` is locked nowhere else,
    // so holding it here cannot deadlock.
    let mut slot = state.hotkeys.lock();
    let prev = slot.take().unwrap_or_default();
    match hotkey::swap_hotkey_set(app, &binds, dispatch, prev) {
        Ok(set) => {
            *slot = Some(set);
        }
        Err(e) => {
            log::warn!("hotkey swap failed: {e}");
            *slot = Some(e.restored);
        }
    }
    drop(slot);
    // A rebind rewrites the menu items' accelerator labels.
    refresh_tray_menu(app);
}

/// Rust→bar event the `toggle_input` hotkey fires. The webview owns the
/// capsule⇄input morph (and the "open card counts as shown" collapse),
/// so dispatch only emits — it never touches `chat_open`.
const EV_BAR_TOGGLE_INPUT: &str = "bar:toggle-input";
/// Rust→bar events the shared menu and the `start_listen` /
/// `show_history` hotkeys fire. The webview owns both surfaces —
/// dispatch only emits.
const EV_BAR_START_LISTEN: &str = "bar:start-listen";
const EV_BAR_SHOW_HISTORY: &str = "bar:show-history";
/// Emitted to the `bar` window only — a preset palette pick
/// (`presets_palette_select`); payload is the full `Preset`.
pub(crate) const EV_PRESET_PICK: &str = "bar:preset-pick";
/// Rust→palette events emitted from `windows/mod.rs` (`pub(crate)`
/// keeps one const per name): `show_palette` announces the open
/// (`{ query }` seeds the filter), `palette_query` pushes the
/// composer's live `/token`, `palette_key` forwards nav keys while
/// the bar keeps focus.
pub(crate) const EV_PALETTE_OPEN: &str = "palette:open";
pub(crate) const EV_PALETTE_QUERY: &str = "palette:query";
pub(crate) const EV_PALETTE_KEY: &str = "palette:key";
/// Emitted to the `bar` window on every palette hide (`windows`'s
/// `hide_palette`) — the composer stops forwarding `/`-mode keys.
/// Emitted from `windows/mod.rs`; `pub(crate)` keeps one const.
pub(crate) const EV_PALETTE_CLOSED: &str = "bar:palette-closed";

/// Map [`hotkey::Action`]s onto pool calls / bar events. Owns an
/// `AppHandle` and re-resolves `AppState` per press, so the same
/// closure survives gate transitions.
pub(crate) fn hotkey_dispatch(app: &AppHandle) -> impl Fn(hotkey::Action) + Send + Sync + 'static {
    let app = app.clone();
    move |action| match action {
        hotkey::Action::ToggleInput => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_TOGGLE_INPUT, ());
        }
        hotkey::Action::ToggleCapture => toggle_capture(&app),
        hotkey::Action::StartListen => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_START_LISTEN, ());
        }
        hotkey::Action::ShowHistory => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_SHOW_HISTORY, ());
        }
        hotkey::Action::ToggleLock => {
            let locked = app.state::<AppState>().config.lock().window.bar_locked;
            set_bar_locked(&app, !locked);
        }
    }
}

/// Map [`deeplink::Action`]s: `Ask` runs only while the gate is `Main`;
/// `Focus` surfaces the bar; `Ignore` never reaches the dispatch
/// (filtered inside `init`). While onboarding runs the bar is hidden, so
/// neither action may surface it.
pub(crate) fn deeplink_dispatch(app: &AppHandle) -> impl Fn(deeplink::Action) + Send + Sync + 'static {
    let app = app.clone();
    move |action| match action {
        deeplink::Action::Ask(text) => {
            let state = app.state::<AppState>();
            if *state.gate.lock() == Gate::Main {
                let bar = state.pool.lock().bar().cloned();
                if let Some(bar) = bar {
                    let _ = bar.set_focus();
                }
                state
                    .ask
                    .send(&app, &state.deps(), &text, false, None, None, None, Vec::new());
            } else {
                // Not ready yet — if the bar is visible (onboarding done,
                // permission pending) surface its gate card instead of
                // the link silently going nowhere.
                log::warn!("deeplink: ask gated off (gate = {:?})", *state.gate.lock());
                if state.onboarding_done() {
                    let bar = state.pool.lock().bar().cloned();
                    if let Some(bar) = bar {
                        let _ = bar.set_focus();
                    }
                }
            }
        }
        deeplink::Action::Focus => {
            let state = app.state::<AppState>();
            // During onboarding the bar is hidden — nothing to focus.
            if !state.onboarding_done() {
                return;
            }
            // Clone the handle out so the pool guard drops before `state`.
            let bar = state.pool.lock().bar().cloned();
            if let Some(bar) = bar {
                let _ = bar.set_focus();
            }
        }
        deeplink::Action::Ignore => {}
    }
}

/// Map `menu.*` item ids (menus.rs — shared by the tray menu and the
/// bar's right-click popup) onto the same actions the bar buttons,
/// commands, and hotkeys use. Registered once via `app.on_menu_event`
/// in `setup`: menu events broadcast to EVERY listener, so this must
/// be the only handler matching `menu.*` ids.
pub(crate) fn menu_dispatch() -> impl Fn(&AppHandle, tauri::menu::MenuEvent) + Send + Sync + 'static {
    move |app, event| {
        let id: &str = event.id().as_ref();
        // The four `menu.pos.*` edge ids share one table with the menu
        // builders (menus::POS_EDGES) — looked up here rather than
        // matched, so an edge can't drift from its `Dir`.
        if let Some(dir) = menus::pos_edge_dir(id) {
            snap_edge_and_refresh(app, dir);
            return;
        }
        match id {
            menus::MENU_ASK => {
                let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_TOGGLE_INPUT, ());
            }
            menus::MENU_CAPTURE => toggle_capture(app),
            menus::MENU_LISTEN => {
                let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_START_LISTEN, ());
            }
            menus::MENU_HISTORY => {
                let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_SHOW_HISTORY, ());
            }
            menus::MENU_POS_CENTER => {
                app.state::<AppState>().pool.lock().recenter_bar();
                refresh_tray_menu(app);
            }
            menus::MENU_LOCK => {
                let locked = app.state::<AppState>().config.lock().window.bar_locked;
                set_bar_locked(app, !locked);
            }
            menus::MENU_SETTINGS => show_settings(app),
            menus::MENU_SUPPORT => {
                if let Err(e) = app
                    .opener()
                    .open_url(menubar::SUPPORT_MAILTO, None::<&str>)
                {
                    log::warn!("open support mailto failed: {e}");
                }
            }
            // The item is built only in debug builds (menus.rs), so this
            // arm compiles out in release — `menu.devtools` can't fire
            // there anyway. Menu events carry no window identity: the bar
            // owns the only popup, so it's the target (the tray copy
            // inspects the bar too).
            #[cfg(debug_assertions)]
            menus::MENU_DEVTOOLS => {
                if let Some(bar) = app.get_webview_window(windows::BAR_LABEL) {
                    bar.open_devtools();
                }
            }
            menus::MENU_QUIT => app.exit(0),
            _ => {}
        }
    }
}
