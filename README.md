# Marvis

**Private / Personal AI for All** — a privacy-first AI co-pilot that lives on your desktop.

Marvis is an always-available assistant that floats above your workspace as a
small translucent bar. It can see your screen (with your permission), answer
questions about what's on it, and stream responses into an overlay panel —
all while keeping your API keys, history, and screen data entirely on your
machine. No accounts, no cloud sync, nothing leaves your device except the
requests you send to the LLM provider you choose.
**Tauri 2 + Rust + React**, where the Rust core owns all sensitive logic and the webview is UI only.

## Features

- **Overlay assistant** — a frameless, always-on-top bar with draggable,
  liquid-glass panels (Ask / Listen / Settings) that stack underneath it.
- **Ask** — type a question or press `Cmd+Enter`; Marvis grabs the latest
  screen frame, sends it with your prompt to the configured LLM, and streams
  the answer back as markdown. Screen content stays in an in-memory ring
  buffer (~60 s horizon, capped at 120 frames / 64 MB) — screenshots are
  never written to disk.
- **Bring your own model** — OpenAI, Anthropic, Gemini, or fully local
  inference via [Ollama](https://ollama.com) (no API key needed).
- **Encrypted keystore** — provider API keys live in `keys.enc`, encrypted
  with AES-256-GCM under a passphrase-derived Argon2id key. The UI only ever
  sees masked keys (`openai: set ••••1234`). Keys are validated against the
  provider before they're stored.
- **Gated startup** — the app unlocks in stages: passphrase → screen-recording
  permission → full feature set. Locking the keystore tears down capture,
  hides panels, and cancels in-flight requests.
- **Global hotkeys** — summon, move, scroll, snap, and click-through the
  overlay from anywhere (see [Hotkeys](#hotkeys)).
- **Deep links** — `marvis://ask?text=...` focuses the bar and fires an Ask;
  any other `marvis://` link just surfaces the app.
- **Local persistence** — sessions and messages in SQLite at
  `~/.marvis/marvis.db` (`0600`).

## Requirements

- **macOS** (Apple Silicon or Intel). Screen capture uses ScreenCaptureKit;
  the liquid-glass effect uses `NSGlassEffectView` on macOS 26+ and falls back
  to `NSVisualEffectView` on earlier versions.
- [Bun](https://bun.sh) 1.3+
- [Rust](https://rustup.rs) stable toolchain (for the Tauri core)
- Screen Recording permission — granted on first run via the built-in prompt
- An API key for OpenAI / Anthropic / Gemini, **or** a running
  [Ollama](https://ollama.com) daemon for local models

## Getting started

```bash
# install dependencies
bun install

# run the desktop app (vite dev server + tauri dev)
bun run build:dev
```

On first launch:

1. **Set a passphrase** — creates the encrypted `keys.enc` keystore.
2. **Grant Screen Recording** — macOS prompt; required for screen-aware Ask.
3. **Add a provider key** — Settings panel → paste an API key (validated
   before saving), or pick an Ollama model for local inference.

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

Defaults (configurable under `[hotkeys]` in `~/.marvis/config.toml`):

| Action | Shortcut |
| --- | --- |
| Toggle overlay visibility | `Cmd+/` |
| Ask (send / screen-only ask) | `Cmd+Enter` |
| Move bar | `Cmd+↑ ↓ ← →` |
| Snap bar to display edge | `Cmd+Shift+← →` |
| Move bar to display *n* | `Cmd+Shift+<n>` |
| Toggle click-through | `Cmd+M` |
| Scroll Ask panel | `Cmd+Shift+↑ ↓` |
| Manual screenshot ask | `Cmd+Shift+S` |

## Repository layout

Turborepo monorepo managed with Bun workspaces:

```text
apps/
  native/        # Marvis desktop app
    src/         #   React 19 webview (Vite + Tailwind 4) — UI only
    src-tauri/   #   Rust core: windows, capture, keystore, LLM adapters,
                 #   storage, hotkeys, permissions, deep links
  web/           # Next.js marketing/web app (early scaffold)
packages/
  ui/            # @marvis/ui — shared component library
docs/
  superpowers/   # design specs and implementation plans
```

The Rust core (`apps/native/src-tauri/src/`) is where everything sensitive
happens — screen capture, key encryption, LLM streaming, persistence. The
React webview renders views selected by `?view=` per window and talks to the
core through typed `invoke()` commands and events. Secrets and image bytes
never cross that boundary.

## Data locations

Everything Marvis writes lives in a single dotdir created with `0700`
permissions — no `~/Library/Application Support`, no temp files:

```text
~/.marvis/
├── keys.enc      # 0600 — Argon2id + AES-256-GCM API keyring
├── config.toml   # 0644 — non-secret prefs (models, hotkeys, window pos)
├── marvis.db     # 0600 — SQLite sessions & messages
└── models/       # (planned) local model files
```

## Roadmap

- **Phase 1 (current)** — overlay windows, keystore, screen capture, LLM
  adapters, Ask end-to-end.
- **Phase 2** - Design with design.md and implement landing page
- **Phase 2** — Listen: mic + system-audio transcription (Deepgram / local
  whisper.cpp), live meeting summaries.
- **Phase 3** — keybind editor UI, session history UI, prompt presets,
  Ollama model management, Gemini search grounding toggle.

## License

MIT
