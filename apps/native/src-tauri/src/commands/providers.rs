use crate::*;

/// `keystore_status` return value and `keystore:changed` payload:
/// `{"keys": [[provider, "…last4"], …]}`. Masked only — plaintext keys
/// never leave `Keystore`. (There is no lock state: `keys.json` is
/// plaintext-on-disk inside the 0700 `~/.marvis` root.)
pub(crate) fn keystore_status_payload(keystore: &Keystore) -> serde_json::Value {
    json!({ "keys": keystore.masked_status() })
}

/// Deepgram is an STT-only key and must never enter the LLM provider
/// catalog. Key commands accept it alongside the LLM provider ids.
pub(crate) fn is_key_management_provider(provider: &str) -> bool {
    provider == "deepgram" || ProviderKind::from_str(provider).is_some()
}

pub(crate) const DEEPGRAM_UNVERIFIED_MESSAGE: &str =
    "Deepgram key accepted after non-empty shape validation; no live provider probe was performed";

pub(crate) fn normalize_deepgram_key(key: &str) -> Result<String, String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        Err("Deepgram API key must not be empty after trimming".to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

pub(crate) fn deepgram_validation_payload(key: &str) -> serde_json::Value {
    match normalize_deepgram_key(key) {
        Ok(_) => json!({ "ok": true, "message": DEEPGRAM_UNVERIFIED_MESSAGE }),
        Err(error) => json!({ "ok": false, "error": error }),
    }
}

/// Raise the alert toast with `message`.
///
/// The toast is a window of its own because the bar is a fixed-height
/// capsule (172⇄600 wide) — the old inline error row squeezed the
/// pill's content. It is purely informational and auto-dismisses.
pub(crate) fn show_alert(app: &AppHandle, message: &str) {
    let state = app.state::<AppState>();
    let payload = json!({ "message": message });
    *state.alert.lock() = Some(payload.clone());
    let _ = app.emit_to(windows::ALERT_LABEL, "alert:show", payload);
    state.pool.lock().show_alert();
}

/// Surface the prefs window in settings mode. Works at ANY gate —
/// the tray item must respond even mid-onboarding (the bar's `Cmd+,`
/// is a webview key, so it can't fire while the bar is hidden).
pub(crate) fn show_settings(app: &AppHandle) {
    app.state::<AppState>()
        .pool
        .lock()
        .show_prefs(app, "settings");
}

/// Broadcast the (masked) keystore status after any mutation.
pub(crate) fn emit_keystore_changed(app: &AppHandle, keystore: &Keystore) {
    let _ = app.emit("keystore:changed", keystore_status_payload(keystore));
}

/// `(model, base_url)` args for building a provider. `model` is the
/// provider's remembered pick (`providers.models.<id>`), else its first
/// static model (`""` for Ollama — its `validate` hits `/api/tags` and
/// never names a model — and for a compatible endpoint, whose validation
/// hits `/models`). `base_url` is the configured `compat.base_url`
/// (only `Compatible` reads it; `None` when unset).
pub(crate) fn provider_args(state: &AppState, kind: ProviderKind) -> (String, Option<String>) {
    let cfg = state.config.lock();
    let model = cfg.providers.model_for(kind.as_str());
    let base_url = Some(cfg.compat.base_url.clone()).filter(|u| !u.is_empty());
    (model, base_url)
}

/// One usable entry in the failover chain — the ask pipeline tries
/// candidates front-to-back until one answers.
pub(crate) struct ProviderCandidate {
    /// Provider id (`ProviderKind::as_str`) — reported on `ask:done`.
    pub id: String,
    /// The model the adapter was built with — reported on `ask:done`.
    pub model: String,
    /// Arc'd so the answering candidate can share it with the detached
    /// title sidecar — that task outlives the borrow of `candidates`.
    pub provider: Arc<dyn llm::Provider>,
}

/// The failover chain in priority order: `providers.order`, minus the
/// disabled, minus the unusable — no key where one is required, no
/// `compat.base_url` for `compatible`, no resolvable model. The first
/// entry is what `model_get_selected` reports; the ask pipeline walks the
/// rest on failure.
pub(crate) fn provider_candidates(cfg: &Config, ks: &Keystore) -> Vec<ProviderCandidate> {
    cfg.providers
        .order
        .iter()
        .filter(|id| cfg.providers.is_enabled(id))
        .filter_map(|id| {
            let kind = ProviderKind::from_str(id)?;
            let api_key = ks.key(id);
            if api_key.is_none() && !kind.key_optional() {
                return None; // no key — can't answer
            }
            let base_url = Some(cfg.compat.base_url.clone()).filter(|u| !u.is_empty());
            if kind == ProviderKind::Compatible && base_url.is_none() {
                return None; // endpoint never configured
            }
            let model = cfg.providers.model_for(id);
            if model.is_empty() {
                return None; // live-list provider with nothing selected
            }
            Some(ProviderCandidate {
                id: id.clone(),
                provider: Arc::from(make_provider(kind, api_key, model.clone(), base_url)),
                model,
            })
        })
        .collect()
}

/// The configured screen reader (`[vision]`): `vision.provider` resolved
/// the same way as a chain candidate — key where one is required,
/// `compat.base_url` for a compatible endpoint, a resolvable model from
/// `vision.models.<id>` or the provider's vision default — but
/// INDEPENDENT of `providers.order`/`disabled` (a chat-disabled provider
/// may still read the screen). `None` = off or unusable; the ask then
/// attaches the frame to the answering provider as before.
pub(crate) fn vision_candidate(cfg: &Config, ks: &Keystore) -> Option<ProviderCandidate> {
    let kind = ProviderKind::from_str(&cfg.vision.provider).filter(|k| k.is_vision())?;
    let api_key = ks.key(kind.as_str());
    if api_key.is_none() && !kind.key_optional() {
        return None; // no key — can't read
    }
    let base_url = Some(cfg.compat.base_url.clone()).filter(|u| !u.is_empty());
    if kind == ProviderKind::Compatible && base_url.is_none() {
        return None; // endpoint never configured
    }
    let model = cfg.vision.model_for(kind.as_str());
    if model.is_empty() {
        return None; // compatible with no model id typed yet
    }
    Some(ProviderCandidate {
        id: kind.as_str().to_string(),
        provider: Arc::from(make_provider(kind, api_key, model.clone(), base_url)),
        model,
    })
}

/// `GET /api/tags` → model names; any error (daemon down, bad body) maps
/// to an empty list — the dropdown just shows nothing.
pub(crate) async fn ollama_models() -> Vec<String> {
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(llm::CONNECT_TIMEOUT)
        .timeout(llm::VALIDATE_TIMEOUT)
        .build()
    else {
        return Vec::new();
    };
    let Ok(resp) = client.get(OLLAMA_TAGS_URL).send().await else {
        return Vec::new();
    };
    let Ok(body) = resp.json::<serde_json::Value>().await else {
        return Vec::new();
    };
    body["models"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter_map(|m| m["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// `window.bar_*` accepts a JSON number (set) or null (forget the pref).
pub(crate) fn window_pref_value(value: &serde_json::Value) -> Result<Option<f64>, String> {
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_f64()
            .map(Some)
            .ok_or_else(|| "window position must be a number or null".to_string())
    }
}

/// `{"keys": masked_status()}` — the shape the Providers tab renders;
/// never carries plaintext. No init/unlock/lock commands exist: the
/// store is plaintext `keys.json` and is always ready.
#[tauri::command]
pub(crate) fn keystore_status(state: State<'_, AppState>) -> serde_json::Value {
    keystore_status_payload(&state.keystore.lock())
}

/// Store a key and broadcast `keystore:changed`. Normal LLM provider keys
/// are live-validated before storage; Deepgram keys are trimmed and stored
/// after non-empty shape validation only, without a live provider probe.
#[tauri::command]
pub(crate) async fn keystore_set_key(
    app: AppHandle,
    provider: String,
    key: String,
) -> Result<serde_json::Value, String> {
    if !is_key_management_provider(&provider) {
        return Err(format!("unknown provider {provider:?}"));
    }
    let key = if provider == "deepgram" {
        normalize_deepgram_key(&key)?
    } else {
        let kind = ProviderKind::from_str(&provider).expect("validated provider id");
        let (model, base_url) = {
            let state = app.state::<AppState>();
            provider_args(&state, kind)
        };
        make_provider(kind, Some(key.clone()), model, base_url)
            .validate()
            .await
            .map_err(|e| e.to_string())?;
        key
    };
    let state = app.state::<AppState>();
    let payload = {
        let mut ks = state.keystore.lock();
        ks.set_key(&provider, &key).map_err(|e| e.to_string())?;
        emit_keystore_changed(&app, &ks);
        keystore_status_payload(&ks)
    };
    if provider == "deepgram" {
        refresh_speech_setup(&app);
    }
    Ok(payload)
}

/// Remove a provider key and broadcast `keystore:changed`.
#[tauri::command]
pub(crate) fn keystore_remove_key(app: AppHandle, provider: String) -> Result<serde_json::Value, String> {
    if !is_key_management_provider(&provider) {
        return Err(format!("unknown provider {provider:?}"));
    }
    let state = app.state::<AppState>();
    let payload = {
        let mut ks = state.keystore.lock();
        ks.remove_key(&provider).map_err(|e| e.to_string())?;
        emit_keystore_changed(&app, &ks);
        keystore_status_payload(&ks)
    };
    if provider == "deepgram" {
        refresh_speech_setup(&app);
    }
    Ok(payload)
}

// ---------------------------------------------------------------------------
// Commands — models
// ---------------------------------------------------------------------------

/// Test a candidate key WITHOUT storing it: `{"ok": true}` or
/// `{"ok": false, "error": "..."}` — validation failures are data, not
/// command errors, so the UI can render them inline. Normal LLM keys are
/// live-probed; Deepgram only gets non-empty shape validation and reports
/// that no live provider probe was performed.
#[tauri::command]
pub(crate) async fn model_validate_key(app: AppHandle, provider: String, key: String) -> serde_json::Value {
    if provider == "deepgram" {
        return deepgram_validation_payload(&key);
    }
    let Some(kind) = ProviderKind::from_str(&provider) else {
        return json!({ "ok": false, "error": format!("unknown provider {provider:?}") });
    };
    let (model, base_url) = {
        let state = app.state::<AppState>();
        provider_args(&state, kind)
    };
    // `Some(key)` is also right for a keyless compatible endpoint: the
    // adapter treats an empty key as "no auth header", and the frontend
    // passes "" when the endpoint is open.
    match make_provider(kind, Some(key), model, base_url)
        .validate()
        .await
    {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `{"provider": ..., "model": ...}` — the provider that would answer an
/// ask right now (the first usable entry in `providers.order`), or `null`
/// when no provider is usable. Replaces the old `[models] llm_*` pair:
/// selection is now per-provider memory (`providers.models`) plus the
/// priority order — whichever enabled provider sorts first answers.
#[tauri::command]
pub(crate) fn model_get_selected(state: State<'_, AppState>) -> serde_json::Value {
    let cfg = state.config.lock();
    let ks = state.keystore.lock();
    match provider_candidates(&cfg, &ks).into_iter().next() {
        Some(c) => json!({ "provider": c.id, "model": c.model }),
        None => serde_json::Value::Null,
    }
}

/// Persist `provider`'s model pick to `providers.models.<id>`; returns
/// the EFFECTIVE selection — the first usable provider in
/// `providers.order` — or `null` when none is usable. A write for a
/// non-primary provider must not relabel who's answering, so the return
/// resolves the chain rather than echoing the write. Broadcasts
/// `config:changed` like every config write.
#[tauri::command]
pub(crate) fn model_set_selected(
    app: AppHandle,
    provider: String,
    model: String,
) -> Result<serde_json::Value, String> {
    if ProviderKind::from_str(&provider).is_none() {
        return Err(format!("unknown provider {provider:?}"));
    }
    let state = app.state::<AppState>();
    let (updated, resolved) = {
        let mut cfg = state.config.lock();
        cfg.providers.models.insert(provider, model);
        config::save(&cfg).map_err(|e| e.to_string())?;
        let ks = state.keystore.lock();
        let resolved = provider_candidates(&cfg, &ks).into_iter().next();
        (cfg.clone(), resolved)
    };
    let _ = app.emit("config:changed", &updated);
    Ok(match resolved {
        Some(c) => json!({ "provider": c.id, "model": c.model }),
        None => serde_json::Value::Null,
    })
}

/// Drag-order write from Settings → Providers: `order` must be a
/// permutation of the known provider ids — anything else is rejected so
/// a stale frontend can't silently drop or invent a provider.
#[tauri::command]
pub(crate) fn providers_reorder(app: AppHandle, order: Vec<String>) -> Result<Config, String> {
    let state = app.state::<AppState>();
    let mut sorted = order.clone();
    sorted.sort();
    let mut known: Vec<String> = ProviderKind::ALL
        .iter()
        .map(|k| k.as_str().to_string())
        .collect();
    known.sort();
    if sorted != known {
        return Err("order must be a permutation of the provider catalog".to_string());
    }
    let updated = {
        let mut cfg = state.config.lock();
        cfg.providers.order = order;
        config::save(&cfg).map_err(|e| e.to_string())?;
        cfg.clone()
    };
    let _ = app.emit("config:changed", &updated);
    Ok(updated)
}

/// Flip one provider's enabled switch. Disabled providers keep their
/// slot in `order` and their stored key — they're skipped at ask time
/// but stay visible (and draggable) in settings.
#[tauri::command]
pub(crate) fn provider_set_enabled(app: AppHandle, provider: String, enabled: bool) -> Result<Config, String> {
    if ProviderKind::from_str(&provider).is_none() {
        return Err(format!("unknown provider {provider:?}"));
    }
    let state = app.state::<AppState>();
    let updated = {
        let mut cfg = state.config.lock();
        if enabled {
            cfg.providers.disabled.retain(|d| d != &provider);
        } else if !cfg.providers.disabled.contains(&provider) {
            cfg.providers.disabled.push(provider);
        }
        config::save(&cfg).map_err(|e| e.to_string())?;
        cfg.clone()
    };
    let _ = app.emit("config:changed", &updated);
    Ok(updated)
}

/// Static per-provider lists (spec); Ollama resolves `GET /api/tags`,
/// a compatible endpoint resolves `GET {base}/models` with its stored key
/// (if any), and OpenRouter resolves `GET {OPENROUTER_BASE_URL}/models`
/// the same way — falling back to the curated static list when the fetch
/// comes back empty (offline, or a keyed listing with no key stored yet).
/// Unknown providers and fetch errors → empty list.
#[tauri::command]
pub(crate) async fn model_list_available(app: AppHandle, provider: String) -> Vec<String> {
    match ProviderKind::from_str(&provider) {
        Some(ProviderKind::Ollama) => ollama_models().await,
        Some(ProviderKind::Compatible) => {
            let state = app.state::<AppState>();
            let (base_url, key) = {
                let base = state.config.lock().compat.base_url.clone();
                // No stored key → `None` — the listing still works for
                // open endpoints and simply comes back empty for keyed
                // ones, matching the every-failure-is-[] contract.
                let key = state.keystore.lock().key(ProviderKind::Compatible.as_str());
                (base, key)
            };
            llm::compat::list_models(&base_url, key).await
        }
        Some(ProviderKind::OpenRouter) => {
            // `key()` returns an owned Option — the lock drops before await.
            let key = app
                .state::<AppState>()
                .keystore
                .lock()
                .key(ProviderKind::OpenRouter.as_str());
            let live = llm::compat::list_models(llm::compat::OPENROUTER_BASE_URL, key).await;
            if live.is_empty() {
                llm::static_models(ProviderKind::OpenRouter)
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            } else {
                live
            }
        }
        Some(kind) => llm::static_models(kind)
            .iter()
            .map(|s| s.to_string())
            .collect(),
        None => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Commands — ask
// ---------------------------------------------------------------------------

