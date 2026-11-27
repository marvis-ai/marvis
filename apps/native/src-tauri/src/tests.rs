use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

/// Unique temp dir per test; `Db::at` creates it.
fn tmp_dir() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!(
        "marvis-lib-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn audio_export_preserves_original_and_copies_to_other_destinations() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("recording.wav");
    let contents = b"original recording contents";
    std::fs::write(&source, contents).unwrap();
    let source = source.canonicalize().unwrap();
    assert!(copy_audio_export(&source, &source).is_err());
    #[cfg(unix)]
    {
        let alias = dir.join("alias.wav");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        assert!(copy_audio_export(&source, &alias).is_err());
    }
    assert_eq!(std::fs::read(&source).unwrap(), contents);
    let destination = dir.join("export.wav");
    copy_audio_export(&source, &destination).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), contents);
    std::fs::write(&destination, b"old export").unwrap();
    copy_audio_export(&source, &destination).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), contents);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn removed_listen_placeholder_is_absent_from_command_contract() {
    // Keep this tied to the actual registration source rather than a
    // second hand-maintained list of command names.
    assert!(!include_str!("lib.rs").contains(concat!("listen_", "stub")));
    let source = include_str!("lib.rs");
    assert!(source.contains("listen_start,"));
    assert!(source.contains("listen_stop,"));
    assert!(source.contains("listen_pause,"));
    assert!(source.contains("listen_resume,"));
    assert!(source.contains("listen_status,"));
    assert!(source.contains("dictation_start,"));
    assert!(source.contains("dictation_stop,"));
    assert!(source.contains("dictation_status,"));
    assert!(source.contains("whisper_status,"));
    assert!(source.contains("sherpa_status,"));
    assert!(source.contains("sherpa_download,"));
    assert!(source.contains("sherpa_cancel_download,"));
    assert!(source.contains("sherpa_remove_model,"));
    assert!(source.contains("session_resume,"));
}

/// The composer's stop button plus the packet identity that keeps a
/// stream out of the wrong conversation: `ask_stop` cancels ONE
/// session's run through `AskService::stop`. Deletion also cancels
/// its run; ending/resuming sessions only detaches the view, so a
/// detached run keeps writing to its own session. The emit fold
/// `run`-tags every `ask:*` packet
/// (in-flight deliveries of a killed run drop) and `send_chain`
/// `session_id`-tags them (a detached run's packets drop on any
/// view that isn't showing that session). `ask_runs` is the
/// per-session resync replacing the single-run `ask_current`.
#[test]
fn ask_stop_and_packet_tagging_are_in_the_contract() {
    let lib = include_str!("lib.rs");
    assert!(lib.contains(concat!("ask", "_stop,")));
    assert!(lib.contains(concat!("ask", "_runs,")));
    let commands = include_str!("commands/ask.rs");
    assert!(commands.contains("pub(crate) fn ask_stop("));
    assert!(commands.contains("state.ask.stop(&app, session_id)"));
    assert!(commands.contains("pub(crate) fn ask_runs("));
    let sessions = include_str!("commands/sessions.rs");
    assert!(sessions.contains("state.ask.stop(&app, id)"));
    let view_boundaries = sessions
        .split("pub(crate) fn session_end_active")
        .nth(1)
        .unwrap();
    assert!(!view_boundaries.contains("state.ask.stop"));
    let ask = include_str!("ask/mod.rs");
    assert!(ask.contains("pub fn stop(&self, app: &AppHandle, session_id: i64)"));
    assert!(ask.contains("payload[\"run\"]"));
    let pipeline = include_str!("ask/pipeline.rs");
    assert!(pipeline.contains("payload[\"session_id\"]"));
}

