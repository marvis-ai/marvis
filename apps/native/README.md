# Marvis Desktop

**Private, personal AI that lives on your desktop.** Marvis is an
always-on-top floating bar for macOS, Windows, and Linux: it can see
your screen (with permission), hear your meetings, and answer through
the AI providers *you* configure — your keys, your history, and your
screen data never leave the machine except in the requests you choose
to send.

Built with **Tauri 2** (Rust core) and **React 19 / TypeScript / Vite**
(webview UI). The Rust core owns everything sensitive — capture, audio,
keys, storage — the webview is UI only.

> This package is `@marvis/native`, the desktop app in the
> [Marvis monorepo](../../README.md). macOS, Windows, and Linux build
> from one codebase — platform seams (`capture`, `audio`,
> `permissions`) sit behind per-OS `{macos,windows,linux}.rs` modules.

## Features

- **Floating bar** — a breathing capsule that morphs into an input pill,
  then into a chat/listen card. Docks to any screen edge (top / bottom /
  left / right rail), drags anywhere, remembers its position.
- **Ask** — type a question (`Enter` / `Cmd`/`Ctrl+Enter` sends,
  `Shift+Enter`
  adds a line): the latest captured screen frame goes with it and the
  answer streams back as markdown. An optional **vision provider** reads
  the frame first and answers over its text description, so chat
  providers never need image support.
- **Provider failover chain** — OpenAI, Anthropic, Gemini, OpenRouter,
  Ollama, or any OpenAI-compatible endpoint (bring your own keys). Drag
  to re-rank, toggle to disable; a failed provider hands off to the next.
- **Listen** — dual-channel meeting capture: mic ("me") + system audio
  ("them") → streaming speech-to-text → live transcript with speaker
  labels → rolling AI summaries every 5 turns.
- **Your choice of STT** — Deepgram (WebSocket streaming, interim
  results) or fully-local `whisper-cli` (bundled whisper.cpp sidecar;
  tiny / base / small models downloaded on demand from Hugging Face).
- **Screen capture** — continuous ~4 fps stream (ScreenCaptureKit on
  macOS, Windows.Graphics.Capture, XDG portal + PipeWire on Linux) into
  an in-memory ring buffer (120 frames / 64 MB, JPEG @ 384 px).
  Screenshots are never written to disk.
- **Hotkeys** — `Cmd`/`Ctrl+Alt+Space` shows/hides the bar's input
  (global, rebindable in Settings → Hotkeys). Inside the bar:
  `Cmd`/`Ctrl+,` opens settings, `Enter` / `Cmd`/`Ctrl+Enter` sends,
  `Shift+Enter` adds a line.
- **`marvis://` deep links** — `marvis://ask?text=…` asks; any other
  `marvis://*` link focuses the bar.
- **Onboarding + permission gates** — guided first-run wizard, explicit
  screen-consent and microphone flows, and a tray icon
  (Show/Hide · Settings · Quit).
