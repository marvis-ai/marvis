use crate::*;

#[tauri::command]
pub(crate) fn config_get(state: State<'_, AppState>) -> Config {
    state.config.lock().clone()
}

/// Limited writable surface: `hotkeys.<action>`, `window.bar_x`,
/// `window.bar_y` (number sets, null clears), `app.onboarding_done`
/// (bool), `app.appearance` (`auto|light|dark`), `app.accent`
/// (`#rrggbb`, `""` resets to the spec slate), `app.main_language`
/// (`en|zh|ja|ko|fr|es` — chat/summary output; STT auto-detects),
/// `compat.name`,
/// `compat.base_url` (validated http(s) URL; `""` clears),
/// `models.stt_provider` (`deepgram|whisper|sherpa`), and `models.stt_model`
/// (a trimmed non-empty identifier), `recording.auto_screenshots` (bool),
/// `recording.fps` (`8|4|2` — a write during a live capture restarts it
/// so the new rate applies now), `recording.read_interval_secs` (u64
/// ≥1 — minimum seconds between ambient screen reads; applies on the
/// next capture start), `recording.summary_prompt` (string), and
/// `prompts.custom` (array of `{id, name, text}` presets —
/// replaces the whole custom list; rejected rows fail the write), and
/// `memory.provider`/`memory.model`/`memory.enabled` (the dedicated
/// Memory LLM pick; enabling requires provider+model already set).
/// Provider order/switches/models have
/// their own commands (`providers_reorder`,
/// `provider_set_enabled`, `model_set_selected`). Persists `config.toml`
/// and returns the updated config. A `hotkeys.*` write delta-swaps the
/// registered set. Every
/// successful write broadcasts `config:changed` so open windows
/// re-render (appearance flips, provider lists, the bar's drag hint).
///
/// `app.onboarding_done` is the wizard's completion write: `true` ends
/// onboarding → `transition_gate` can now reach `Main` (capture, the
/// card) and the bar appears; `false` (a re-run) reverses it.
#[tauri::command(async)]
pub(crate) fn config_set(
    app: AppHandle,
    key: String,
    value: serde_json::Value,
) -> Result<Config, String> {
    let state = app.state::<AppState>();
    let mut hotkeys_changed = false;
    let mut onboarding_changed = false;
    let mut accent_changed = false;
    let mut stt_changed = false;
    let fps_changed: bool;
    {
        let mut cfg = state.config.lock();
        let prev_fps = cfg.recording.fps;
        match key.as_str() {
            "window.bar_x" => cfg.window.bar_x = window_pref_value(&value)?,
            "window.bar_y" => cfg.window.bar_y = window_pref_value(&value)?,
            "window.bar_locked" => {
                cfg.window.bar_locked =
                    value.as_bool().ok_or("window.bar_locked must be a bool")?;
            }
            "app.onboarding_done" => {
                let v = value
                    .as_bool()
                    .ok_or("app.onboarding_done must be a bool")?;
                onboarding_changed = cfg.app.onboarding_done != v;
                cfg.app.onboarding_done = v;
            }
            "app.appearance" => {
                let v = value.as_str().ok_or("app.appearance must be a string")?;
                if !matches!(v, "auto" | "light" | "dark") {
                    return Err(format!("unknown appearance {v:?}"));
                }
                cfg.app.appearance = v.to_string();
            }
            "app.accent" => {
                let v = value
                    .as_str()
                    .ok_or("app.accent must be a string")?
                    .trim()
                    .to_string();
                let next = if v.is_empty() {
                    config::DEFAULT_ACCENT.to_string()
                } else {
                    let ok = v.len() == 7
                        && v.starts_with('#')
                        && v[1..].chars().all(|c| c.is_ascii_hexdigit());
                    if !ok {
                        return Err("app.accent must be a #rrggbb color".to_string());
                    }
                    v
                };
                accent_changed = cfg.app.accent != next;
                cfg.app.accent = next;
            }
            "app.main_language" => {
                cfg.app.main_language = config::validate_main_language(
                    value.as_str().ok_or("app.main_language must be a string")?,
                )?;
            }
            key if config::apply_recording_config(&mut cfg.recording, key, &value)? => {}
            key if config::apply_prompts_config(&mut cfg.prompts, key, &value)? => {}
            key if config::apply_memory_config(&mut cfg.memory, key, &value)? => {}
            "compat.name" => {
                cfg.compat.name = value
                    .as_str()
                    .ok_or("compat.name must be a string")?
                    .trim()
                    .to_string();
            }
            "compat.base_url" => {
                let v = value
                    .as_str()
                    .ok_or("compat.base_url must be a string")?
                    .trim();
                if !v.is_empty() && !llm::compat::is_valid_base_url(v) {
                    return Err("compat.base_url must be an http(s):// URL".to_string());
                }
                cfg.compat.base_url = v.to_string();
            }
            key if config::apply_stt_config(&mut cfg.models, key, &value)? => {
                stt_changed = true;
            }
            "vision.provider" => {
                let v = value
                    .as_str()
                    .ok_or("vision.provider must be a string")?
                    .trim();
                let ok = v.is_empty() || ProviderKind::from_str(v).is_some_and(|k| k.is_vision());
                if !ok {
                    return Err(format!("unknown vision provider {v:?}"));
                }
                cfg.vision.provider = v.to_string();
            }
            _ if key.starts_with("vision.models.") => {
                let id = &key["vision.models.".len()..];
                if ProviderKind::from_str(id).is_none_or(|k| !k.is_vision()) {
                    return Err(format!("unknown vision provider {id:?}"));
                }
                let model = value
                    .as_str()
                    .ok_or("vision.models.* must be a string")?
                    .trim();
                if model.is_empty() {
                    cfg.vision.models.remove(id);
                } else {
                    cfg.vision.models.insert(id.to_string(), model.to_string());
                }
            }
            _ if key.starts_with("hotkeys.") => {
                let name = &key["hotkeys.".len()..];
                if !config::default_hotkeys().contains_key(name) {
                    return Err(format!("unknown hotkey action {name:?}"));
                }
                let accel = value.as_str().ok_or("hotkey binding must be a string")?;
                if hotkey::accelerator_for(accel).is_none() {
                    return Err(format!("accelerator {accel:?} doesn't parse"));
                }
                cfg.hotkeys.insert(name.to_string(), accel.to_string());
                hotkeys_changed = true;
            }
            _ => return Err(format!("unknown or read-only config key {key:?}")),
        }
        config::save(&cfg).map_err(|e| e.to_string())?;
        fps_changed = cfg.recording.fps != prev_fps;
    }
    if hotkeys_changed {
        swap_hotkeys(&app);
    }
    if accent_changed {
        // Re-tint the bar's glass now — otherwise the new accent only
        // reaches the material on the next pill⇄card morph.
        state.pool.lock().refresh_bar_glass(&app);
    }
    if fps_changed {
        let _transition = state.gate_transition.lock();
        let restart = {
            let capture = state.capture.lock();
            if capture.as_ref().is_some_and(PlatformCapture::is_running) {
                state.capture_restart.lock().clone()
            } else {
                None
            }
        };
        if let Some(mut restart) = restart {
            // Resolve before stopping: a stale picker identity must never
            // widen the capture to the primary display.
            let resolved = if restart.target.is_none() {
                primary_display_source().map(|(source, w, h)| {
                    restart.source = source;
                    restart.width = w;
                    restart.height = h;
                })
            } else {
                #[cfg(not(target_os = "linux"))]
                let result = if let Some(id) = &restart.resolver_id {
                    capture::resolve_candidate(id).map(|res| {
                        restart.source = res.source;
                        restart.width = res.w;
                        restart.height = res.h;
                        restart.target = Some(CaptureTarget {
                            kind: res.kind,
                            label: res.label,
                        });
                    })
                } else {
                    Ok(())
                };
                #[cfg(target_os = "linux")]
                let result: anyhow::Result<()> = Ok(());
                result
            };
            match resolved {
                Ok(()) => {
                    stop_capture(&app);
                    start_capture_with_resolver(
                        &app,
                        restart.source,
                        restart.width,
                        restart.height,
                        restart.target,
                        restart.resolver_id,
                    );
                }
                Err(e) => log::warn!("capture: restart source failed: {e}"),
            }
        }
    }
    if onboarding_changed {
        // Gate first: `enter_main` starts capture while the
        // wizard is still the visible window; then the bar un-hides.
        // `transition_gate`/`sync_bar_visibility` reach the pool's
        // window getters, which park the caller on the main queue —
        // this command runs on a worker, so hop rather than run them
        // off the main thread.
        run_on_main(&app, "config_set", |app| {
            transition_gate(app);
            app.state::<AppState>().sync_bar_visibility();
        });
    }
    let updated = state.config.lock().clone();
    let _ = app.emit("config:changed", &updated);
    if stt_changed {
        refresh_speech_setup(&app);
    }
    // bar_locked/edge/hotkey writes all show up in the menu's
    // labels/checks — rebuild unconditionally (writes are user-driven).
    refresh_tray_menu(&app);
    Ok(updated)
}
