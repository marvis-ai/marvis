<p align="center">
  <img src="assets/marvis-logo.svg" alt="Marvis" width="260">
</p>

<p align="center">
  <strong>Private / Personal AI for All</strong> — a privacy-first AI co-pilot that lives on your desktop.
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/platform-macOS-lightgrey.svg" alt="Platform: macOS">
  <a href="https://github.com/MarvisLLC/marvis/stargazers"><img src="https://img.shields.io/github/stars/MarvisLLC/marvis" alt="GitHub stars"></a>
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
- **Ask** — type a question or press `Cmd+Enter`; Marvis grabs the latest
  screen frame, sends it with your prompt to your LLM, and streams the
  answer back as markdown. Screen content stays in an in-memory ring buffer
  (~60 s horizon, capped at 120 frames / 64 MB) — screenshots are never
  written to disk. An optional **vision provider** can read the frame first
  and answer over its text description.
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
- **Global hotkey** — `Cmd+Alt+Space` shows/hides the bar's input,
  rebindable (see [Hotkeys](#hotkeys)).
- **Deep links** — `marvis://ask?text=...` focuses the bar and fires an Ask;
  any other `marvis://` link just surfaces the app.
- **Onboarding + permission gates** — guided first-run wizard; the app
  unlocks only after onboarding completes and Screen Recording is granted.
- **Local persistence** — sessions, messages, transcripts, and summaries in
  SQLite at `~/.marvis/marvis.db` (`0600`).

## Requirements

- **macOS** (Apple Silicon or Intel). Screen/audio capture uses
  ScreenCaptureKit; permissions use CoreGraphics + AVFoundation.
- [Bun](https://bun.sh) 1.3+
- [Rust](https://rustup.rs) stable toolchain (for the Tauri core)
- Xcode Command Line Tools (`xcode-select --install`)
- Screen Recording + Microphone permission — requested in-app on first run
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
Screen Recording permission → a provider key (validated before saving, or
pick an Ollama model) → optional voice models for local transcription. The
floating bar appears once onboarding completes and screen permission is
granted.

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
| Show / hide the input | `Cmd+Alt+Space` |

Fixed keys inside the bar (they fire only while it's focused):

| Action | Shortcut |
| --- | --- |
| Open settings | `Cmd+,` |
| Send | `Enter` |
| New line | `Shift+Enter` |
| Send with screenshot | `Cmd+Enter` |

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

Everything Marvis writes lives in a single dotdir created with `0700`
permissions — no `~/Library/Application Support`, no temp files:

```text
~/.marvis/
├── keys.json       # 0600 — plaintext provider → API key map (see note)
├── config.toml     # 0644 — non-secret prefs (providers, hotkeys, window pos)
├── marvis.db       # 0600 — SQLite sessions, messages, transcripts, summaries
└── models/whisper/ # downloaded ggml whisper models (tiny / base / small)
```

**Honest note:** `keys.json` is plaintext inside the `0700` root — there is
no encryption in the current build. Keys are never serialized to the UI
(masked `…last4` only) and never logged.

## Roadmap

- **Done** — overlay bar, provider failover chain, screen-aware Ask,
  hotkey rebinding, deep links, tray icon, onboarding wizard.
- **Done** — Listen: mic + system-audio transcription
  (Deepgram / bundled whisper.cpp), live transcript, rolling summaries.
- **Next** — session history UI, prompt presets, Ollama model management,
  Gemini search grounding toggle.

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

## Security

Marvis handles screen captures, microphone audio, and provider API keys. If
you find a vulnerability, please report it privately to
<chenillen@gmail.com> instead of opening a public issue.

## License

Marvis is open source under the [MIT License](LICENSE) © 2026 Marvis AI.

---

Inspired by [Cheating Daddy](https://github.com/sohzm/cheating-daddy) and [Glass](https://github.com/pickle-com/glass).
