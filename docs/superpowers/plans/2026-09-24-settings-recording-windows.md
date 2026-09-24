# Settings: Main Language, Recording Section, and Window Tweaks — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `app.main_language`, a `[recording]` config section driving a new Recording settings tab (auto-screenshots, fps, voice-summary instruction), re-center-to-screen-center, prefs-window liquid glass + focus-floating, and a bar row pinned to the bottom of the open card.

**Architecture:** New serde-defaulted config keys validated in `config.rs` and written through the existing `config_set` surface; the language code is read at session start by the ask/summary prompt builders and by `make_stt_provider` (per-provider hint mapping). `enter_main` consults `recording.auto_screenshots`; `MacosCapture::new(fps)` maps fps → `minimum_frame_interval`, and an fps write restarts a live capture. The prefs window goes transparent + `TitleBarStyle::Transparent` + `hidden_title` with `GlassMaterialVariant::Sidebar`, and mirrors focus into `set_always_on_top`. The bar card always lays out `flex-col-reverse` (row at bottom); `growDir` is deleted.

**Tech Stack:** Tauri 2 (Rust), React + TS, `tauri-plugin-liquid-glass`, bun (tests/typecheck), cargo test.

## Global Constraints

- `app.main_language` ∈ `en | zh | ja | ko | fr | es`; default `"en"`; Chinese is stored as `zh` (UI shows 中文).
- `recording.fps` ∈ `{2, 4, 8}` frames per second; default `4`.
- `recording.auto_screenshots` only gates the `Main`-entry auto start; manual `capture_start` always works.
- `recording.summary_prompt` stores text (template text or custom); `""` = Meeting default.
- Five templates: Meeting (default), Book/Article, Lecture, Interview, Brainstorm.
- All Rust edits follow existing doc-comment style; all React components are arrow functions; icons via `@marvis/ui` `Icon`-suffixed exports only; bun for all package/scripts.
- Commands: `bun test` (frontend), `bun run check-types` (apps/native), `cargo test` (src-tauri).

---

### Task 1: Config schema + `config_set` surface + TS types

**Files:**

- Modify: `apps/native/src-tauri/src/config.rs`
- Modify: `apps/native/src-tauri/src/lib.rs` (config_set, ~L1671)
- Modify: `apps/native/src/lib/commands.ts` (~L88-123)

**Interfaces:**

- Produces: `Config.recording: RecordingPrefs { auto_screenshots: bool, fps: u32, summary_prompt: String }`, `AppPrefs.main_language: String`; `config::validate_main_language(&str) -> Result<String,String>`; `config::apply_recording_config(&mut RecordingPrefs, &str, &serde_json::Value) -> Result<bool,String>`; config_set keys `app.main_language`, `recording.auto_screenshots`, `recording.fps`, `recording.summary_prompt`. Later tasks consume `cfg.recording.fps` in `start_capture` and `fps_changed` restart in `config_set`.

- [ ] **Step 1: Failing tests** — append to `config.rs` `mod tests`:

```rust
    #[test]
    fn recording_keys_apply_and_validate() {
        let mut rec = RecordingPrefs::default();
        assert!(apply_recording_config(
            &mut rec, "recording.auto_screenshots", &serde_json::json!(false)).unwrap());
        assert!(!rec.auto_screenshots);
        assert!(apply_recording_config(&mut rec, "recording.fps", &serde_json::json!(8)).unwrap());
        assert_eq!(rec.fps, 8);
        assert_eq!(
            apply_recording_config(&mut rec, "recording.fps", &serde_json::json!(5)).unwrap_err(),
            "unknown fps 5"
        );
        assert!(apply_recording_config(
            &mut rec, "recording.summary_prompt", &serde_json::json!("  custom  ")).unwrap());
        assert_eq!(rec.summary_prompt, "custom");
        assert!(!apply_recording_config(&mut rec, "recording.other", &serde_json::json!(1)).unwrap());
        assert_eq!(validate_main_language(" zh ").unwrap(), "zh");
        assert_eq!(
            validate_main_language("cn").unwrap_err(),
            "unknown language \"cn\""
        );
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
```

- [ ] **Step 2: Run** — `cd apps/native/src-tauri && cargo test recording_keys_apply_and_validate normalize_clamps` → FAIL (missing items).