/// The preset surface: `presets_list` plus the palette commands
/// registered (open/select/close plus the `/`-session plumbing —
/// key forward, query push, visibility probe), and the palette
/// event constants declared — all asserted against the
/// registration source itself. `concat!` keeps each literal out
/// of this file's text (same trick as the `concat!("listen_",
/// "stub")` negative assert above) so the asserts can't
/// self-satisfy.
#[test]
fn preset_commands_and_dispatch_are_in_the_contract() {
    let source = include_str!("lib.rs");
    assert!(source.contains(concat!("presets", "_list,")));
    assert!(source.contains(concat!("presets", "_palette_open,")));
    assert!(source.contains(concat!("presets", "_palette_select,")));
    assert!(source.contains(concat!("presets", "_palette_close,")));
    assert!(source.contains(concat!("presets", "_palette_key,")));
    assert!(source.contains(concat!("presets", "_palette_query,")));
    assert!(source.contains(concat!("presets", "_palette_height,")));
    let dispatch = include_str!("dispatch.rs");
    assert!(dispatch.contains(concat!("EV_", "PRESET_PICK")));
    assert!(dispatch.contains(concat!("EV_", "PALETTE_OPEN")));
    assert!(dispatch.contains(concat!("EV_", "PALETTE_QUERY")));
    assert!(dispatch.contains(concat!("EV_", "PALETTE_KEY")));
    assert!(dispatch.contains(concat!("EV_", "PALETTE_CLOSED")));
}

/// The export surface: `save_text_file` is registered in
/// `generate_handler!`, gate-guarded like the palette commands, and
/// runs its blocking dialog off the async executor. `concat!` keeps
/// the literal out of this file's text so the assert can't
/// self-satisfy.
#[test]
fn export_command_is_registered_and_gate_guarded() {
    let source = include_str!("lib.rs");
    assert!(source.contains(concat!("save", "_text_file,")));
    let body = include_str!("commands/files.rs")
        .split("async fn save_text_file(")
        .nth(1)
        .and_then(|rest| rest.split("\n/// ").next())
        .expect("save_text_file body not found");
    assert!(
        body.contains("*state.gate.lock() != Gate::Main"),
        "save_text_file must check Gate::Main"
    );
    assert!(
        body.contains("spawn_blocking"),
        "save_text_file's dialog must run off the async executor"
    );
}

#[test]
fn audio_export_command_is_registered_and_guarded() {
    let source = include_str!("lib.rs");
    assert!(source.contains(concat!("save", "_audio_file,")));
    let body = include_str!("commands/files.rs")
        .split("async fn save_audio_file(")
        .nth(1)
        .and_then(|rest| rest.split("\n/// ").next())
        .expect("save_audio_file body not found");
    assert!(body.contains("*state.gate.lock() != Gate::Main"));
    assert!(body.contains("spawn_blocking"));
    assert!(body.contains("audio_file"));
    assert!(!body.contains("source_path"));
}

#[test]
fn suggested_name_sanitizes_separators_controls_and_caps() {
    assert_eq!(sanitize_suggested_name("a/b\\c.md", "marvis-export.md"), "abc.md");
    assert_eq!(sanitize_suggested_name("n\u{0}ame.md", "marvis-export.md"), "name.md");
    assert_eq!(sanitize_suggested_name("   ", "marvis-export.md"), "marvis-export.md");
    assert_eq!(sanitize_suggested_name("", "marvis-export.md"), "marvis-export.md");
    assert_eq!(sanitize_suggested_name(&"x".repeat(200), "marvis-export.md").len(), 80);
    assert_eq!(sanitize_suggested_name("notes.md", "marvis-export.md"), "notes.md");
    assert_eq!(sanitize_suggested_name(" /\\ ", "marvis-export.wav"), "marvis-export.wav");
}

/// The punctuation auto-install chain: a completed sherpa download
/// task clears its own `active` slot then re-enters
/// `refresh_speech_setup`, which calls `ensure_punct` — the unit
/// tests drive `ensure_punct` directly, so this asserts both halves
/// of the re-entrant wiring stay connected.
#[test]
fn punct_chain_reenters_refresh_and_refresh_calls_ensure_punct() {
    assert!(include_str!("sherpa_models/manager.rs").contains("crate::refresh_speech_setup(app)"));
    assert!(include_str!("commands/dictation.rs").contains("ensure_punct("));
}

