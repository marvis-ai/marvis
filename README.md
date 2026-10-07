<!-- markdownlint-disable MD041 -->
<p align="center">
  <img src="assets/marvis-logo.svg" alt="Marvis" width="260">
</p>

<p align="center">
  <strong>Private / Personal AI for All</strong> — a privacy-first AI
  co-pilot that lives on your desktop.
</p>

<p align="center">
  <a href="LICENSE"><img
    src="https://img.shields.io/badge/license-Apache_2.0-blue.svg"
    alt="License: Apache-2.0"></a>
  <img
    src="https://img.shields.io/badge/platform-macOS_Windows_Linux-lightgrey"
    alt="Platforms: macOS, Windows, Linux">
  <a href="https://github.com/MarvisLLC/marvis/stargazers"><img
    src="https://img.shields.io/github/stars/MarvisLLC/marvis"
    alt="GitHub stars"></a>
</p>

Marvis is an always-available assistant that floats above your workspace as a
small translucent bar. It can see your screen (with your permission), hear
your meetings, answer questions about what's on screen, and stream responses
into an overlay panel — all while keeping your API keys, history, and screen
data entirely on your machine. No accounts, no cloud sync, nothing leaves
your device except the requests you send to the providers you choose.

**Tauri 2 + Rust + React** — the Rust core owns all sensitive logic
(capture, audio, keys, storage) and the webview is UI only.

## Features

- **Overlay assistant** — a frameless, always-on-top bar that morphs from a
  breathing capsule into an input pill, then into a chat/listen card. Docks
  to any screen edge, drags anywhere, remembers its position.
- **Ask** — type a question or press `Cmd`/`Ctrl+Enter`; Marvis grabs
  the latest screen frame, sends it with your prompt to your LLM, and
  streams the answer back as markdown. Screen content stays in an
  in-memory ring buffer (~60 s horizon, capped at 120 frames / 64 MB) —
  screenshots are never written to disk. An optional **vision provider**
  can read the frame first and answer over its text description.
- **Listen** — dual-channel meeting capture: mic ("me") + system audio
  ("them") → streaming speech-to-text → live transcript with speaker labels
  → rolling AI summaries every 5 turns.