- [ ] **Step 3: Implement** — `config.rs`: add `main_language: String` to `AppPrefs` (doc: *"`en | zh | ja | ko | fr | es` — the user's main language: chatbox/summary output language and the STT hint. Validated by `config_set`."*) with `Default` impl `"en".into()`; new struct after `VisionPrefs`:

```rust
/// `[recording]` — ambient screen capture + voice-summary prefs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingPrefs {
    /// `enter_main` auto-starts the ambient screen recorder while the
    /// gate is `Main`; `false` leaves capture manual-only — the bar's
    /// record toggle still starts a session.
    pub auto_screenshots: bool,
    /// Screen frame-rate cap: 8 | 4 | 2 fps.
    pub fps: u32,
    /// The summary focus instruction appended to the summary system
    /// prompt (template text or custom); `""` reads as Meeting.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub summary_prompt: String,
}

impl Default for RecordingPrefs {
    fn default() -> Self {
        Self { auto_screenshots: true, fps: 4, summary_prompt: String::new() }
    }
}
```

Add `pub recording: RecordingPrefs` to `Config` (struct + `Default` impl), append to `normalize()`:

```rust
        if !matches!(
            self.app.main_language.as_str(),
            "en" | "zh" | "ja" | "ko" | "fr" | "es"
        ) {
            self.app.main_language = "en".into();
        }
        if !matches!(self.recording.fps, 2 | 4 | 8) {
            self.recording.fps = 4;
        }
        self.recording.summary_prompt = self.recording.summary_prompt.trim().to_string();
```

And the two helpers beside `apply_stt_config`:

```rust
/// The six `app.main_language` codes (Settings → General).
pub(crate) fn validate_main_language(value: &str) -> Result<String, String> {
    let value = value.trim();
    if matches!(value, "en" | "zh" | "ja" | "ko" | "fr" | "es") {
        Ok(value.to_string())
    } else {
        Err(format!("unknown language {value:?}"))
    }
}

/// Apply the `recording.*` config command keys — same handled-shape as
/// [`apply_stt_config`].
pub(crate) fn apply_recording_config(
    recording: &mut RecordingPrefs,
    key: &str,
    value: &serde_json::Value,
) -> Result<bool, String> {
    match key {
        "recording.auto_screenshots" => {
            recording.auto_screenshots = value
                .as_bool()
                .ok_or("recording.auto_screenshots must be a bool")?;
            Ok(true)
        }
        "recording.fps" => {
            let fps = value.as_u64().ok_or("recording.fps must be a number")?;
            if !matches!(fps, 2 | 4 | 8) {
                return Err(format!("unknown fps {fps}"));
            }
            recording.fps = fps as u32;
            Ok(true)
        }
        "recording.summary_prompt" => {
            recording.summary_prompt = value
                .as_str()
                .ok_or("recording.summary_prompt must be a string")?
                .trim()
                .to_string();
            Ok(true)
        }
        _ => Ok(false),
    }
}
```

- [ ] **Step 4: Wire `config_set`** — in `lib.rs` `config_set`, add `let mut fps_changed = false;` beside the other flags, `let prev_fps = cfg.recording.fps;` right after `let mut cfg = state.config.lock();`, and two match arms after `"app.accent"`:

```rust
            "app.main_language" => {
                cfg.app.main_language = config::validate_main_language(
                    value.as_str().ok_or("app.main_language must be a string")?,
                )?;
            }
            key if config::apply_recording_config(&mut cfg.recording, key, &value)? => {}
```

After `config::save(&cfg)` (still inside the block): `fps_changed = cfg.recording.fps != prev_fps;`. After the `accent_changed` block (outside the lock):

```rust
    if fps_changed
        && state
            .capture
            .lock()
            .as_ref()
            .is_some_and(MacosCapture::is_running)
    {
        // A live session keeps its old cadence — rebuild it so the new
        // rate applies immediately.
        stop_capture(&app);
        start_capture(&app);
    }
```

Update the `config_set` doc comment's writable-key list with the four new keys.

- [ ] **Step 5: TS types** — `commands.ts`: add to `AppPrefs` (`/** 'en' | 'zh' | 'ja' | 'ko' | 'fr' | 'es' — output + STT language. */ main_language: string;`), new `RecordingPrefs` interface (`auto_screenshots: boolean; fps: number; summary_prompt: string;` with doc comments mirroring Rust), and `recording: RecordingPrefs;` on `Config`.