/// Both speech services must route providers through the shared
/// curated setup check — a missing model download or key is a
/// Settings fix surfaced as `needs_setup`, never a raw engine/path
/// error.
#[test]
fn sherpa_setup_check_is_wired_into_both_speech_services() {
    assert!(include_str!("dictation/mod.rs").contains("stt_setup_error("));
    assert!(include_str!("listen/mod.rs").contains("stt_setup_error("));
}

#[test]
fn app_exit_registration_covers_requested_and_completed_tauri_exit() {
    let source = include_str!("lib.rs");
    assert!(source.contains("tauri::RunEvent::ExitRequested { .. }"));
    assert!(source.contains("| tauri::RunEvent::Exit"));
    assert!(source.contains("stop_speech(app);"));
}

/// The app menubar keeps its contract: About carries `version
/// (commit-count build)` + the copyright line, Settings and the
/// Position items reuse the shared `menu.*` ids so the single
/// dispatcher covers them, and the persistent menubar's Position
/// checks are re-synced from the shared refresh funnel (it is never
/// rebuilt like the tray copy).
#[test]
fn app_menubar_keeps_about_settings_and_position_wiring() {
    let menubar = include_str!("menubar.rs");
    assert!(menubar.contains("MARVIS_BUILD_NUMBER"));
    assert!(menubar.contains("© 2026 Marvis AI LLC"));
    assert!(menubar.contains("menus::MENU_SETTINGS"));
    assert!(menubar.contains("position_submenu"));
    // The View menu also carries the Lock check — synced by the
    // same funnel — and dodges AppKit's "Enter Full Screen"
    // injection via a zero-width-space title.
    assert!(menubar.contains("lock_item"));
    assert!(menubar.contains("menus::MENU_LOCK"));
    assert!(menubar.contains("View\\u{200B}"));
    // Help menu: native `HELP_SUBMENU_ID` (macOS adds its search
    // field) + the support mailto through the shared dispatcher.
    assert!(menubar.contains("HELP_SUBMENU_ID"));
    assert!(menubar.contains("menus::MENU_SUPPORT"));
    assert!(menubar.contains("mailto:support@getmarvis.com"));
    let chrome = include_str!("chrome.rs");
    assert!(chrome.contains("menubar::sync_position_checks"));
    let dispatch = include_str!("dispatch.rs");
    assert!(dispatch.contains("menus::MENU_SUPPORT =>"));
    // The shared menu and the menubar must build their Position
    // submenus from the same helper — two hand-maintained copies of
    // the `menu.pos.*` items would silently drift.
    let menus = include_str!("menus.rs");
    assert!(menus.contains("position_submenu(app, edge)?"));
}

/// The speech commands keep their documented guards in source: both
/// start commands gate on `Main`, and each refuses to run while the
/// other mode's durable status is `listening` (mutual exclusion must
/// hold even for a stale/racing invoke).
#[test]
fn speech_commands_keep_gate_and_mutual_exclusion_guards() {
    let source = concat!(
        include_str!("commands/listen.rs"),
        include_str!("commands/dictation.rs")
    );
    assert!(source.contains("Dictation is unavailable until setup is complete"));
    assert!(source.contains("state.listen.status().is_listening()"));
    assert!(source.contains("state.dictation.status().is_listening()"));
    assert!(source.contains("dictation:state"));
    assert!(source.contains("dictation:draft"));
    assert!(source.contains("dictation:error"));
}

