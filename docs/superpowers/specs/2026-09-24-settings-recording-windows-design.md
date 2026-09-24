# Settings: Main Language, Recording Section, and Window Tweaks

## Goal

Add a Main Language preference, a new Recording settings section, and three window tweaks: re-center defaults to the work-area center, the prefs window can overlap the bar while focused, the prefs window gets the liquid-glass Sidebar variant, and the open card pins the bar row to its bottom edge.

## Scope

Covers `config.toml` schema, the `config_set` writable surface, Settings UI (General, Bar, new Recording tab), the ask/summary/STT language plumbing, the ambient capture cadence and auto-start gate, the prefs window's chrome/material/level behavior, and the bar card's row ordering. Existing provider/session/event semantics remain intact unless described below.

## Config schema

New keys, all `#[serde(default)]` so older files load cleanly:

```toml
[app]
main_language = "en"          # en | zh | ja | ko | fr | es

[recording]
auto_screenshots = true       # ambient capture auto-starts in Main
fps = 4                       # 8 | 4 | 2 frames per second
summary_prompt = ""           # "" → built-in meeting instruction
```

New writable `config_set` keys: `app.main_language` (enum-validated), `recording.auto_screenshots` (bool), `recording.fps` (number ∈ {2,4,8}), `recording.summary_prompt` (string, trimmed). Chinese is stored as `zh` (the BCP-47/STT code); the UI shows native names.

## Main Language

Consumers of `app.main_language`:

- **Chat (ask)**: plumb the code through `kick` → `send_chain` → `build_messages`; the live system prompt gains a trailing directive — *"The user's preferred reply language is {Name}; respond in it unless the current request explicitly asks for a different language."* An explicit per-request language still wins over the preference.
- **Summary (listen)**: `build_summary_messages` appends *"Write tldr, bullets, follow_ups, and topic in {Name}."* to the summary system prompt.
- **STT (dictation + listen)**: `make_stt_provider` gains a `language` argument.
  - Deepgram appends `&language=` to the listen URL (`zh` → `zh-CN`; en/ja/ko/fr/es pass through).
  - whisper-cli uses `-l {code}` (whisper accepts all six codes directly).
  - sherpa SenseVoice maps en/zh/ja/ko; fr/es fall back to `auto`.

## Bar re-center

`recenter_bar` and `position_bar_at_startup`'s no-saved-position default both move to the work-area center (`center_x − w/2`, `center_y − h/2`). `BAR_TOP_OFFSET` is removed. The Bar tab's Re-center copy is updated, and the edge picker re-reads `window_bar_edge` after re-centering (center is equidistant; the backend's nearest-edge read settles it).

## Recording tab

New `RecordingTab` in the Settings sidebar (between Bar and Providers).

- **Screen — "Take screenshots automatically"** (Switch → `recording.auto_screenshots`, default on). ON = today's behavior: `enter_main` starts ambient capture. OFF = `enter_main` skips `start_capture`; the bar's record toggle still starts a session manually. The write does not kill a live session — it is a startup preference; the bar toggle owns live capture state.
- **Screen — "Frame rate"** (Seg → `recording.fps`, 8/4/2 fps, default 4). `MacosCapture::new(fps)` sets `minimum_frame_interval = 1/fps` (the current 0.25 s ≙ 4 fps). A `recording.fps` write while capture is running restarts capture so the new rate applies immediately.
- **Voice — "Summary instruction"**: five template presets + editable text. Selecting a template writes its text to `recording.summary_prompt`; editing diverges it into "Custom" (the selection is derived by comparing the stored text to the presets). Shipped presets:
  - Meeting (default): decisions, action items with owners and deadlines, open questions.
  - Book/Article: key ideas, themes, takeaways, memorable claims.
  - Lecture: concepts taught, definitions, examples, emphasized points.
  - Interview: the candidate's answers, strengths, concerns, notable questions.
  - Brainstorm: ideas proposed, pros/cons discussed, converging directions.

The instruction is appended to the summary system prompt as a *focus* directive after the JSON contract, which stays mandatory — `parse_summary` and the listen card are unchanged. An empty `summary_prompt` uses the Meeting text.

## Prefs window: level + material

- **Overlap the bar while active**: the prefs window stays a normal decorated window but its always-on-top mirrors focus — `WindowEvent::Focused(f)` → `set_always_on_top(f)`, and hide/close resets it to `false`. Focused prefs joins the bar's floating level and orders above it; blurred it returns to the normal level. The bar's own `always_on_top` is untouched — it still floats over every other app.
- **Liquid glass (Sidebar variant)**: `build_prefs_window` switches to `transparent(true)` + `title_bar_style(Transparent)` + `hidden_title(true)` (traffic lights overlay the content — the macOS Settings.app look) and applies `set_effect` with `variant: GlassMaterialVariant::Sidebar`, `corner_radius: 0`, no tint — same detached-thread pattern as `build_window`. Pre-26 macOS falls back to the plugin's NSVisualEffectView (variant is a no-op there).
- CSS: the prefs root goes transparent under `data-material = glass|vibrancy`; the sidebar keeps its translucent fill and gains top padding for the traffic lights.

## Card layout: bar row at the bottom

An open card always lays out `flex-col-reverse` — the bar row pins to the card's **bottom** edge, the chat/listen section above — regardless of physical grow direction (which still picks the roomier side per `expand_dir_for`). For a top-docked bar the row rides the card's bottom edge during the expand animation; a bottom-docked bar is unchanged. `growDir` becomes unused and is removed from `useCardGeometry`; the row divider is `border-t` unconditionally.

## Tests

Match existing test culture:

- `config.rs`: validation of `app.main_language` / `recording.fps` / `recording.auto_screenshots` / `recording.summary_prompt` through `config_set`.
- `prompts.rs` / `ask.rs` / `listen.rs`: the language directive lands in the chat system message and the summary system message; the summary focus instruction lands after the JSON contract.
- `capture`: fps → `minimum_frame_interval` mapping.
- Existing suites keep passing (no signature changes leak outside the listed seams).
