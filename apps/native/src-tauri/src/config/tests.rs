use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

fn tempfile_dir() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "marvis-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn stt_command_keys_apply_and_validate_values() {
    let mut models = ModelPrefs::default();
    assert!(apply_stt_config(
        &mut models,
        "models.stt_provider",
        &serde_json::json!(" whisper "),
    )
    .unwrap());
    assert_eq!(models.stt_provider, "whisper");
    assert!(apply_stt_config(
        &mut models,
        "models.stt_model",
        &serde_json::json!("  base  "),
    )
    .unwrap());
    assert_eq!(models.stt_model, "base");
    assert_eq!(
        apply_stt_config(
            &mut models,
            "models.stt_provider",
            &serde_json::json!("unknown"),
        )
        .unwrap_err(),
        "unknown STT provider \"unknown\""
    );
    models.stt_provider = "whisper".into();
    assert_eq!(
        apply_stt_config(
            &mut models,
            "models.stt_model",
            &serde_json::json!("https://example.com/model.bin"),
        )
        .unwrap_err(),
        "unknown Whisper model \"https://example.com/model.bin\""
    );
    assert_eq!(
        apply_stt_config(
            &mut models,
            "models.stt_model",
            &serde_json::json!(" GGML-SMALL.BIN "),
        )
        .unwrap_err(),
        "unknown Whisper model \"GGML-SMALL.BIN\""
    );
    assert!(apply_stt_config(
        &mut models,
        "models.stt_model",
        &serde_json::json!(" small "),
    )
    .unwrap());
    assert_eq!(models.stt_model, "small");
    assert!(apply_stt_config(
        &mut models,
        "models.stt_model",
        &serde_json::json!(" ggml-small.bin "),
    )
    .is_ok());
    assert_eq!(models.stt_model, "small");
    models.stt_provider = "deepgram".into();
    assert!(apply_stt_config(
        &mut models,
        "models.stt_model",
        &serde_json::json!("nova-3"),
    )
    .is_ok());
    assert_eq!(models.stt_model, "nova-3");
    assert!(apply_stt_config(
        &mut models,
        "models.stt_provider",
        &serde_json::json!("whisper"),
    )
    .is_ok());
    assert_eq!(models.stt_model, "tiny");
    assert!(apply_stt_config(
        &mut models,
        "models.stt_provider",
        &serde_json::json!("deepgram"),
    )
    .is_ok());
    assert_eq!(models.stt_model, "nova-2");
    assert_eq!(
        apply_stt_config(&mut models, "models.stt_model", &serde_json::json!("  "),).unwrap_err(),
        "STT model must not be empty"
    );
    assert!(!apply_stt_config(&mut models, "models.other", &serde_json::json!("value"),).unwrap());
}

/// Provider and model are one transactional preference: every switch
/// must leave `stt_model` valid for the new provider — a foreign catalog
/// id (whisper or sherpa) resets to that provider's default, while a
/// custom deepgram name survives.
#[test]
fn stt_provider_switch_never_pairs_a_foreign_catalog_model() {
    let mut models = ModelPrefs::default();
    // deepgram → sherpa adopts the sherpa catalog default.
    assert!(apply_stt_config(
        &mut models,
        "models.stt_provider",
        &serde_json::json!("sherpa"),
    )
    .unwrap());
    assert_eq!(models.stt_model, "sense-voice");
    // sherpa → deepgram must not send "sense-voice" as a hosted model id.
    assert!(apply_stt_config(
        &mut models,
        "models.stt_provider",
        &serde_json::json!("deepgram"),
    )
    .unwrap());
    assert_eq!(models.stt_model, "nova-2");
    // sherpa → whisper resets to the whisper catalog default.
    assert!(apply_stt_config(
        &mut models,
        "models.stt_provider",
        &serde_json::json!("sherpa"),
    )
    .unwrap());
    assert!(apply_stt_config(
        &mut models,
        "models.stt_provider",
        &serde_json::json!("whisper"),
    )
    .unwrap());
    assert_eq!(models.stt_model, "tiny");
    // whisper → sherpa resets to the sherpa catalog default.
    assert!(apply_stt_config(
        &mut models,
        "models.stt_provider",
        &serde_json::json!("sherpa"),
    )
    .unwrap());
    assert_eq!(models.stt_model, "sense-voice");
    // A custom (non-catalog) deepgram model is the user's own value — a
    // deepgram reselect keeps it rather than resetting to nova-2.
    models.stt_provider = "deepgram".into();
    models.stt_model = "nova-3".into();
    assert!(apply_stt_config(
        &mut models,
        "models.stt_provider",
        &serde_json::json!("deepgram"),
    )
    .unwrap());
    assert_eq!(models.stt_model, "nova-3");
}