/// Both start commands must hold `speech_lifecycle` across their
/// peer check, the mic-permission await, and the service start —
/// that's what makes the mutual exclusion atomic against a
/// concurrent first-start (the check-then-act gap is what raced).
#[test]
fn speech_starts_share_a_lifecycle_lock() {
    let source = include_str!("state.rs");
    assert!(source.contains("speech_lifecycle: tokio::sync::Mutex<()>"));
    for (source, signature) in [
        (include_str!("commands/listen.rs"), "async fn listen_start"),
        (include_str!("commands/dictation.rs"), "async fn dictation_start"),
    ] {
        let body = source
            .split(signature)
            .nth(1)
            .and_then(|rest| rest.split("\n#[tauri::command").next())
            .unwrap_or_else(|| panic!("{signature} body not found"));
        assert!(
            body.contains("state.speech_lifecycle.lock().await"),
            "{signature} must hold the speech lifecycle lock across the await"
        );
    }
}

/// `capture_start` keeps the same crafted-invoke guard as
/// `ask_send`/`ask_send_screen_only`: it mutates only while the gate
/// is `Main`, and holds `gate_transition` across the check + start
/// so a racing `transition_gate` can't interleave between them.
/// `capture_stop` deliberately stays ungated — stopping must always
/// be safe.
#[test]
fn capture_start_is_gate_guarded_but_capture_stop_is_not() {
    let source = include_str!("commands/capture.rs");
    let start_body = source
        .split("fn capture_start(app: AppHandle)")
        .nth(1)
        .and_then(|rest| rest.split("\n#[tauri::command").next())
        .expect("capture_start body not found");
    assert!(
        start_body.contains("*state.gate.lock() != Gate::Main"),
        "capture_start must drop the invoke while gate != Main"
    );
    let transition_lock = start_body
        .find("state.gate_transition.try_lock()")
        .expect("capture_start must hold gate_transition across check + start");
    let gate_check = start_body.find("*state.gate.lock() != Gate::Main").unwrap();
    let start_call = start_body
        .find("start_capture(&app, source, w, h, None)")
        .expect("capture_start must resolve the display filter into start_capture");
    assert!(
        transition_lock < gate_check && transition_lock < start_call,
        "gate_transition must be taken before the gate check and start_capture"
    );
    let stop_body = source
        .split("fn capture_stop(app: AppHandle)")
        .nth(1)
        .and_then(|rest| rest.split("\n#[tauri::command").next())
        .expect("capture_stop body not found");
    assert!(
        !stop_body.contains("Gate::Main"),
        "capture_stop must stay callable at any gate"
    );
}

/// `capture_pick_and_start` mutates (the picked scope becomes the
/// live capture), so it keeps the same crafted-invoke gate guard as
/// `capture_start`, and the `Picked` callback re-checks the gate
/// under `gate_transition` — a pick must never light capture
/// outside `Main`. `try_lock`, not `lock`: the callback runs on the
/// main thread and a `gate_transition` holder blocks on it via
/// `tray.set_menu`, so parking here deadlocks. The body is bounded
/// on the next `fn` so the assertions can't leak into neighbouring
/// tests.
#[test]
fn capture_pick_and_start_is_gate_guarded() {
    let src = include_str!("commands/capture.rs");
    let body = src
        .split("fn capture_pick_and_start")
        .nth(1)
        .and_then(|rest| rest.split("\nfn ").next())
        .expect("command exists");
    assert!(
        body.contains("*state.gate.lock() != Gate::Main"),
        "picker start must be Main-gated"
    );
    assert!(
        body.contains("gate_transition.try_lock()"),
        "the picked callback must re-check the gate under gate_transition \
         via try_lock (main-thread callback must never park on it)"
    );
}

/// Marvis's windows stay user-capturable on macOS (no
/// `set_content_protected`), so the picker's own filter — which
/// carries no self-exclusion — would composite our bar into a
/// DISPLAY pick's recording. The display arm must re-resolve
/// through `resolve_candidate` (`d:` = display minus own windows);
/// window/app picks can't be ours (`excluded_bundle_ids`).
#[test]
fn capture_pick_and_start_reresolves_display_picks() {
    let src = include_str!("commands/capture.rs");
    let body = src
        .split("fn capture_pick_and_start")
        .nth(1)
        .and_then(|rest| rest.split("\nfn ").next())
        .expect("command exists");
    assert!(
        body.contains("resolve_candidate(&format!(\"d:{id}\"))"),
        "a display pick must rebuild the filter via resolve_candidate (self-excluding)"
    );
    assert!(
        body.contains("set_excluded_bundle_ids"),
        "Marvis must stay excluded from the picker's offer list"
    );
}