- [ ] **Step 6: Run + commit** — `cargo test config` passes; `bun run check-types` in apps/native passes; commit `feat: add main_language + recording config section`.

### Task 2: Main language → prompts (chat + summary)

**Files:**

- Modify: `apps/native/src-tauri/src/prompts.rs`
- Modify: `apps/native/src-tauri/src/ask.rs` (kick ~L223, send_chain ~L380, build_messages ~L598, tests)
- Modify: `apps/native/src-tauri/src/listen.rs` (import L15, build_summary_messages ~L755, generate_summary ~L789, test ~L831)

**Interfaces:**

- Produces: `prompts::language_name(&str)->&'static str`, `live_system_prompt_for(&str)->String`, `summary_system_prompt_for(&str,&str)->String`, `DEFAULT_SUMMARY_INSTRUCTION`; `send_chain(..., fresh_session, language: &str)`; `build_summary_messages(history, previous, language, focus)`.

- [ ] **Step 1: Failing tests** — `prompts.rs` tests:

```rust
    #[test]
    fn language_directives_join_the_system_prompts() {
        assert_eq!(language_name("zh"), "Chinese");
        assert_eq!(language_name("bogus"), "English");
        let live = live_system_prompt_for("ja");
        assert!(live.contains("preferred reply language is Japanese"));
        let summary = summary_system_prompt_for("fr", "Focus on risks.");
        assert!(summary.contains("Write tldr, bullets, follow_ups, and topic in French."));
        assert!(summary.contains("## Focus\n\nFocus on risks."));
        let default = summary_system_prompt_for("en", "   ");
        assert!(default.contains(DEFAULT_SUMMARY_INSTRUCTION));
    }
```

- [ ] **Step 2: Implement prompts.rs** — after `summary_context`:

```rust
/// The six `app.main_language` codes as the English name a prompt
/// instruction uses — the model reads this, so it must be English
/// text, not the autonym. Unknown/empty reads as English.
pub fn language_name(code: &str) -> &'static str {
    match code.trim() {
        "zh" => "Chinese",
        "ja" => "Japanese",
        "ko" => "Korean",
        "fr" => "French",
        "es" => "Spanish",
        _ => "English",
    }
}

/// The live prompt plus the main-language directive: the preference is
/// the standing default, an explicit per-request language still wins.
pub fn live_system_prompt_for(language: &str) -> String {
    format!(
        "{}\n\nThe user's preferred reply language is {}; respond in it unless the current request explicitly asks for a different language.",
        live_system_prompt(),
        language_name(language),
    )
}

/// The default summary focus — the Meeting template; an empty
/// `recording.summary_prompt` reads as this.
pub const DEFAULT_SUMMARY_INSTRUCTION: &str =
    "Focus on decisions made, action items with owners and deadlines, and open questions.";

/// The summary prompt plus the language directive and the user's focus
/// instruction — appended AFTER the JSON contract so the output shape
/// stays mandatory whatever the focus says.
pub fn summary_system_prompt_for(language: &str, focus: &str) -> String {
    let focus = if focus.trim().is_empty() {
        DEFAULT_SUMMARY_INSTRUCTION
    } else {
        focus.trim()
    };
    format!(
        "{}\n\nWrite tldr, bullets, follow_ups, and topic in {}.\n\n## Focus\n\n{}",
        summary_system_prompt(),
        language_name(language),
        focus,
    )
}
```

- [ ] **Step 3: Plumb ask.rs** — `kick`: the `cfg` lock block becomes
  `(candidates, vision, language)` with `cfg.app.main_language.clone()` as
  the third tuple member; the spawn passes `&language` as a new LAST arg to
  `send_chain`. `send_chain` signature gains `language: &str`; its
  `build_messages(...)` call passes it through; `build_messages` gains
  `language: &str` and uses `ChatMessage::text(Role::System, live_system_prompt_for(language))`
  (import `live_system_prompt_for`; `live_system_prompt` may remain for tests).
  Update every test call site: append `, "en"` to `send_chain(...)` calls and
  pass `"en"` to `build_messages` tests (assert the system part contains
  "preferred reply language is English").