#[test]
fn load_returns_defaults_when_file_missing() {
    let tmp = tempfile_dir();
    let cfg = Config::load_from(tmp.join("config.toml")).unwrap();
    // Default order is the provider catalog; nothing disabled.
    assert_eq!(
        cfg.providers.order,
        ProviderKind::ALL
            .iter()
            .map(|k| k.as_str().to_string())
            .collect::<Vec<_>>()
    );
    assert!(cfg.providers.disabled.is_empty());
    assert_eq!(cfg.hotkeys["toggle_input"], "CmdOrCtrl+Alt+Space");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn save_then_load_roundtrips() {
    let tmp = tempfile_dir();
    let mut cfg = Config::default();
    cfg.providers.order = vec!["ollama".into(), "openai".into()];
    cfg.providers.disabled = vec!["gemini".into()];
    cfg.providers
        .models
        .insert("ollama".into(), "qwen3:8b".into());
    cfg.save_to(tmp.join("config.toml")).unwrap();
    let back = Config::load_from(tmp.join("config.toml")).unwrap();
    // Order persists verbatim, gaps fill in catalog order at the end.
    assert_eq!(
        back.providers.order,
        vec![
            "ollama",
            "openai",
            "anthropic",
            "gemini",
            "openrouter",
            "compatible"
        ]
    );
    assert_eq!(back.providers.disabled, vec!["gemini"]);
    assert_eq!(back.providers.models["ollama"], "qwen3:8b");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn legacy_llm_pair_migrates_into_provider_models() {
    // A config written before `[providers]` existed keeps its pick —
    // it becomes that provider's remembered model, and the legacy
    // fields never serialize back out.
    let tmp = tempfile_dir();
    let path = tmp.join("config.toml");
    std::fs::write(
        &path,
        "[models]\nllm_provider = \"anthropic\"\nllm_model = \"claude-opus-4-1\"\n",
    )
    .unwrap();
    let cfg = Config::load_from(&path).unwrap();
    assert_eq!(cfg.providers.models["anthropic"], "claude-opus-4-1");
    cfg.save_to(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("llm_provider"));
    assert!(!text.contains("llm_model"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn normalize_drops_unknown_ids_and_dedupes() {
    let tmp = tempfile_dir();
    let path = tmp.join("config.toml");
    std::fs::write(
        &path,
        "[providers]\norder = [\"openai\", \"bogus\", \"openai\", \"gemini\"]\n\
             disabled = [\"bogus\", \"ollama\", \"ollama\"]\n\
             [providers.models]\nbogus = \"x\"\nopenai = \"gpt-4o-mini\"\n",
    )
    .unwrap();
    let cfg = Config::load_from(&path).unwrap();
    assert_eq!(
        cfg.providers.order,
        vec![
            "openai",
            "gemini",
            "anthropic",
            "openrouter",
            "ollama",
            "compatible"
        ]
    );
    assert_eq!(cfg.providers.disabled, vec!["ollama"]);
    assert!(!cfg.providers.models.contains_key("bogus"));
    assert_eq!(cfg.providers.models["openai"], "gpt-4o-mini");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn model_for_falls_back_to_static_first() {
    let cfg = Config::default();
    assert_eq!(cfg.providers.model_for("openai"), "gpt-4o");
    // Live-list providers have no static default.
    assert_eq!(cfg.providers.model_for("ollama"), "");
    let mut cfg = Config::default();
    cfg.providers
        .models
        .insert("openai".into(), "gpt-4o-mini".into());
    assert_eq!(cfg.providers.model_for("openai"), "gpt-4o-mini");
}

#[test]
fn vision_section_roundtrips_and_defaults_to_off() {
    // No [vision] in the file → reader off with empty memory.
    let tmp = tempfile_dir();
    let cfg = Config::load_from(tmp.join("config.toml")).unwrap();
    assert_eq!(cfg.vision.provider, "");
    assert!(cfg.vision.models.is_empty());

    let mut cfg = Config::default();
    cfg.vision.provider = "gemini".into();
    cfg.vision
        .models
        .insert("gemini".into(), "gemini-2.5-pro".into());
    let path = tmp.join("config.toml");
    cfg.save_to(&path).unwrap();
    let back = Config::load_from(&path).unwrap();
    assert_eq!(back.vision.provider, "gemini");
    assert_eq!(back.vision.models["gemini"], "gemini-2.5-pro");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn normalize_drops_non_vision_provider_and_stale_models() {
    // Anthropic reads images but isn't in the reader's pick list —
    // a hand-edited config can't name it; unknown ids clear too.
    // Model memory keeps only vision ids with a non-empty pick.
    let tmp = tempfile_dir();
    let path = tmp.join("config.toml");
    std::fs::write(
            &path,
            "[vision]\nprovider = \"anthropic\"\n\
             [vision.models]\nanthropic = \"claude-opus-4-1\"\nopenai = \"gpt-4o-mini\"\nbogus = \"x\"\n",
        )
        .unwrap();
    let cfg = Config::load_from(&path).unwrap();
    assert_eq!(cfg.vision.provider, "");
    assert!(!cfg.vision.models.contains_key("anthropic"));
    assert!(!cfg.vision.models.contains_key("bogus"));
    assert_eq!(cfg.vision.models["openai"], "gpt-4o-mini");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn vision_model_for_falls_back_to_the_vision_default() {
    let cfg = Config::default();
    // Vision defaults are the cheap readers, not the chat flagships.
    assert_eq!(cfg.vision.model_for("openai"), "gpt-4o-mini");
    assert_eq!(cfg.vision.model_for("gemini"), "gemini-2.5-flash");
    assert_eq!(
        cfg.vision.model_for("openrouter"),
        "google/gemini-2.0-flash-001"
    );
    // Compatible is free-text — no static default exists.
    assert_eq!(cfg.vision.model_for("compatible"), "");
    let mut cfg = Config::default();
    cfg.vision.models.insert("openai".into(), "gpt-4o".into());
    assert_eq!(cfg.vision.model_for("openai"), "gpt-4o");
}

#[test]
fn save_is_atomic_and_leaves_no_tmp_file() {
    let tmp = tempfile_dir();
    let path = tmp.join("config.toml");
    Config::default().save_to(&path).unwrap();
    assert!(path.exists());
    assert!(!tmp.join("config.toml.tmp").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn partial_toml_fills_defaults_for_missing_keys() {
    // A config with only some keys set must keep defaults for the rest —
    // silently resetting everything on a partial file is a bug.
    let tmp = tempfile_dir();
    let path = tmp.join("config.toml");
    std::fs::write(&path, "[models]\nllm_provider = \"anthropic\"\n").unwrap();
    let cfg = Config::load_from(&path).unwrap();
    // The legacy pair still deserializes (for migration), and the
    // migrated model lands on the provider — but a partial file's
    // defaults fill everything else.
    assert_eq!(cfg.models.llm_provider, "anthropic");
    assert_eq!(cfg.models.llm_model, "gpt-4o");
    assert_eq!(cfg.providers.models["anthropic"], "gpt-4o");
    assert_eq!(cfg.models.stt_provider, "deepgram");
    assert_eq!(cfg.hotkeys["toggle_input"], "CmdOrCtrl+Alt+Space");
    assert!(cfg.window.bar_x.is_none());

    // Spec example writes integer positions (`bar_x = 812`) into f64 fields.
    std::fs::write(&path, "[window]\nbar_x = 812\nbar_y = 21\n").unwrap();
    let cfg = Config::load_from(&path).unwrap();
    assert_eq!(cfg.window.bar_x, Some(812.0));
    assert_eq!(cfg.window.bar_y, Some(21.0));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn partial_hotkeys_merge_with_defaults_and_drop_stale() {
    // A user override on the one rebindable action lands; names
    // this build doesn't know — including the retired
    // `toggle_visibility`/`next_step`/`screen_only`/`show_settings`
    // set — are dropped rather than kept forever.
    let tmp = tempfile_dir();
    let path = tmp.join("config.toml");
    std::fs::write(
            &path,
            "[hotkeys]\ntoggle_input = \"Ctrl+Alt+J\"\ntoggle_visibility = \"Cmd+/\"\nnext_step = \"Cmd+Enter\"\n",
        )
        .unwrap();
    let cfg = Config::load_from(&path).unwrap();
    assert_eq!(cfg.hotkeys["toggle_input"], "Ctrl+Alt+J");
    assert!(!cfg.hotkeys.contains_key("toggle_visibility"));
    assert!(!cfg.hotkeys.contains_key("next_step"));
    // The user override lands alongside the defaults it didn't set.
    assert_eq!(cfg.hotkeys.len(), default_hotkeys().len());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn default_hotkeys_matches_spec_table() {
    let hk = default_hotkeys();
    assert_eq!(hk.len(), 5);
    assert_eq!(hk["toggle_input"], "CmdOrCtrl+Alt+Space");
    assert_eq!(hk["toggle_capture"], "CmdOrCtrl+Alt+R");
    assert_eq!(hk["start_listen"], "CmdOrCtrl+Alt+T");
    assert_eq!(hk["show_history"], "CmdOrCtrl+Alt+H");
    assert_eq!(hk["toggle_lock"], "CmdOrCtrl+Shift+L");
}

#[test]
fn stt_provider_validation_accepts_supported_values_only() {
    assert!(validate_stt_provider("deepgram").is_ok());
    assert!(validate_stt_provider("whisper").is_ok());
    assert!(validate_stt_provider("assemblyai").is_err());
}

#[test]
fn stt_model_validation_trims_and_rejects_blank_values() {
    assert_eq!(validate_stt_model(" nova-2 ").unwrap(), "nova-2");
    assert_eq!(
        validate_stt_model(" ggml-base.bin ").unwrap(),
        "ggml-base.bin"
    );
    assert!(validate_stt_model("   ").is_err());
}

#[test]
fn stt_provider_validation_accepts_sherpa() {
    assert!(validate_stt_provider("sherpa").is_ok());
    assert!(validate_stt_provider("assemblyai").is_err());
}

#[test]
fn sherpa_model_validation_uses_the_sherpa_catalog() {
    assert_eq!(
        validate_sherpa_model(" sense-voice ").unwrap(),
        "sense-voice"
    );
    assert!(validate_sherpa_model("ggml-base.bin").is_err());
    assert!(validate_sherpa_model("../x").is_err());
}

#[test]
fn provider_switch_to_sherpa_defaults_the_model() {
    let mut models = ModelPrefs {
        stt_provider: "deepgram".into(),
        stt_model: "nova-2".into(),
        ..Default::default()
    };
    apply_stt_config(
        &mut models,
        "models.stt_provider",
        &serde_json::json!("sherpa"),
    )
    .unwrap();
    assert_eq!(models.stt_provider, "sherpa");
    assert_eq!(models.stt_model, "sense-voice");
}

#[test]
fn stt_model_write_validates_against_the_active_provider() {
    let mut models = ModelPrefs {
        stt_provider: "sherpa".into(),
        stt_model: "sense-voice".into(),
        ..Default::default()
    };
    assert!(apply_stt_config(&mut models, "models.stt_model", &serde_json::json!("tiny")).is_err());
    assert_eq!(models.stt_model, "sense-voice");
}

#[test]
fn recording_keys_apply_and_validate() {
    let mut rec = RecordingPrefs::default();
    assert!(apply_recording_config(
        &mut rec,
        "recording.auto_screenshots",
        &serde_json::json!(false)
    )
    .unwrap());
    assert!(!rec.auto_screenshots);
    assert!(apply_recording_config(&mut rec, "recording.fps", &serde_json::json!(8)).unwrap());
    assert_eq!(rec.fps, 8);
    assert_eq!(
        apply_recording_config(&mut rec, "recording.fps", &serde_json::json!(5)).unwrap_err(),
        "unknown fps 5"
    );
    assert!(apply_recording_config(
        &mut rec,
        "recording.summary_prompt",
        &serde_json::json!("  custom  ")
    )
    .unwrap());
    assert_eq!(rec.summary_prompt, "custom");
    assert!(!apply_recording_config(&mut rec, "recording.other", &serde_json::json!(1)).unwrap());
    assert_eq!(validate_main_language(" zh ").unwrap(), "zh");
    assert_eq!(
        validate_main_language("cn").unwrap_err(),
        "unknown language \"cn\""
    );
}

#[test]
fn recording_read_interval_defaults_and_sets() {
    let prefs = RecordingPrefs::default();
    assert_eq!(prefs.read_interval_secs, 3);
    let mut cfg = Config::default();
    assert!(apply_recording_config(
        &mut cfg.recording,
        "recording.read_interval_secs",
        &serde_json::json!(5)
    )
    .unwrap());
    assert_eq!(cfg.recording.read_interval_secs, 5);
    assert!(apply_recording_config(
        &mut cfg.recording,
        "recording.read_interval_secs",
        &serde_json::json!(0)
    )
    .is_err());
}

#[test]
fn normalize_clamps_language_and_fps() {
    let mut cfg = Config::default();
    cfg.app.main_language = "klingon".into();
    cfg.recording.fps = 60;
    cfg.recording.summary_prompt = "  pad  ".into();
    cfg.normalize();
    assert_eq!(cfg.app.main_language, "en");
    assert_eq!(cfg.recording.fps, 4);
    assert_eq!(cfg.recording.summary_prompt, "pad");
}

#[test]
fn prompts_custom_roundtrips_and_normalizes() {
    // Round-trip: a custom preset survives save/load.
    let dir = tempfile_dir();
    let path = dir.join("config.toml");
    let mut cfg = Config::default();
    cfg.prompts.custom = vec![crate::presets::Preset {
        id: "u:test".into(),
        name: "Test".into(),
        text: "Be terse.".into(),
    }];
    cfg.save_to(&path).unwrap();
    let loaded = Config::load_from(&path).unwrap();
    assert_eq!(loaded.prompts.custom.len(), 1);
    assert_eq!(loaded.prompts.custom[0].id, "u:test");

    // A hand-edited file keeps valid rows and drops malformed ones.
    // (The stale `kind` keys double as a compatibility check — the
    // older schema's field is ignored on load, not an error.)
    std::fs::write(
        &path,
        "[[prompts.custom]]\n\
             id = \"u:ok\"\nname = \"Ok\"\nkind = \"instruct\"\ntext = \"t\"\n\
             [[prompts.custom]]\n\
             id = \"\"\nname = \"bad\"\nkind = \"instruct\"\ntext = \"t\"\n\
             [[prompts.custom]]\n\
             id = \"u:ok\"\nname = \"dup\"\nkind = \"instruct\"\ntext = \"t\"\n",
    )
    .unwrap();
    let loaded = Config::load_from(&path).unwrap();
    assert_eq!(loaded.prompts.custom.len(), 1);
    assert_eq!(loaded.prompts.custom[0].id, "u:ok");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prompts_custom_write_validates() {
    let mut prompts = PromptPrefs::default();
    let good = serde_json::json!([{
        "id": "u:a1", "name": "A", "text": "do {input}"
    }]);
    assert!(apply_prompts_config(&mut prompts, "prompts.custom", &good).unwrap());
    assert_eq!(prompts.custom[0].text, "do {input}");
    assert!(!apply_prompts_config(&mut prompts, "other.key", &serde_json::json!([])).unwrap());

    let bad = serde_json::json!([{ "id": "u:a1", "name": "", "text": "x" }]);
    assert!(apply_prompts_config(&mut prompts, "prompts.custom", &bad).is_err());
}

#[test]
fn memory_defaults_disabled_and_roundtrips() {
    let tmp = tempfile_dir();
    let path = tmp.join("config.toml");
    let mut cfg = Config::default();

    assert!(!cfg.memory.enabled);
    assert!(cfg.memory.provider.is_empty());
    assert!(cfg.memory.model.is_empty());

    cfg.memory.enabled = true;
    cfg.memory.provider = "ollama".into();
    cfg.memory.model = "qwen3:8b".into();
    cfg.save_to(&path).unwrap();

    let loaded = Config::load_from(&path).unwrap();
    assert_eq!(loaded.memory, cfg.memory);
    let _ = std::fs::remove_dir_all(tmp);
}

#[test]
fn memory_config_rejects_invalid_enable_and_provider() {
    let mut memory = MemoryPrefs::default();

    assert_eq!(
        apply_memory_config(&mut memory, "memory.enabled", &serde_json::json!(true),).unwrap_err(),
        "memory.provider and memory.model must be set before enabling memory"
    );
    assert_eq!(
        apply_memory_config(
            &mut memory,
            "memory.provider",
            &serde_json::json!("unknown"),
        )
        .unwrap_err(),
        "unknown memory provider \"unknown\""
    );
    assert!(
        !apply_memory_config(&mut memory, "memory.other", &serde_json::json!("value"),).unwrap()
    );
}

#[test]
fn memory_config_only_allows_clearing_selection_while_disabled() {
    for key in ["memory.provider", "memory.model"] {
        for blank in ["", " \t\n "] {
            let mut memory = MemoryPrefs {
                enabled: true,
                provider: "openai".into(),
                model: "gpt-4o".into(),
            };
            let original = memory.clone();
            assert!(apply_memory_config(&mut memory, key, &serde_json::json!(blank)).is_err());
            assert_eq!(
                memory, original,
                "a rejected write must not mutate preferences"
            );

            apply_memory_config(&mut memory, "memory.enabled", &serde_json::json!(false)).unwrap();
            assert!(apply_memory_config(&mut memory, key, &serde_json::json!(blank)).unwrap());
            assert!(if key == "memory.provider" {
                memory.provider.is_empty()
            } else {
                memory.model.is_empty()
            });
            assert!(
                apply_memory_config(&mut memory, "memory.enabled", &serde_json::json!(true))
                    .is_err()
            );
        }
    }
}

#[test]
fn memory_normalization_disables_incomplete_hand_edited_config() {
    let tmp = tempfile_dir();
    let path = tmp.join("config.toml");
    std::fs::write(
        &path,
        "[memory]\nenabled = true\nprovider = \"openai\"\nmodel = \"\"\n",
    )
    .unwrap();

    let cfg = Config::load_from(&path).unwrap();
    assert!(!cfg.memory.enabled);
    assert_eq!(cfg.memory.provider, "openai");
    assert!(cfg.memory.model.is_empty());
    let _ = std::fs::remove_dir_all(tmp);
}

#[test]
fn disabled_providers_keep_only_the_first_known_occurrence() {
    let mut config = Config::default();
    config.providers.disabled = ["openai", "gemini", "openai", "unknown", "gemini"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    config.normalize();
    assert_eq!(config.providers.disabled, ["openai", "gemini"]);
}

#[test]
fn invalid_config_is_backed_up_before_default_fallback() {
    let dir = tempfile_dir();
    let path = dir.join("config.toml");
    let invalid = b"[app\ninvalid = \xff";
    std::fs::write(&path, invalid).unwrap();
    assert_eq!(load_or_default(&path), Config::default());
    assert_eq!(std::fs::read(dir.join("config.toml.bak")).unwrap(), invalid);
    assert_eq!(std::fs::read(path).unwrap(), invalid);
    std::fs::remove_dir_all(dir).unwrap();
}