/// Windows parity with the macOS rule "visible to the user,
/// invisible to Marvis's own captures": WGC can't exclude windows
/// from a monitor grab, so `capture/windows.rs` applies
/// `WDA_EXCLUDEFROMCAPTURE` dynamically — a refcounted guard holds
/// it only while a capture session is live, then restores
/// `WDA_NONE`. `build_window` must consult `protection_engaged`
/// (not protect unconditionally) so a window born mid-capture
/// starts protected without hiding Marvis from user screenshots
/// the rest of the time.
#[test]
fn windows_self_protects_only_while_capturing() {
    let backend = include_str!("capture/windows.rs");
    for needle in [
        "struct SelfProtection",
        "SetWindowDisplayAffinity",
        "WDA_EXCLUDEFROMCAPTURE",
        "WDA_NONE",
        "pub(crate) fn protection_engaged()",
    ] {
        assert!(backend.contains(needle), "windows backend must keep {needle}");
    }
    let pool = include_str!("windows/build.rs");
    assert!(
        pool.contains("set_content_protected(crate::capture::protection_engaged())"),
        "windows builds must consult protection_engaged, not protect unconditionally"
    );
}

/// The custom picker's begin/select share `capture_start`'s
/// crafted-invoke guard: `gate_transition` held across the check +
/// action so a racing leave-Main can't interleave. `select`
/// additionally stops a live capture first — `start_capture` is
/// idempotent and would silently keep the old scope.
#[test]
fn capture_pick_commands_are_gate_guarded() {
    let source = include_str!("commands/capture.rs");
    let body = |sig: &str| {
        source
            .split(sig)
            .nth(1)
            .and_then(|rest| rest.split("\n#[tauri::command").next())
            .unwrap_or_else(|| panic!("{sig} body not found"))
    };
    for sig in [
        "fn capture_pick_begin(app: AppHandle)",
        "fn capture_pick_list(app: AppHandle)",
        "fn capture_pick_select(app: AppHandle, id: String)",
    ] {
        let b = body(sig);
        let lock = b
            .find("state.gate_transition.try_lock()")
            .unwrap_or_else(|| panic!("{sig} must hold gate_transition"));
        let check = b
            .find("*state.gate.lock() != Gate::Main")
            .unwrap_or_else(|| panic!("{sig} must check Gate::Main"));
        assert!(lock < check, "{sig}: lock must precede the check");
    }
    let select = body("fn capture_pick_select(app: AppHandle, id: String)");
    let stop = select
        .find("stop_capture(&app)")
        .expect("select must stop a live capture before retargeting");
    let start = select
        .find("start_capture_with_resolver(&app")
        .expect("select must start_capture with the resolved filter");
    assert!(stop < start, "select must stop before starting");
}

/// `leave_main` must stop dictation: the session is bound to the
/// ask input that leaving Main hides, so an invisible dictation
/// must not keep the microphone. Listen is deliberately untouched —
/// meeting Listen is independent of card visibility.
#[test]
fn leave_main_stops_dictation_but_not_listen() {
    let source = include_str!("gate.rs");
    let body = source
        .split("fn leave_main(app: &AppHandle)")
        .nth(1)
        .and_then(|rest| rest.split("\nfn ").next())
        .expect("leave_main body not found");
    assert!(body.contains("state.dictation.stop()"));
    assert!(!body.contains("state.listen.stop()"));
}