- [ ] **Step 4: Plumb listen.rs** — import `summary_system_prompt_for` (drop `summary_system_prompt` if unused); `build_summary_messages` gains `language: &str, focus: &str` and uses `summary_system_prompt_for(language, focus)`; `generate_summary` calls it with `&config.app.main_language, &config.recording.summary_prompt`. Update the existing test: `build_summary_messages("...", Some("..."), "en", "")` and assert the system part contains `English` and `DEFAULT_SUMMARY_INSTRUCTION`; add a second call with `"zh", "Focus on book themes."` asserting both land.

- [ ] **Step 5: Run + commit** — `cargo test prompts ask:: listen::` (module filters) green; commit `feat: main language drives chat and summary output`.

### Task 3: STT language hints (deepgram / whisper / sherpa)

**Files:**

- Modify: `apps/native/src-tauri/src/stt/mod.rs` (`make_stt_provider` ~L38)
- Modify: `apps/native/src-tauri/src/stt/deepgram.rs`
- Modify: `apps/native/src-tauri/src/stt/whisper.rs`
- Modify: `apps/native/src-tauri/src/stt/sherpa.rs`
- Modify: `apps/native/src-tauri/src/listen.rs` (~L407), `apps/native/src-tauri/src/dictation.rs` (~L218)

**Interfaces:**

- Produces: `make_stt_provider(provider, key, model, channel, bundled_whisper, language: &str)`; per-provider mappers `deepgram_language`, `whisper_language`, `sense_voice_language` (all `&str -> &'static str`/Option).

- [ ] **Step 1: Failing tests**

```rust
// deepgram.rs tests
assert_eq!(deepgram_language("zh"), Some("zh-CN"));
assert_eq!(deepgram_language("en"), Some("en"));
assert_eq!(deepgram_language("bogus"), None);
// whisper.rs tests — extend cli_args_detect_language_and_never_translate:
let args = whisper_cli_args(Path::new("/m.bin"), Path::new("/w.wav"), "zh");
// → language arg is "zh"; whisper_cli_args(.., "bogus") → "auto"
// sherpa.rs tests
assert_eq!(sense_voice_language("fr"), "auto");
assert_eq!(sense_voice_language("ko"), "ko");
```

- [ ] **Step 2: Implement** — `make_stt_provider` gains `language: &str` and forwards it.
  - **Deepgram:** `DeepgramProvider` gains `language: String`; `new(key, model, channel, language: &str)` and `with_endpoint` take it; `start` clones it into `run_worker(..., language: String, ...)` → `run_session(..., language: &str, ...)`; the URL appends `deepgram_language(language).map(|c| format!("&language={c}")).unwrap_or_default()` after `smart_format=true`. Update `run_session`/`run_worker` test call sites with `"en"`/`"en".to_string()`.
  - **Whisper:** `WhisperProvider::new(model, channel, bundled, language: &str)` stores `language: String`; `run_chunks` gains `language: String` (add `#[allow(clippy::too_many_arguments)]` if needed — it already has 8); `transcribe_window` gains `language: &str`; `whisper_cli_args(model, wav, language)` uses `"-l", whisper_language(language)` where the mapper returns the six codes verbatim else `"auto"`. Update the two `run_chunks`/args test call sites.
  - **Sherpa:** `SherpaProvider::new(model, channel, language: &str)` stores it; the engine cache key becomes `(PathBuf, String)`: `ENGINE: Mutex<Option<(PathBuf, String, Arc<SherpaEngine>)>>`; `engine_for(model_dir, language)` / `engine_for_with(model_dir, language, spawn)` hit only when dir AND language match and the engine is alive; `SherpaEngine::spawn(model_dir, language)` → `create_recognizer(dir, language)` sets `language: Some(sense_voice_language(language).into())`. `start` calls `engine_for(&self.model_dir, &self.language)`. Update test sites (`SherpaProvider::new("sense-voice", SpeakerChannel::Me, "en")`, `engine_for_with(dir, "en", ...)`, the ENGINE seed tuple).
  - **Call sites:** listen.rs `make_stt_provider(&provider_name, key.clone(), model.clone(), channel, bundled_whisper, &config.app.main_language)`; dictation.rs same with `&config.app.main_language`.

- [ ] **Step 3: Run + commit** — `cargo test stt` green; commit `feat: main language hints STT providers`.

### Task 4: Recording config → capture runtime

**Files:**

- Modify: `apps/native/src-tauri/src/capture/macos.rs` (`MacosCapture::new` ~L141, const ~L39, test ~L351)
- Modify: `apps/native/src-tauri/src/lib.rs` (`enter_main` ~L275, `start_capture` ~L335)

