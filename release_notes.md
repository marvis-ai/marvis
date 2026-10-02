# v0.1.0

The first public release of **Marvis** — a privacy-first AI co-pilot that
lives on your desktop. Marvis floats above your workspace as a small
translucent bar: it can see your screen (with your permission), hear your
meetings, and stream answers into an overlay — while your API keys,
history, and screen data stay entirely on your machine. No accounts, no
cloud sync; nothing leaves your device except the requests you send to the
providers you choose.

## Highlights

### The bar

- A frameless, always-on-top overlay that morphs from a breathing capsule
  into an input pill, then into a chat/listen card.
- Docks to any screen edge, drags anywhere, remembers its position, and
  can be locked in place.
- Right-click context menu and tray icon with live state — start/stop
  capture, start a Listen, open history, position, and lock.
- Liquid-glass material on macOS; launch wordmark intro; lazy-loaded views
  keep the webview light.

### Ask — screen-aware questions

- Type a question and press `Cmd`/`Ctrl+Enter` to send it with the latest
  screen frame; answers stream back as markdown.
- Screen content lives in an in-memory ring buffer (~60 s horizon, capped
  at 120 frames / 64 MB) — screenshots are never written to disk.
- Choose exactly what to share: a native content-sharing picker offers
  displays, windows, and thumbnails before capture starts.
- An optional vision provider can read the frame first and answer over its
  text description.
- Dictate straight into the input with the push-to-talk mic.
- Token usage is tracked per message, with a retry command for failed
  sends.

### Listen — meeting transcription

- Dual-channel capture: your mic ("me") + system audio ("them") flow into
  streaming speech-to-text.
- Live diarized transcript with speaker labels, color-coded turns, speaker
  filters, elapsed timer, pause/resume, and one-click transcript copy.
- Rolling AI summaries pin a TLDR strip above the transcript, refreshed
  every five turns — with follow-up chips that ask questions against the
  meeting's own chat context.
- Optional voice enrollment identifies you as "You" across sessions and
  languages.
- Each session can record a single 16 kHz mono WAV mixing both channels.

### Bring your own model

- LLM providers: OpenAI, Anthropic, Gemini, OpenRouter,
  [Ollama](https://ollama.com) (no API key needed), or any
  OpenAI-compatible endpoint.
- Providers form a drag-to-reorder failover chain — a failed call hands
  off to the next configured provider.
- Speech-to-text: Deepgram streaming (interim results), fully-local
  bundled `whisper.cpp` (tiny / base / small models downloaded on demand
  from Hugging Face), or sherpa-onnx SenseVoice with an optional English
  punctuation model and built-in diarization.
- A main-language preference drives chat, summary, and transcription
  output.

### History & settings

- Sessions, messages, transcripts, and summaries persist locally — reopen
  any chat or listen session where you left off.
- Guided first-run onboarding: appearance → screen consent → provider key
  (validated before saving, or pick an Ollama model) → optional local
  voice models.
- A liquid-glass settings window covers appearance, providers, recording,
  hotkeys, and language.
- `marvis://` deep links — `marvis://ask?text=...` focuses the bar and
  fires an Ask; any other link surfaces the app.

## Hotkeys

Five rebindable global chords (Settings → Hotkeys or `[hotkeys]` in
`~/.marvis/config.toml`):

| Action | Default |
| --- | --- |
| Show / hide the input | `Cmd`/`Ctrl+Alt+Space` |
| Toggle screen capture | `Cmd`/`Ctrl+Alt+R` |
| Start a Listen | `Cmd`/`Ctrl+Alt+T` |
| Open history | `Cmd`/`Ctrl+Alt+H` |
| Lock bar position | `Cmd`/`Ctrl+Shift+L` |

Fixed in-bar keys: `Enter` send · `Shift+Enter` new line ·
`Cmd`/`Ctrl+Enter` send with screenshot · `Cmd`/`Ctrl+,` settings.

## Platform support

| Platform | Screen capture | System audio | Consent |
| --- | --- | --- | --- |
| macOS (Apple Silicon / Intel) | ScreenCaptureKit | ScreenCaptureKit | Screen Recording permission |
| Windows (x86_64) | Windows.Graphics.Capture | WASAPI loopback | none — WGC is the consent boundary |
| Linux (x86_64) | XDG portal + PipeWire | PulseAudio monitor | portal picker (persisted token) |

## Downloads

| Platform | Artifacts |
| --- | --- |
| macOS Apple Silicon | `Marvis_0.1.0_aarch64.dmg` |
| macOS Intel | `Marvis_0.1.0_x64.dmg` |
| Windows x64 | `Marvis_0.1.0_x64-setup.exe` (NSIS), `Marvis_0.1.0_x64_en-US.msi` |
| Linux x64 | `Marvis_0.1.0_amd64.AppImage`, `Marvis_0.1.0_amd64.deb` |

## Privacy notes

Everything Marvis writes lives under `~/.marvis`
(`%USERPROFILE%\.marvis` on Windows), created `0700` where the OS has
mode bits:

- `keys.json` (`0600`) — plaintext provider API keys inside the `0700`
  root; there is no encryption in this build. Keys are never serialized
  to the UI (masked `…last4` only) and never logged.
- `config.toml` — non-secret preferences.
- `marvis.db` (`0600`) — SQLite sessions, messages, transcripts,
  summaries.
- `models/` — downloaded whisper / sherpa voice models.

Security issues: please report privately to <support@getmarvis.com>.

## What's next

Session history UI improvements, prompt presets, Ollama model management,
and a Gemini search-grounding toggle are on the
[roadmap](https://github.com/MarvisLLC/marvis#roadmap). Bug reports and
feature requests →
[GitHub issues](https://github.com/MarvisLLC/marvis/issues).