/// Mutual exclusion keys off the durable `listening` state of each
/// service — `error`/`idle` must not block the other mode.
#[test]
fn speech_status_predicates_only_match_listening() {
    assert!(!listen::ListenStatus {
        state: "idle".into(),
        provider: None,
        session_id: None,
        audio_file: None,
        turns: 0,
        mic: false,
        error: None,
        started_at: None,
        paused_secs: 0,
        paused_since: None,
    }
    .is_listening());
    assert!(listen::ListenStatus {
        state: "listening".into(),
        provider: None,
        session_id: None,
        audio_file: None,
        turns: 0,
        mic: false,
        error: None,
        started_at: None,
        paused_secs: 0,
        paused_since: None,
    }
    .is_listening());
    // A paused Listen still owns the audio sources — mutual exclusion
    // must hold through the pause.
    assert!(listen::ListenStatus {
        state: "paused".into(),
        provider: None,
        session_id: None,
        audio_file: None,
        turns: 0,
        mic: false,
        error: None,
        started_at: None,
        paused_secs: 0,
        paused_since: None,
    }
    .is_listening());

    assert!(!dictation::DictationStatus {
        state: "error".into(),
        provider: None,
        error: None,
    }
    .is_listening());
    assert!(dictation::DictationStatus {
        state: "listening".into(),
        provider: None,
        error: None,
    }
    .is_listening());
}