**Interfaces:**

- Produces: `capture::macos::frame_interval_secs(fps: u32) -> f64` (crate-visible), `MacosCapture::new(fps: u32)`; `enter_main` no-ops (but still emits `capture:state`) when `auto_screenshots` is false; `config_set` `recording.fps` restarts a live capture (Task 1 Step 4).

- [ ] **Step 1: Failing test** — `macos.rs` tests:

```rust
    #[test]
    fn frame_interval_maps_fps_to_seconds() {
        assert_eq!(frame_interval_secs(8), 0.125);
        assert_eq!(frame_interval_secs(4), 0.25);
        assert_eq!(frame_interval_secs(2), 0.5);
        assert_eq!(frame_interval_secs(0), 1.0); // defensive floor
    }
```

- [ ] **Step 2: Implement** — delete `FRAME_INTERVAL_SECS`; add `pub(crate) fn frame_interval_secs(fps: u32) -> f64 { 1.0 / f64::from(fps.max(1)) }`; `MacosCapture::new(fps: u32)` uses `CMTime::from_seconds(frame_interval_secs(fps), 600)`; the ignored test calls `MacosCapture::new(4)`. `lib.rs` `enter_main`:

```rust
fn enter_main(app: &AppHandle) {
    // Settings → Recording: ambient capture is opt-out; the bar's record
    // toggle stays a manual override either way.
    let state = app.state::<AppState>();
    if state.config.lock().recording.auto_screenshots {
        start_capture(app);
    } else {
        // Still broadcast — listeners resync on every Main entry.
        let status = capture_snapshot(&state);
        emit_capture_state(app, &status);
    }
}
```

`start_capture`: `let fps = state.config.lock().recording.fps;` before the capture lock; `MacosCapture::new(fps)`. Fix the one `enter_main` doc line if it mentions always starting capture.

- [ ] **Step 3: Run + commit** — `cargo test capture` green; commit `feat: recording prefs drive ambient capture cadence and auto-start`.

### Task 5: Bar re-center → work-area center

**Files:**

- Modify: `apps/native/src-tauri/src/windows/mod.rs` (`BAR_TOP_OFFSET` ~L98, `recenter_bar` ~L490, `position_bar_at_startup` ~L560, `window_recenter` doc ~L1484)
- Modify: `apps/native/src/components/prefs/BarTab.tsx` (~L37-42, ~L69)

- [ ] **Step 1: Implement Rust** — delete `BAR_TOP_OFFSET`; in both places the default rect becomes `x: work.center_x() - BAR_IDLE_W / 2.0, y: work.center_y() - BAR_H / 2.0`; update doc comments (`recenter_bar`: "centered on the primary work area"; `position_bar_at_startup`: "centered on the primary work area"; `window_recenter` doc in lib.rs: "the middle of the screen").

- [ ] **Step 2: Implement UI** — `recenter` re-reads the edge (center is equidistant — no assumed edge):

```tsx
  const recenter = () => {
    void windowRecenter()
      .then(() => windowBarEdge())
      .then((e) => setEdge(asEdge(e)))
      .catch(() => {});
  };
```

PrefRow sub → `'Back to the default spot — the middle of the screen.'`

- [ ] **Step 3: Run + commit** — `cargo test windows` + `bun run check-types` green; commit `feat: re-center bar at work-area center`.

### Task 6: Card layout — bar row pinned to the bottom

**Files:**

- Modify: `apps/native/src/hooks/useCardGeometry.ts`
- Modify: `apps/native/src/views/Bar.tsx` (~L8-13 header, ~L96, ~L303-316, ~L489-520)

- [ ] **Step 1: Simplify the hook** — delete `growDir` state, `collapsedY`, the `onMoved` listener and `outerPosition` read, and the `getCurrentWindow` import; the resize `read` becomes `setCardOpen(window.innerHeight > BAR_H + OPEN_EPS)`; return `{ cardOpen }`. Update the header comment (the `growDir` paragraph goes away).