- **Bring your own model** — OpenAI, Anthropic, Gemini, OpenRouter,
  [Ollama](https://ollama.com) (no API key needed), or any
  OpenAI-compatible endpoint. Providers form a drag-to-reorder failover
  chain — a failed call hands off to the next configured provider.
- **Your choice of STT** — Deepgram (streaming, interim results) or
  fully-local `whisper-cli` (bundled whisper.cpp sidecar; tiny / base /
  small models downloaded on demand from Hugging Face).
- **Global hotkey** — `Cmd`/`Ctrl+Alt+Space` shows/hides the bar's
  input, rebindable (see [Hotkeys](#hotkeys)).
- **Deep links** — `marvis://ask?text=...` focuses the bar and fires an Ask;
  any other `marvis://` link just surfaces the app.
- **Onboarding + permission gates** — guided first-run wizard; the app
  unlocks only after onboarding completes and screen consent exists —
  macOS Screen Recording, the Linux portal pick, no preflight on
  Windows.
- **Local persistence** — sessions, messages, transcripts, and
  summaries in SQLite at `~/.marvis/marvis.db` (`0600` on macOS/Linux).

## Requirements

| | Screen capture | System audio | Screen consent |
| --- | --- | --- | --- |
| macOS (Apple Silicon / Intel) | ScreenCaptureKit | ScreenCaptureKit | Screen Recording permission |
| Windows (x86_64) | Windows.Graphics.Capture | WASAPI loopback | none — WGC is the consent boundary |
| Linux (x86_64) | XDG portal + PipeWire | PulseAudio monitor | portal picker (persisted token) |

Build tooling:

- [Bun](https://bun.sh) 1.3+
- [Rust](https://rustup.rs) stable toolchain (for the Tauri core)
- macOS: Xcode Command Line Tools (`xcode-select --install`)
- Windows: Visual Studio C++ workload (MSVC)
- Linux: `webkit2gtk-4.1`, `gtk-3`, `appindicator`, PipeWire and
  PulseAudio dev packages — see `.github/workflows/marvis-build.yml`
  for the exact `apt` list
- Screen Recording + Microphone prompts appear in-app on first run
  where the OS has them — Linux needs neither (the portal dialog is
  the consent; PulseAudio mics need no permission)
- At least one provider: an API key for OpenAI / Anthropic / Gemini /
  OpenRouter / a compatible endpoint, **or** a running
  [Ollama](https://ollama.com) daemon for local models

## Getting started

```bash
# install dependencies
bun install

# run the desktop app (vite dev server + tauri dev)
bun run build:dev
```

On first launch, the onboarding wizard walks you through appearance →
screen consent → a provider key (validated before saving, or pick an
Ollama model) → optional voice models for local transcription. The
floating bar appears once onboarding completes and consent exists —
instantly on Windows (no preflight), after the portal pick on Linux.

Other root scripts:

```bash
bun run dev           # vite/next dev servers only (no desktop shell)
bun run build         # build all workspaces
bun run lint          # lint all workspaces
bun run check-types   # typecheck all workspaces
```

Rust unit tests for the Tauri core:

```bash
cd apps/native/src-tauri && cargo test
```

## Hotkeys

The one global chord is rebindable under `[hotkeys]` in
`~/.marvis/config.toml` or Settings → Hotkeys:

| Action | Shortcut |
| --- | --- |
| Show / hide the input | `Cmd`/`Ctrl+Alt+Space` |

Fixed keys inside the bar (they fire only while it's focused):

| Action | Shortcut |
| --- | --- |
| Open settings | `Cmd`/`Ctrl+,` |
| Send | `Enter` |
| New line | `Shift+Enter` |
| Send with screenshot | `Cmd`/`Ctrl+Enter` |

The bar itself moves by pointer drag or the Settings → Bar edge picker —
there are no fixed move/scroll/click-through shortcuts.

## Repository layout

Turborepo monorepo managed with Bun workspaces:

```text
apps/
  native/        # Marvis desktop app (see apps/native/README.md)
    src/         #   React 19 webview (Vite + Tailwind 4) — UI only
    src-tauri/   #   Rust core: windows, capture, audio, STT, LLM adapters,
                 #   storage, hotkeys, permissions, deep links
  web/           # Next.js marketing/web app
packages/
  ui/            # @marvis/ui — shared component library
functions/       # Waitlist endpoint (Neon/Hono)
db/              # Waitlist SQL migrations
assets/          # Brand SVGs and the landing-page prototype
docs/
  superpowers/   # design specs and implementation plans
```

The Rust core (`apps/native/src-tauri/src/`) is where everything sensitive
happens — screen capture, audio, keystorage, LLM streaming, persistence.
The React webview renders views selected by `?view=` per window and talks
to the core through typed `invoke()` commands and events. Secrets and image
bytes never cross that boundary. See
[`apps/native/README.md`](apps/native/README.md) for the full architecture.

## Data locations

Everything Marvis writes lives in a single dotdir — `~/.marvis`
(`%USERPROFILE%\.marvis` on Windows), created `0700` where the OS has
mode bits — no platform application-data dir, no temp files:

```text
~/.marvis/
├── keys.json       # 0600 — plaintext provider → API key map (see note)
├── config.toml     # 0644 — non-secret prefs (providers, hotkeys, window pos)
├── marvis.db       # 0600 — SQLite sessions, messages, transcripts, summaries
└── models/whisper/ # downloaded ggml whisper models (tiny / base / small)
```

**Honest note:** `keys.json` is plaintext inside the `0700` root
(mode bits are macOS/Linux; on Windows the user-profile ACL covers it)
— there is no encryption in the current build. Keys are never
serialized to the UI (masked `…last4` only) and never logged.

## Roadmap

North star: a screen-aware copilot with a meeting assistant built in —
its own memory on your machine, tools and skills, and simple agentic
loops under your approval. The full plan lives in
[ROADMAP.md](ROADMAP.md); phases are ordered by dependency, not date.

- **Done** — overlay bar, provider failover chain, screen-aware Ask,
  Listen (mic + system-audio transcription, diarized live transcript,
  rolling summaries), dictation, hotkey rebinding, deep links, tray,
  onboarding wizard, Windows + Linux ports with CI bundles for all
  three OSes.
- **Phase 0 — Ship-ready** — signed + notarized builds, auto-update,
  opt-in crash reporting, package-manager distribution.
- **Phase 1 — Deepen the loop** — session history polish, prompt
  presets, Ollama model management, Gemini search grounding, transcript
  export, WAV playback.
- **Phases 2–4 — Memory, Skills & tools, Agentic loops** — local recall,
  guarded tool execution (MCP), and bounded observe→propose→approve
  cycles. All `exploring` — design discussions on GitHub issues first.

## Contributing

Contributions are welcome — bug reports and feature requests go to
[GitHub issues](https://github.com/MarvisLLC/marvis/issues), and pull
requests are appreciated. A few conventions that keep the codebase
consistent:

- **bun** for all package management and scripts.
- Rust changes: run `cargo test` in `apps/native/src-tauri` — unit tests
  live beside their modules; `AppState::for_test` binds every filesystem
  path under a temp root, so tests never touch `~/.marvis`.
- React components use arrow functions and named exports; icons come
  from `@marvis/ui` with `Icon`-suffixed names (`XIcon`, `InfoIcon`).
- UI work should follow `apps/native/DESIGN.md` — one accent hue,
  hairline borders, motion only when it communicates state.
- The command/event contract is dual-sided: change
  `apps/native/src/lib/commands.ts` / `events.ts` and the Rust emits
  together.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full contributor guide.

## Security

Marvis handles screen captures, microphone audio, and provider API keys. If
you find a vulnerability, please report it privately to
<support@getmarvis.com> instead of opening a public issue — see
[SECURITY.md](SECURITY.md) for the full policy.

## License

Marvis is open source under the [Apache License 2.0](LICENSE) © 2026 Marvis
AI.

---

Inspired by [Cheating Daddy](https://github.com/sohzm/cheating-daddy) and [Glass](https://github.com/pickle-com/glass).