/// The dictation service is wired into `AppState` alongside Listen and
/// starts idle; stop is safe on a fresh service.
#[test]
fn dictation_service_is_wired_into_app_state() {
    let tmp = tmp_dir();
    let state = AppState::for_test(&tmp);
    let status = state.dictation.status();
    assert_eq!(status.state, "idle");
    assert_eq!(status.provider, None);
    assert_eq!(status.error, None);
    let draft = state.dictation.stop();
    assert_eq!(draft.text, "");
    assert!(draft.finality);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Brief smoke test: `config_get` against a temp `AppState` returns
/// defaults — proves `for_test` builds without a runtime and every
/// field is wired.
#[test]
fn config_get_returns_defaults_on_temp_state() {
    let tmp = tmp_dir();
    let state = AppState::for_test(&tmp);
    assert_eq!(state.config.lock().clone(), Config::default());
    // Fresh dir → onboarding isn't done, so the gate can't be `Main`
    // even if this CI machine happens to have screen permission.
    assert_eq!(app_gate(&state), Gate::NeedsPermission);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The `capture:state` contract carries the picker-selected scope:
/// `target` is `null` on the auto primary-display path and the
/// `{"kind","label"}` pair after a picker selection — the picker's
/// status UI (Task 7) keys off exactly this shape.
#[test]
fn capture_snapshot_serializes_the_picker_target() {
    let tmp = tmp_dir();
    let state = AppState::for_test(&tmp);
    let snap = capture_snapshot(&state);
    assert_eq!(snap["running"], false);
    assert_eq!(snap["frames"], 0);
    assert!(snap["target"].is_null(), "auto path reports null target");

    *state.capture_target.lock() = Some(CaptureTarget {
        kind: "window",
        label: "Finder".into(),
    });
    let snap = capture_snapshot(&state);
    assert_eq!(snap["target"]["kind"], "window");
    assert_eq!(snap["target"]["label"], "Finder");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The failover chain honours `providers.order`, skips disabled ids,
/// and drops providers with no key (where required) or no model.
#[test]
fn deepgram_keys_are_trimmed_and_unverified() {
    assert_eq!(
        normalize_deepgram_key("  dg-test-key \n").unwrap(),
        "dg-test-key"
    );

    let accepted = deepgram_validation_payload("  dg-test-key \t");
    assert_eq!(accepted["ok"], true);
    assert!(accepted["message"]
        .as_str()
        .unwrap()
        .contains("no live provider probe"));

    let rejected = deepgram_validation_payload(" \t\n");
    assert_eq!(rejected["ok"], false);
    assert!(rejected["error"]
        .as_str()
        .unwrap()
        .contains("after trimming"));
}

#[test]
fn deepgram_is_key_only_and_never_enters_llm_chain() {
    assert!(is_key_management_provider("deepgram"));
    assert!(!is_key_management_provider("not-a-provider"));

    let tmp = tmp_dir();
    let state = AppState::for_test(&tmp);
    {
        let mut ks = state.keystore.lock();
        ks.set_key("deepgram", "dg-test-key").unwrap();
    }
    let (cfg, ks) = (state.config.lock(), state.keystore.lock());
    assert!(provider_candidates(&cfg, &ks)
        .iter()
        .all(|c| c.id != "deepgram"));
    assert_eq!(
        ks.masked_status(),
        vec![("deepgram".into(), Some("…-key".into()))]
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn provider_chain_respects_order_enablement_and_usability() {
    let tmp = tmp_dir();
    let state = AppState::for_test(&tmp);
    {
        let mut ks = state.keystore.lock();
        ks.set_key("openai", "sk-a").unwrap();
        ks.set_key("gemini", "AIza-b").unwrap();
    }
    {
        let mut cfg = state.config.lock();
        cfg.providers.order = vec![
            "openai".into(),
            "gemini".into(),
            "anthropic".into(),
            "openrouter".into(),
            "ollama".into(),
            "compatible".into(),
        ];
        cfg.providers.disabled = vec!["gemini".into()];
    }
    let (cfg, ks) = (state.config.lock(), state.keystore.lock());
    let chain = provider_candidates(&cfg, &ks);
    let ids: Vec<&str> = chain.iter().map(|c| c.id.as_str()).collect();
    // openai (keyed) first; gemini disabled; anthropic/openrouter
    // dropped (no key); ollama/compatible dropped (no model set:
    // their static list is empty).
    assert_eq!(ids, vec!["openai"]);
    assert_eq!(chain[0].model, "gpt-4o");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A selected-but-disabled or keyless provider yields to the next
/// usable entry — this is the fallback the user asked for.
#[test]
fn provider_chain_falls_back_past_unusable_providers() {
    let tmp = tmp_dir();
    let state = AppState::for_test(&tmp);
    {
        let mut ks = state.keystore.lock();
        ks.set_key("gemini", "AIza-b").unwrap();
        ks.set_key("anthropic", "sk-ant-c").unwrap();
    }
    {
        let mut cfg = state.config.lock();
        // openai first but no key; gemini disabled; anthropic wins.
        cfg.providers.disabled = vec!["gemini".into()];
    }
    let (cfg, ks) = (state.config.lock(), state.keystore.lock());
    let chain = provider_candidates(&cfg, &ks);
    let ids: Vec<&str> = chain.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["anthropic"]);
    assert_eq!(chain[0].model, "claude-sonnet-4-5");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Ollama and a configured compatible endpoint need no key — both
/// stay in the chain once they can resolve a model.
#[test]
fn provider_chain_keeps_keyless_providers_with_models() {
    let tmp = tmp_dir();
    let state = AppState::for_test(&tmp);
    {
        let mut cfg = state.config.lock();
        cfg.providers
            .models
            .insert("ollama".into(), "qwen3:8b".into());
        cfg.providers
            .models
            .insert("compatible".into(), "llama-3.3-70b".into());
        cfg.compat.base_url = "http://localhost:8000/v1".into();
    }
    let (cfg, ks) = (state.config.lock(), state.keystore.lock());
    let ids: Vec<String> = provider_candidates(&cfg, &ks)
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(ids, vec!["ollama", "compatible"]);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A compatible provider with no `base_url` configured is unusable —
/// it must not enter the chain on key-memory alone.
#[test]
fn provider_chain_requires_a_base_url_for_compatible() {
    let tmp = tmp_dir();
    let state = AppState::for_test(&tmp);
    {
        let mut cfg = state.config.lock();
        cfg.providers
            .models
            .insert("compatible".into(), "llama-3.3-70b".into());
    }
    let (cfg, ks) = (state.config.lock(), state.keystore.lock());
    let ids: Vec<String> = provider_candidates(&cfg, &ks)
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(ids, Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The screen reader is off until `vision.provider` names a usable
/// pick — keyed for hosted, `compat.base_url` + a model for
/// compatible — and resolves its model from `vision.models` or the
/// vision default.
#[test]
fn vision_candidate_resolves_only_when_usable() {
    let tmp = tmp_dir();
    let state = AppState::for_test(&tmp);
    {
        let mut ks = state.keystore.lock();
        ks.set_key("gemini", "AIza-b").unwrap();
    }
    let (cfg, ks) = (state.config.lock(), state.keystore.lock());

    // Unset → off.
    assert!(vision_candidate(&cfg, &ks).is_none());

    // A keyed provider resolves its vision default model.
    let mut cfg = cfg.clone();
    cfg.vision.provider = "gemini".into();
    let c = vision_candidate(&cfg, &ks).unwrap();
    assert_eq!(c.id, "gemini");
    assert_eq!(c.model, "gemini-2.5-flash");

    // `vision.models` beats the default.
    cfg.vision
        .models
        .insert("gemini".into(), "gemini-2.5-pro".into());
    assert_eq!(vision_candidate(&cfg, &ks).unwrap().model, "gemini-2.5-pro");

    // A hosted pick with no key is unusable → off.
    cfg.vision.provider = "openai".into();
    assert!(vision_candidate(&cfg, &ks).is_none());
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The reader ignores `providers.disabled` — a chat-disabled provider
/// may still read the screen. Compatible additionally needs the
/// shared endpoint and an explicit model id.
#[test]
fn vision_candidate_ignores_disabled_and_supports_compatible() {
    let tmp = tmp_dir();
    let state = AppState::for_test(&tmp);
    {
        let mut ks = state.keystore.lock();
        ks.set_key("openai", "sk-a").unwrap();
    }
    let mut cfg = state.config.lock().clone();
    cfg.providers.disabled = vec!["openai".into()];
    cfg.vision.provider = "openai".into();
    let ks = state.keystore.lock();
    let c = vision_candidate(&cfg, &ks).unwrap();
    assert_eq!(c.id, "openai");

    // Compatible: no endpoint → unusable; endpoint but no model →
    // unusable; both → resolves the typed id.
    cfg.vision.provider = "compatible".into();
    assert!(vision_candidate(&cfg, &ks).is_none());
    cfg.compat.base_url = "http://localhost:8000/v1".into();
    assert!(vision_candidate(&cfg, &ks).is_none());
    cfg.vision
        .models
        .insert("compatible".into(), "qwen2.5-vl".into());
    assert_eq!(vision_candidate(&cfg, &ks).unwrap().model, "qwen2.5-vl");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `ask_send`'s attachment input is part of the command contract:
/// the command takes the webview's normalized JPEGs, the input
/// struct lives in ask.rs, and its serde shape accepts the
/// camelCase `jpegBase64` key the bridge sends.
#[test]
fn ask_attachment_contract_is_registered() {
    let source = include_str!("commands/ask.rs");
    let ask = include_str!("ask/mod.rs");
    assert!(source.contains("attachments: Option<Vec<ask::AskAttachmentInput>>"));
    assert!(ask.contains(concat!("pub struct AskAttachment", "Input")));
    let parsed: crate::ask::AskAttachmentInput = serde_json::from_value(
        json!({"name": "a.png", "jpegBase64": "AA=="}),
    )
    .expect("jpegBase64 must deserialize");
    assert_eq!(parsed.name, "a.png");
    assert_eq!(parsed.jpeg_base64, "AA==");
}

/// The memory CRUD surface: all three commands ride
/// `generate_handler!` and the `memory:changed` broadcast name is
/// part of this file's contract.
#[test]
fn memory_commands_and_event_are_registered() {
    let source = include_str!("lib.rs");
    assert!(source.contains("memory_list,"));
    assert!(source.contains("memory_update,"));
    assert!(source.contains("memory_delete,"));
    assert!(source.contains("memory_history,"));
    assert!(source.contains(concat!("memory", ":changed")));
}