- [ ] **Step 2: Bar.tsx** — `const { cardOpen } = useCardGeometry(cardRef, stageRef);`; rowCls: `growDir === 'up' ? 'border-t' : 'border-b'` → `'border-t'` (row sits under the section now — comment: *"the row is bottom-pinned under `flex-col-reverse`, so the divider is always its top edge"*); stage: `growDir === 'up' ? 'justify-end' : 'justify-start'` → `'justify-end'` and delete `data-pos`/`data-dir`; card class: `growDir === 'up' ? 'flex-col-reverse' : 'flex-col'` → `'flex-col-reverse'`; update the header comment: the bar row is the card's bottom-anchored footer regardless of grow direction.

- [ ] **Step 3: Run + commit** — `bun test` + `bun run check-types` green; commit `feat: pin the bar row to the card's bottom edge`.

### Task 7: Prefs window — liquid glass + focus-floating level

**Files:**

- Modify: `apps/native/src-tauri/src/windows/mod.rs` (`PREFS_LABEL` doc ~L83, `build_prefs_window` ~L857, `hide_prefs` ~L357)
- Modify: `apps/native/src/views/Prefs.tsx` (root div ~L100)
- Modify: `apps/native/src/index.css` (after the `.glass-surface` rules ~L116)
- Modify: `apps/native/src/components/prefs/SettingsMode.tsx` (nav padding ~L40-41, content ~L68)

- [ ] **Step 1: Rust** — rewrite `build_prefs_window`: add `.title_bar_style(tauri::TitleBarStyle::Transparent)`, `.hidden_title(true)`, `.transparent(true)`; extend `on_window_event` with `tauri::WindowEvent::Focused(focused) => { let _ = handle.set_always_on_top(*focused); }` (match arms — the `CloseRequested` arm also gains `let _ = handle.set_always_on_top(false);` before `hide()`); after `set_content_protected`, spawn the detached `set_effect` thread with `LiquidGlassConfig { corner_radius: 0.0, tint_color: None, variant: tauri_plugin_liquid_glass::GlassMaterialVariant::Sidebar, ..Default::default() }`. `hide_prefs` adds `let _ = win.set_always_on_top(false);`. Update `PREFS_LABEL`/`build_prefs_window` doc comments (no longer "opaque, NOT always-on-top" — now glass, floats only while focused). Import `GlassMaterialVariant` at the top (`use tauri_plugin_liquid_glass::{GlassMaterialVariant, LiquidGlassConfig, LiquidGlassExt};`).

- [ ] **Step 2: CSS/TSX** — `Prefs.tsx` root: add `prefs-shell` to the className (before `relative`); `index.css` after the `.glass-surface` block:

```css
/* The prefs window's own material: the glass view fills the whole
   window (transparent titlebar), so the webview root goes transparent
   under a native material just like .glass-stage. */
[data-material='glass'] .prefs-shell,
[data-material='vibrancy'] .prefs-shell {
  background: transparent;
}
```

`SettingsMode.tsx`: nav `px-2 py-2.5` → `px-2 pt-9 pb-2.5` (clears the floating traffic lights); content `px-6 pt-5 pb-5.5` → `px-6 pt-8 pb-5.5`.

- [ ] **Step 3: Run + commit** — `cargo build` (windows code must compile — `tauri::TitleBarStyle` exists in tauri 2.11) + `bun run check-types` green; commit `feat: liquid-glass prefs window that floats only while focused`.

### Task 8: Settings UI — General language row + Recording tab

**Files:**

- Modify: `packages/ui/src/index.ts` (add `VideoIcon` to the lucide export list)
- Modify: `apps/native/src/components/prefs/GeneralTab.tsx`
- Create: `apps/native/src/components/prefs/RecordingTab.tsx`
- Modify: `apps/native/src/components/prefs/SettingsMode.tsx`

**Interfaces:**

- Consumes: `Config.app.main_language`, `Config.recording.*`, `configSet` keys from Task 1; `PrefRow`/`Seg`/`Switch` from `./bits`; `MODEL_SEL`, `PRF_ROW`, `PR_LABEL`, `PR_SUB`, `PRF_ROWS`, `H2`, `SUB` from `@/lib/classes`.

- [ ] **Step 1: Icon** — add `VideoIcon` to the lucide-react export list in `packages/ui/src/index.ts` (alphabetical position, matching the existing list).

- [ ] **Step 2: GeneralTab** — add after the Appearance row:

```tsx
const LANGUAGES = [
  { id: 'en', label: 'English' },
  { id: 'zh', label: '中文' },
  { id: 'ja', label: '日本語' },
  { id: 'ko', label: '한국어' },
  { id: 'fr', label: 'Français' },
  { id: 'es', label: 'Español' },
] as const;
```