- **Local-first privacy** — everything under `~/.marvis` (0700 on
  unix). See [Data & privacy](#data--privacy).

## Requirements

- **macOS** (Apple Silicon or Intel), **Windows** x86_64, or **Linux**
  x86_64 — capture uses ScreenCaptureKit / Windows.Graphics.Capture /
  XDG portal + PipeWire; system audio uses SCK / WASAPI loopback /
  PulseAudio monitor
- [Bun](https://bun.sh) 1.3+
- [Rust](https://rustup.rs) stable toolchain
- macOS builds: Xcode Command Line Tools (`xcode-select --install`);
  Windows builds: MSVC toolchain; Linux builds: `webkit2gtk-4.1`,
  `gtk-3`, PipeWire/PulseAudio dev packages (apt list in
  `.github/workflows/marvis-build.yml`)
- Screen consent + Microphone permission — requested in-app on first
  run where the OS has them (no screen preflight on Windows; Linux
  consents via the portal pick)
- At least one provider: an API key (OpenAI / Anthropic / Gemini /
  OpenRouter / compatible endpoint) **or** a local
  [Ollama](https://ollama.com) daemon
- Optional for local transcription: a validated `whisper-cli-<target>` artifact
  staged by `src-tauri/scripts/build-marvis.sh`, or a PATH/Homebrew/user
  fallback

## Getting started

From the repo root:

```bash
bun install          # install workspace deps
bun run build:dev    # stage a dev whisper-cli if present, then vite + tauri dev
```

If `src-tauri/binaries/whisper-cli-<target>` already exists, `build:dev` copies
that validated artifact beside `target/debug/Marvis` so the debug app reports
`Bundled with Marvis`. If no validated artifact exists, development continues
with the PATH/Homebrew/user fallback instead of failing.

First launch walks you through onboarding: appearance → screen-recording
permission → a provider key → optional voice models. The bar appears
when onboarding completes and screen permission is granted.

Other scripts in this package:

```bash
bun run dev          # vite only — webview in a browser at :1420
bun run build        # tsc + vite build → dist/
bun run tauri build  # release .app bundle (needs a staged whisper-cli binary)
bun test             # frontend tests (bun test)

cd src-tauri && cargo test   # Rust unit tests
```

### Release Whisper packaging

Release packaging consumes the exact artifacts from
`.github/workflows/marvis-build.yml`; do not build or commit binaries
locally. On a macOS runner with the target toolchain, download the
artifacts for the exact workflow run, stage and validate one target,
then build and verify the signed app:

```bash
RUN_ID=123456789
TARGET=aarch64-apple-darwin # use x86_64-apple-darwin on an Intel runner
mkdir -p /tmp/whisper-artifact
 gh run download "$RUN_ID" --repo MarvisLLC/marvis \
  --name "whisper-cli-$TARGET" --dir /tmp/whisper-artifact
cd apps/native/src-tauri
bash scripts/build-marvis.sh --stage \
  "/tmp/whisper-artifact/whisper-cli-$TARGET" \
  --checksum "/tmp/whisper-artifact/whisper-cli-$TARGET.sha256" \
  --target "$TARGET"
bash scripts/check-task-2-packaging.sh --target "$TARGET"
cd ..
bun install --frozen-lockfile
bun run tauri build --target "$TARGET"
cd src-tauri
bash scripts/verify-release-app.sh \
  "target/$TARGET/release/bundle/macos/Marvis.app" "$TARGET"
```

The workflow is manual-only: the default run builds only unsigned CLI
artifacts, and app packaging is enabled only by an explicit
release-candidate input. Set `package_app` only for an intentional
release candidate, after provisioning the signing certificate and
identity in the runner keychain. `APPLE_SIGNING_IDENTITY` is a
prerequisite, not certificate provisioning; no credentials are stored
in this repository. The package jobs fail closed when the identity is
absent, and `verify-release-app.sh` always runs strict recursive
`codesign --verify --deep --strict` verification. An unsigned or
invalid app is not reported as a release artifact. `/tmp` and
`binaries/` are staging/build locations and remain ignored by Git.

The `.sha256` files generated by the pinned build and checked by `--stage` are
build-output integrity checks for the exact downloaded artifact. They are not a
trusted release identity or a substitute for signed release provenance. A
release operator must retain the exact workflow run/artifact provenance and
record the verified checksums in the release notes or controlled release
manifest; never invent expected hashes or replace SHA-256 verification.

## Architecture

One webview bundle serves every window: each `WebviewWindow` loads
`index.html?view=<label>` and `src/App.tsx` switches on the query —
`bar` (default), `alert`, `prefs`.

```text
┌────────────────────────── Rust core (src-tauri) ──────────────────────────┐
│  AppState (lib.rs) — Mutex/Arc bundle wired via app.manage()              │
│                                                                          │
│  Gate: NeedsPermission ──(onboarding_done ∧ screen access)──> Main        │
│   enter_main: full hotkey set + start capture                             │
│   leave_main: cancel ask, stop capture, limited hotkeys                   │
│                                                                          │
│  ask.rs     question + newest ring frame → failover chain → stream tokens │
│  listen.rs  mic+system PCM → STT → TurnAssembler → transcripts, summaries │
│  llm/       Provider trait + adapters: openai anthropic gemini ollama     │
│             compat (OpenAI-compatible incl. OpenRouter) — shared SSE/     │
│             NDJSON streamers                                              │
│  stt/       SttProvider trait: deepgram (WS streaming) | whisper (sidecar)│
│  audio/     AudioSource trait: MicSource (cpal) | SystemAudioSource (SCK) │
│  capture/   FrameSource → MacosCapture (ScreenCaptureKit) → RingBuffer    │
│  windows/   WindowPool (bar/alert/prefs) + layout math + movement animator│
│  hotkey.rs  global-shortcut → Action → dispatch closure                   │
│  deeplink.rs marvis:// URL → Action → dispatch closure                    │
│  tray.rs    menu-bar icon                                                 │
│  keystore.rs keys.json (0600, atomic writes, masked_status for UI)        │
│  config.rs  config.toml (serde-defaulted sections, config_set validation) │
│  storage.rs marvis.db: sessions · ai_messages · transcripts · summaries   │
│  voice_models.rs whisper model catalog/download (sha1, cancel token)      │
│  permissions.rs screen (CoreGraphics) + mic (AVFoundation) status/request │
│  prompts.rs embedded prompts: live / summary / screen-context             │
│  paths.rs   ~/.marvis data root (0700)                                    │
└──────────────────────────────────────────────────────────────────────────┘
```

### Frontend ⇄ backend contract

- **Commands** — typed wrappers in `src/lib/commands.ts` mirror
  `tauri::generate_handler!` names one-to-one.
- **Events** — names in `src/lib/events.ts` mirror the Rust emits:
  `app:state` (gate), `keystore:changed`, `config:changed`,
  `ask:state|chunk|done|error` (bar only), `listen:state|turn|summary|error`,
  `alert:show`, `prefs:mode`, `whisper:download-*`,
  `capture:permission-needed`.
- **Window sizing is Rust-owned** — the webview reports desired size
  (`window_set_bar_expanded`, `window_adjust_height`); `windows/layout.rs`
  computes geometry in logical pixels and `movement.rs` animates it.
- **Resync-over-emit** — every live payload is also readable via a command
  (`ask_current`, `alert_current`, `prefs_mode`, `listen_status`), so an
  emit that races a loading webview is never lost.

### How the pipelines flow

- **Ask:** `ask_send` → newest `RingBuffer` frame → optional vision
  provider describes it (`<screen_context>`) → try each candidate in
  `providers.order` until one streams → both sides persist to
  `ai_messages` in the single `ask` session.
- **Listen:** `MicSource` + `SystemAudioSource` → normalized 16 kHz mono
  `PcmChunk`s → `SttProvider` → `TurnAssembler` state machine (interim vs
  final; 1.5 s of silence closes a turn) → `transcripts` rows +
  `listen:turn` events → every 5 closed turns the LLM writes a
  `summaries` row (tldr / bullets / follow-ups / topic).

## Tech stack

| Layer | Technology |
| --- | --- |
| Shell | Tauri 2 (tray-icon; liquid glass + `macos-private-api` on macOS) |
| Frontend | React 19, TypeScript, Vite 8, Tailwind CSS 4, `@marvis/ui` (workspace), react-markdown + remark-gfm |
| Backend | Rust 2021 — tokio, reqwest (rustls, SSE/NDJSON streaming), serde, parking_lot, thiserror/anyhow |
| Storage | rusqlite (bundled SQLite, WAL, FK cascade) |
| Screen | `screencapturekit` (macOS), `windows-capture` / WGC (Windows), `ashpd` portal + `pipewire` (Linux); `image` (JPEG encode/resize) |
| Audio | `cpal` (mic); system audio: ScreenCaptureKit (macOS), `wasapi` loopback (Windows), PulseAudio monitor (Linux) → 16 kHz mono PCM |
| STT | `tokio-tungstenite` (Deepgram WS), `whisper-cli` sidecar process (whisper.cpp v1.9.2) |
| Platform FFI | `objc2`, `objc2-av-foundation`, `block2`, CoreGraphics (macOS); `windows` crate (Windows); `ashpd` (Linux) |
| Tauri plugins | global-shortcut, deep-link, opener, liquid-glass |
| Package manager | [Bun](https://bun.sh) + Turbo (monorepo) |

## Project structure

```text
apps/native/
├── index.html                  # single entry; ?view= selects the view
├── vite.config.ts              # react + tailwind plugins; '@' → packages/ui/src
├── DESIGN.md                   # the design system spec (tokens, motion, rules)
├── src/                        # webview UI (React + TS + Tailwind)
│   ├── App.tsx                 # ?view= router: bar (default) | alert | prefs
│   ├── views/
│   │   ├── Bar.tsx             # unified overlay: capsule ⇄ input ⇄ chat/listen
│   │   ├── AlertToast.tsx      # auto-dismissing error/info toast window
│   │   └── Prefs.tsx           # settings + onboarding window (mode-switch)
│   ├── components/
│   │   ├── ChatSection.tsx     # multi-turn ask conversation (markdown stream)
│   │   ├── ListenSection.tsx   # live transcript + summaries + waveform
│   │   ├── Iris.tsx            # the animated iris mark
│   │   ├── RetryCard.tsx       # boot-failure retry surface
│   │   └── prefs/              # Settings tabs (General/Bar/Providers/Hotkeys/
│   │                           #   Privacy/About), Onboarding wizard, VoiceSetup,
│   │                           #   VisionSection, ProviderCard, shared bits/types
│   └── lib/
│       ├── commands.ts         # typed invoke() wrappers — the command surface
│       ├── events.ts           # event-name constants + useTauriEvent hook
│       ├── classes.ts          # shared Tailwind class bundles (BTN_*, CHIP…)
│       ├── providers.ts        # provider catalog metadata for the UI
│       ├── format.ts / theme.ts# formatting helpers; accent/appearance applier
│       └── vite-env.d.ts
└── src-tauri/                  # Rust core
    ├── src/
    │   ├── lib.rs              # AppState, commands, gate, dispatch closures
    │   ├── main.rs             # thin entry → marvis_lib::run()
    │   ├── ask.rs              # ask pipeline (frame + failover + stream)
    │   ├── listen.rs           # listen orchestration + TurnAssembler
    │   ├── llm/                # mod.rs (trait/factory) + anthropic compat
    │   │                       #   gemini ollama openai adapters
    │   ├── stt/                # mod.rs (trait/factory) + deepgram whisper
    │   ├── audio/              # mod.rs (PCM normalize) + mic system sources
    │   ├── capture/            # mod.rs (RingBuffer/Frame) + macos.rs (SCK)
    │   ├── windows/            # mod.rs (WindowPool) + layout movement
    │   ├── hotkey.rs  deeplink.rs  tray.rs  permissions.rs
    │   ├── keystore.rs  config.rs  storage.rs  paths.rs  prompts.rs
    │   └── voice_models.rs     # whisper model download manager
    ├── prompts/                # marvis-live / marvis-summary / marvis-screen
    ├── scripts/                # whisper-cli build + packaging check + fixtures
    ├── binaries/               # whisper-cli-<triple> sidecars (CI-staged)
    ├── capabilities/           # default.json window permissions
    ├── icons/                  # tray + app icons (embedded via include_bytes!)
    ├── build.rs                # debug externalBin shim + Swift rpath mirroring
    ├── tauri.conf.json         # productName Marvis, com.getmarvis.marvis
    └── Info.plist
```

## Data & privacy

Everything Marvis stores lives in `~/.marvis`
(`%USERPROFILE%\.marvis` on Windows), created `0700` on unix:

| File | Mode | Contents |
| --- | --- | --- |
| `config.toml` | 0644 | Non-secret prefs: app, provider order/switches/models, hotkeys, window position, compat endpoint, vision, STT pick |
| `keys.json` | 0600 | Plaintext provider → API key map. Atomic tmp→rename writes; a corrupt file is quarantined, never silently destroyed |
| `marvis.db` | 0600 | SQLite (WAL): `sessions`, `ai_messages`, `transcripts`, `summaries`, FK cascade |
| `models/whisper/` | — | Downloaded ggml whisper models (tiny / base / small) |

Honest caveats, by design:

- **Keys are plaintext on disk** inside the 0700 root — there is no
  encryption and no Keychain vault in the current build. Plaintext never
  reaches the UI (masked `…last4` only) and is never logged.
- **Screen frames are memory-only** — the ring buffer is never flushed to
  disk; a frame leaves the machine only when you send an ask.
- **Outbound traffic is exactly what you configure** — LLM calls go to
  your provider chain; STT goes to Deepgram or stays local with
  whisper-cli; model downloads hit Hugging Face only when you click
  download. No telemetry, no accounts, no sync.

## Contributing

Contributions are welcome. A few conventions that keep the codebase
consistent:

- **bun** for all package management and scripts.
- Rust changes: run `cargo test` in `src-tauri` — unit tests live beside
  their modules; `AppState::for_test` binds every filesystem path under a
  temp root, so tests never touch `~/.marvis`.
- React components use arrow functions and named exports; icons come
  from `@marvis/ui` with `Icon`-suffixed names (`XIcon`, `InfoIcon`).
- UI work should follow `DESIGN.md` — one accent hue, hairline borders,
  motion only when it communicates state.
- The command/event contract is dual-sided: change `commands.ts` /
  `events.ts` and the Rust emits together.

## License

MIT — same as the Marvis monorepo (see the root `package.json`).