```tsx
        <PrefRow
          label='Main language'
          sub='The default for chat replies, meeting summaries, and dictation — unless you ask for another language in the moment.'>
          <select
            aria-label='Main language'
            className={cn(MODEL_SEL, 'w-40 flex-none')}
            value={cfg?.app.main_language ?? 'en'}
            onChange={(e) =>
              void configSet('app.main_language', e.target.value)
                .then(data.setConfig)
                .catch(() => {})
            }>
            {LANGUAGES.map((l) => (
              <option key={l.id} value={l.id}>
                {l.label}
              </option>
            ))}
          </select>
        </PrefRow>
```

(`cn`, `MODEL_SEL` imports added; Appearance row drops `last` only if it had it — it doesn't; Accent keeps `last`.)

- [ ] **Step 3: RecordingTab** — new file:

```tsx
/**
 * Recording — screen-capture cadence and the voice-summary instruction.
 * `recording.*` writes broadcast `config:changed`; an fps write
 * restarts a live capture server-side so the new rate applies now.
 */
import { useEffect, useRef, useState } from 'react';
import { configSet } from '@/lib/commands';
import { H2, MODEL_SEL, PRF_ROW, PRF_ROWS, PR_LABEL, PR_SUB, SUB, cn } from '@/lib/classes';
import { PrefRow, Seg, Switch } from './bits';
import type { PrefsData } from './types';

const SUMMARY_TEMPLATES = [
  { id: 'meeting', label: 'Meeting', text: 'Focus on decisions made, action items with owners and deadlines, and open questions.' },
  { id: 'book', label: 'Book / article', text: 'Focus on the key ideas, themes, and takeaways; note memorable claims or quotes.' },
  { id: 'lecture', label: 'Lecture', text: 'Focus on concepts taught, definitions, worked examples, and anything emphasized as important.' },
  { id: 'interview', label: 'Interview', text: "Focus on the candidate's answers, demonstrated strengths, concerns raised, and notable questions." },
  { id: 'brainstorm', label: 'Brainstorm', text: 'Focus on ideas proposed, pros and cons discussed, and the directions the group is converging toward.' },
] as const;

const MEETING_TEXT = SUMMARY_TEMPLATES[0].text;

export const RecordingTab = ({ data }: { data: PrefsData }) => {
  const cfg = data.config;
  const auto = cfg?.recording.auto_screenshots ?? true;
  const fps = cfg?.recording.fps === 8 || cfg?.recording.fps === 2 ? cfg.recording.fps : 4;
  const stored = cfg?.recording.summary_prompt ?? '';
  // '' reads as the Meeting template — the textarea shows the
  // instruction the summary will actually use.
  const shown = stored.trim() === '' ? MEETING_TEXT : stored;
  const [draft, setDraft] = useState(shown);
  useEffect(() => setDraft(shown), [shown]);
  const timer = useRef<number | undefined>(undefined);
  const pending = useRef<string | null>(null);
  // Flush a pending debounced write on unmount (a mode switch remounts
  // this subtree — the prefs window itself never unmounts).
  useEffect(
    () => () => {
      window.clearTimeout(timer.current);
      if (pending.current !== null) {
        void configSet('recording.summary_prompt', pending.current).catch(() => {});
      }
    },
    [],
  );

  const writePrompt = (text: string) => {
    pending.current = text;
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      pending.current = null;
      void configSet('recording.summary_prompt', text).then(data.setConfig).catch(() => {});
    }, 400);
  };

  // The template whose text the draft matches, else 'custom'.
  const selected = SUMMARY_TEMPLATES.find((t) => t.text === draft)?.id ?? 'custom';

  const pickTemplate = (id: string) => {
    const t = SUMMARY_TEMPLATES.find((t) => t.id === id);
    if (!t) return; // 'custom' — the draft already is the custom text
    pending.current = null;
    window.clearTimeout(timer.current);
    setDraft(t.text);
    void configSet('recording.summary_prompt', t.text).then(data.setConfig).catch(() => {});
  };

  return (
    <>
      <h2 className={H2}>Recording</h2>
      <p className={SUB}>Screen capture runs quietly in the background; voice settings shape Listen summaries.</p>

      <div className={PRF_ROWS}>
        <PrefRow
          label='Take screenshots automatically'
          sub='Capture the screen when the app is running — the bar’s record button still works when this is off.'>
          <Switch
            ariaLabel='Take screenshots automatically'
            checked={auto}
            onChange={(on) =>
              void configSet('recording.auto_screenshots', on).then(data.setConfig).catch(() => {})
            }
          />
        </PrefRow>
        <PrefRow
          label='Frame rate'
          sub='How often the screen is sampled — higher rates track motion better.'>
          <Seg
            ariaLabel='Frame rate'
            value={String(fps) as '8' | '4' | '2'}
            onChange={(v) =>
              void configSet('recording.fps', Number(v)).then(data.setConfig).catch(() => {})
            }
            options={[
              { id: '8' as const, label: '8 fps' },
              { id: '4' as const, label: '4 fps' },
              { id: '2' as const, label: '2 fps' },
            ]}
          />
        </PrefRow>
        <div className={cn(PRF_ROW, 'flex-col items-stretch gap-2.5 border-b-0')}>
          <div className='flex items-center justify-between gap-4'>
            <div>
              <div className={PR_LABEL}>Summary instruction</div>
              <div className={PR_SUB}>Guides Listen summaries — pick a template, then edit the text freely.</div>
            </div>
            <select
              aria-label='Summary template'
              className={cn(MODEL_SEL, 'w-40 flex-none')}
              value={selected}
              onChange={(e) => pickTemplate(e.target.value)}>
              {SUMMARY_TEMPLATES.map((t) => (
                <option key={t.id} value={t.id}>{t.label}</option>
              ))}
              <option value='custom'>Custom</option>
            </select>
          </div>
          <textarea
            aria-label='Summary instruction'
            className='min-h-20 w-full resize-y rounded-lg border border-border bg-input-well px-2.5 py-2 text-[12.5px] leading-relaxed text-foreground outline-none transition-[border-color,box-shadow] duration-(--motion-fast) ease-(--ease) focus:border-accent focus:shadow-(--focus-ring)'
            value={draft}
            onChange={(e) => {
              setDraft(e.target.value);
              writePrompt(e.target.value);
            }}
          />
        </div>
      </div>
    </>
  );
};
```

- [ ] **Step 4: SettingsMode** — import `VideoIcon` (replace an existing icon import line) and `RecordingTab`; insert `{ id: 'recording', label: 'Recording', icon: VideoIcon },` after the `bar` entry; render `{tab === 'recording' && <RecordingTab data={data} />}` after the bar line.

- [ ] **Step 5: Run + commit** — `bun run check-types` + `bun test` green; commit `feat: recording settings tab and main-language picker`.

### Task 9: Stale docs + full verification

**Files:**

- Modify: `DESIGN.md` (L253 bar description; the prefs-window row if it claims opaque/non-floating — check the windows table)

- [ ] **Step 1** — update `DESIGN.md` L253's "floats 21px below work-area top" to the work-area center; fix the prefs-window description if it claims opaque/non-glass.

- [ ] **Step 2: Verify** — `cd apps/native/src-tauri && cargo test` (whole suite); `cd apps/native && bun test && bun run check-types`; then `bun run build` (vite+tsc) to catch bundling issues.

- [ ] **Step 3: Commit** — `docs: re-center default + glass prefs in DESIGN.md`.

---

## Self-Review Notes (verified while writing)

- Spec coverage: all spec sections map to Tasks 1–8; language directive wording lives in prompts.rs (single place, tested).
- `send_chain`/`build_messages`/`build_summary_messages` signatures change — all call sites and tests updated inside their tasks (mechanical `"en"` appends).
- `make_stt_provider` signature change — only 2 production call sites (listen.rs, dictation.rs) both have `config: &Config` in scope.
- `engine_for_with`/`ENGINE` key change — sherpa tests updated in Task 3.
- Lock order for the fps restart: `fps_changed` computed inside the config lock; `stop_capture`/`start_capture` run after the block — no `config → capture` hold.
- `enter_main` flag read is a short-lived lock, dropped before `start_capture`.
- `deepgram_language("zh")` → `"zh-CN"`; whisper/sherpa map `zh` → `"zh"`.
- Prefs `set_effect` uses the same detached-thread pattern as `build_window` (pool lock is held by `show_prefs`'s caller path).
- `TitleBarStyle::Transparent` + `hidden_title` confirmed available in tauri 2.11.5.
