# Marvis — Glass Feature Rewrite, Phase 1 Design

Date: 2026-09-18
Status: Approved design (pending written-spec review)
Reference: `~/Codes/OpenSource/glass` (Electron + Lit, read-only reference)

## Goal

Re-implement Glass's feature set inside Marvis's architecture: Tauri 2, Rust core
owns all sensitive logic, React 19 webview is UI only. This spec covers **Phase 1**:
the overlay window system, the keystore, screen capture, LLM adapters, and the Ask
flow end-to-end. Listen/STT/summary and advanced settings are Phase 2/3.

## Feature inventory (from Glass) and disposition

| Glass subsystem | Phase | Notes |
| --- | --- | --- |
| Overlay windows (header bar + ask/listen/settings/shortcut-settings panels) | **1** | `header` → `bar`. Shortcut-settings panel deferred to Phase 3 (keybind editor UI). |
| Ask (screenshot → prompt → SSE stream → persist) | **1** | Uses ring-buffer frame instead of on-disk `screencapture` temp files. |
| Screen capture | **1** | Continuous ScreenCaptureKit stream → in-memory ring buffer (arch rule 2). |
| API-key management (AES-GCM, model selection, validation) | **1** | `keys.enc` encrypted with a random DEK stored in the macOS Keychain behind a `userPresence` ACL — unlock = Touch ID / Face ID / system password (arch rule 1/4). |
| LLM providers: OpenAI, Anthropic, Gemini, Ollama | **1** | Rust `Provider` trait + SSE adapters. `openai-glass`/Portkey **dropped** (hosted service). |
| Global shortcuts | **1** | `tauri-plugin-global-shortcut`; keybinds in `config.toml`; editor UI in Phase 3. |
| macOS permissions (screen recording, mic, keychain gate) | **1** | Screen-recording + mic checks; keychain gate = the DEK item's `SecAccessControl` ACL (user presence per read). |
| Session/message persistence (SQLite) | **1** | rusqlite; `sessions`, `ai_messages`, `provider_keys` (encrypted blobs live in `keys.enc`, not DB). |
| Deep links | **1** | `marvis://` scheme (replaces `pickleglass://`). Route: focus bar; `marvis://ask?text=` prefills Ask. No auth callbacks (no Firebase). |
| Listen (mic + system audio STT, debounced turns, WS session renewal) | 2 | Mic via cpal; system audio via ScreenCaptureKit audio or `SystemAudioDump`-style helper — decided in Phase 2 spec. |
| Live summary (every 5 turns → structured analysis) | 2 | Reuses Phase 1 LLM adapters. |
| Presets/prompt templates UI | 3 | Templates ship built-in from Phase 1 (Ask prompt); editor in Phase 3. |
| Local AI (Ollama service mgmt, whisper.cpp download) | 3 | Ollama *provider* works in Phase 1 (HTTP API); install/model-pull UX in Phase 3. |
| Settings panel UI (model select, key mgmt, shortcuts editor) | 1 minimal / 3 full | Phase 1: provider keys + model select + unlock. |
| Session history UI (sessions list/detail) | 3 | Glass renders it in dropped `pickleglass_web`; Phase 3 adds a Marvis surface. Commands persist from Phase 1. |
| Gemini Google-search grounding toggle | 3 | Provider-level option; off by default (privacy). |
| Firebase auth + sync, embedded `pickleglass_web`, auto-updater, Windows/Linux, content-protection toggle | **dropped** | Conflicts with privacy-first local-only architecture; protection is always-on per arch rule 5. |

## Architecture

All core logic in `apps/native/src-tauri/src/`:

```text
src-tauri/src/
├── lib.rs            # builder, plugin/command/event registration, app state
├── main.rs           # entry
├── windows/
│   ├── mod.rs        # WindowPool (bar, ask, listen, settings), visibility orchestration
│   ├── layout.rs     # layout math: stack under bar, edge snap, clamp to display work area
│   └── movement.rs   # animated position/bounds interpolation (tick-based)
├── keystore/
│   └── mod.rs        # keys.enc load/save, AES-256-GCM, unlock state
├── system_auth.rs    # DekProvider: Keychain DEK behind userPresence ACL (+ LA-eval fallback for unsigned dev builds)
├── capture/
│   ├── mod.rs        # FrameSource trait, RingBuffer (≤120 frames / 64 MB, changed frames)
│   └── macos.rs      # ScreenCaptureKit impl (screencapturekit or cidre binding)
├── llm/
│   ├── mod.rs        # Provider trait, ProviderKind, ChatMessage/ImageContent types
│   ├── openai.rs     # SSE
│   ├── anthropic.rs  # SSE
│   ├── gemini.rs     # SSE (streamGenerateContent)
│   └── ollama.rs     # /api/chat stream (no key needed)
├── ask/
│   └── mod.rs        # orchestrator: frame → messages → stream → events → persist
├── prompts/
│   └── mod.rs        # system prompt builder (port of promptTemplates/promptBuilder)
├── storage/
│   └── mod.rs        # rusqlite: schema, sessions, ai_messages
├── config/
│   └── mod.rs        # config.toml load/save (non-secret prefs: models, hotkeys, window pos)
├── hotkey/
│   └── mod.rs        # global shortcut registration + dispatch
├── permissions/
│   └── mod.rs        # CGPreflightScreenCaptureAccess / request, mic status
└── deeplink/
    └── mod.rs        # marvis:// registration + open-url routing
```

The webview (`apps/native/src/`) is one Vite/React app; `?view=` selects the
component per window:

```text
src/
├── App.tsx            # view router by ?view=bar|ask|listen|settings
├── views/
│   ├── Bar.tsx        # logo, Listen toggle (stub Phase 2), Ask input, settings gear, status
│   ├── AskPanel.tsx   # streaming markdown answer, scroll, close
│   ├── ListenPanel.tsx# Phase 2 stub
│   └── SettingsPanel.tsx # unlock, provider key mgmt (masked), model select
├── components/        # @marvis/ui composition
└── lib/commands.ts    # typed invoke() wrappers + event hooks
```

## Data flow / key flows

### Unlock (keystore)

1. App starts → `keystore_status` → `Locked | Unset | Unlocked`.
2. `Unset` (no `keys.enc`): `keystore_init()` creates a random 32-byte DEK in
   the macOS Keychain (`kSecClassGenericPassword`, `userPresence` ACL — silent
   `SecItemAdd`, no prompt) → encrypt empty keyring → `keys.enc`
   (`source|nonce|ct`, os 0600). `Locked`: `keystore_unlock()` → Keychain
   `SecItemCopyMatching` on the ACL item — **this is the one system-auth
   prompt** (Touch ID / Face ID / system password) → AES-GCM open → keyring in
   Rust memory (`Zeroizing`).
3. React only ever sees masked status: `openai: set ••••1234 | unset`.
4. `keystore_set_key(provider, key)` validates (test call) then re-encrypts file.
5. `keystore_reset()` deletes `keys.enc` → `Unset` — the recovery path for
   obsolete/corrupt files (the Keychain DEK item is kept and reused).

### Ask

1. `ask_send(text)` (hotkey `Cmd+Enter` or Bar input) → show `ask` panel
   (animated), emit `ask:state{loading}`.
2. Latest frame from ring buffer → JPEG (~384px height, q80). No frame yet →
   text-only request (log).
3. Messages: system prompt (port of `pickle_glass_analysis`; transcript context
   is empty until Phase 2) + user text + image.
4. `Provider::stream_chat` → per-token `ask:chunk` events to `ask` window →
   `ask:done{full}`; persist user msg + assistant msg to session row.
5. Multimodal request fails w/ image error → retry text-only (Glass parity).
6. `ask_close` aborts stream (drop tokio task) and hides panel.

### Frames (capture)

- `FrameSource` trait: `start(cb: Fn(Frame))`, `Frame { rgba/jpeg bytes, w, h, ts }`.
- macOS impl: ScreenCaptureKit `SCStream` on a dedicated thread; diff-check
  (cheap hash/downsample compare) → push only *changed* frames into
  `RingBuffer` (cap 120 frames, 64 MB; evict oldest; ~60 s horizon).
- `capture_latest()` → most recent frame for ask. Capture starts after
  screen-recording permission granted; bar shows permission card otherwise.
- Crate risk noted below.

### Windows

- `bar`: 353×47, frameless, transparent, always-on-top, visible on all
  workspaces, content-protected, centered horizontally, 21 px under work-area
  top (Glass geometry). Draggable via `-webkit-app-region: drag` region.
- `ask` (600 w), `listen` (400 w), `settings` (240 w): child panel windows,
  same flags; shown/hidden with fade+slide animation stacked under `bar`
  (layout.rs computes non-overlapping rects; movement.rs interpolates ~200 ms).
- Content-driven resize (Glass `adjustWindowHeight` parity): panels report
  desired content height via `window_adjust_height(name, px)` (throttled in
  the webview); Rust clamps to `maxHeight` (ask/listen ≤900 px, settings
  ≤400 px) and animates the bounds, keeping the stack under `bar`.
- Click-through toggle (`Cmd+M`): `set_ignore_cursor_events` on all windows.
- `Cmd+Arrow` step-move bar (children follow); `Cmd+Shift+Arrow` snap to
  display edge; `Cmd+Shift+<n>` move bar to display n (hardcoded, Glass
  parity); display-remove → re-clamp to primary.
- Liquid glass: `tauri-plugin-liquid-glass` (hkandala) — the Tauri
  equivalent of Glass's `electron-liquid-glass`, same private
  `NSGlassEffectView` API. Applied **from Rust** at window creation
  (`app.liquid_glass().set_effect(window, LiquidGlassConfig{ variant:
  GlassMaterialVariant::Bubbles, corner_radius: … })`) — the webview is
  not involved. `Bubbles` variant matches Glass's choice; macOS <26
  automatically falls back to `NSVisualEffectView`. Requires
  `app.macOSPrivateApi: true` + `transparent: true` in tauri.conf.json
  and `liquid-glass:default` in capabilities.
- Header-state gate (Glass parity): panels only exist once keystore is
  unlocked AND screen permission granted; otherwise bar renders the
  unlock/permission card and only `Cmd+/` stays registered.

### Hotkeys (defaults, configurable in config.toml)

| Action | macOS |
| --- | --- |
| toggleVisibility | `Cmd+/` |
| nextStep (Ask toggle/send screen-only) | `Cmd+Enter` |
| moveUp/Down/Left/Right | `Cmd+Arrows` |
| toggleClickThrough | `Cmd+M` |
| scrollUp/Down (ask panel) | `Cmd+Shift+Up/Down` |
| snap to edge L/R | `Cmd+Shift+Left/Right` (hardcoded, like Glass) |
| move to display n | `Cmd+Shift+<n>` (hardcoded, like Glass) |
| manualScreenshot | `Cmd+Shift+S` → screen-only ask (same as `nextStep` w/ empty input) |
| previous/next response | `Cmd+[` / `Cmd+]` (Phase 3 response history) |

### Deep links

- Register `marvis://` as default protocol client (Info.plist `CFBundleURLTypes`
  - `tauri-plugin-deep-link`).
- `marvis://ask?text=...` → focus bar, open ask panel, prefill/send.
- `marvis://<anything>` → focus bar. No auth handling (no Firebase).

## Command/event surface (Tauri)

Commands (webview → Rust):
`keystore_status`, `keystore_init`, `keystore_unlock`, `keystore_reset`,
`keystore_lock`, `keystore_set_key`, `keystore_remove_key`, `model_validate_key`,
`model_get_selected`, `model_set_selected`, `model_list_available`,
`ask_send`, `ask_close`, `listen_stub` (Phase 2 placeholder),
`window_toggle_all`, `window_show_settings`, `window_hide_settings`,
`window_adjust_height`,
`permissions_status`, `permissions_request_screen`, `permissions_open_prefs`,
`capture_status`, `session_list`, `session_get`, `session_delete`,
`config_get`, `config_set`.

Events (Rust → webview):
`keystore:changed`, `ask:state`, `ask:chunk`, `ask:done`, `ask:error`,
`capture:permission-needed`, `windows:visibility`, `shortcuts:updated`,
`deeplink:ask`.

Secrets and image bytes never appear in commands or events.

## Data locations (`~/.marvis`)

All app data lives under a single dotdir — `~/.marvis` — created at first
launch with `0700` permissions. This consolidates Glass's three scattered
locations (`~/.pickleglass/config.json`, `~/.glass/whisper/*`, and
`~/Library/Application Support/Glass/pickleglass.db`).

```text
~/.marvis/
├── keys.enc        # 0600 — Keychain-DEK + AES-256-GCM keyring (rule 1)
├── config.toml     # 0644 — non-secret prefs (rule 4)
├── marvis.db       # 0600 — SQLite sessions/messages/(Phase 2: transcripts, summaries)
└── models/         # Phase 2/3 — whisper.cpp binary + model files, etc.
    └── whisper/
```

Nothing is written elsewhere: no `~/Library/Application Support`, no temp
screenshots, no plaintext fallbacks. `storage::db_path()`,
`config::path()`, and `keystore::path()` all resolve inside `~/.marvis`.

## config.toml (non-secret, per rule 4)

```toml
[models]
llm_provider = "openai"          # openai|anthropic|gemini|ollama
llm_model    = "gpt-4o"
stt_provider = "deepgram"        # phase 2
stt_model    = "nova-2"

[hotkeys]
toggle_visibility = "Cmd+\\"
next_step = "Cmd+Enter"
# ... (defaults above)

[window]
bar_x = 812                      # remembered position
bar_y = 21
```

## `~/.marvis/keys.enc` format (per rule 1)

```text
[1B dek_source][12B nonce][AES-256-GCM ciphertext || 16B GCM tag appended]
dek_source 0x01 = system-auth DEK; 0x02 reserved (future server-issued key)
DEK = random 32 bytes in the macOS Keychain, gated by a userPresence ACL
plaintext = JSON: { "openai": "sk-...", "anthropic": "...", "gemini": "...", "deepgram": "..." }
```

The DEK source sits behind a `DekProvider` trait (`system_auth.rs`) so the
keystore never knows how the key is obtained: `SystemAuthDekProvider`
(Keychain + `kSecAccessControlUserPresence`, plus an `LAContext.evaluatePolicy`
fallback for unsigned dev builds) is the production impl; source `0x02` is
reserved for a server-issued key arriving with the future auth system.

UX note: every `keystore_unlock` prompts once — the ACL makes macOS demand
user presence per read; no session caching, and that is the point.

Unlocked keyring lives in `Zeroizing<HashMap>` in app state; `keystore_lock`
drops it. No key material in logs, events, or config.toml.

## Persistence (SQLite, `~/.marvis/marvis.db`)

```sql
sessions(id INTEGER PK, type TEXT /*'ask'|'listen'*/, title TEXT,
         started_at INT, ended_at INT, last_active_at INT)
ai_messages(id INTEGER PK, session_id INT FK, role TEXT, content TEXT, ts INT)
-- Phase 2: transcripts(...), summaries(...)
```

## Error handling

- Provider HTTP/SSE errors → `ask:error{message}` + `ask:state{idle}`; UI shows
  inline error, keeps panel open.
- Locked keystore on `ask_send` → `ask:error{needs_unlock}`; bar swaps to
  unlock card.
- Screen permission missing → capture doesn't start; bar shows
  "Grant screen recording" card → `permissions_request_screen` /
  `permissions_open_prefs`.
- Ollama unreachable → `model_validate`/`ask` surface connection error (no key
  needed).
- All fallible commands return `Result<T, String>`-style serializable errors;
  no panics across FFI/FFI-boundary threads.

## Testing

- `cargo test`: ring buffer (caps, eviction, change-detection), keystore
  round-trip + wrong-DEK + tamper detection + obsolete-format handling
  (via `StaticDekProvider` — Keychain/LA paths are manual-verify only),
  config load/save, prompt builder, per-provider SSE chunk parsing
  (recorded fixtures), layout math (stack, snap, clamp).
- Manual: `bun run build:dev` — unlock, set key, ask w/ frame, hotkeys,
  click-through, multi-window animation, `marvis://ask?text=hi`.
- vitest for views in a later pass (harness not yet present).

## Open risks / decisions

1. **ScreenCaptureKit binding**: evaluate `screencapturekit` vs `cidre` crates
   at implementation; fallback = tiny Swift helper emitting JPEG to stdout
   (Glass's `SystemAudioDump` pattern) — still in-memory, no disk.
2. **Animated window moves**: Tauri has no bounds animation; movement.rs
   interpolates `set_position`/`set_size` on a 60 Hz task — validate perf on
   Retina.
3. **Unlock UX**: one system-auth prompt (Touch ID / Face ID / system
   password) per `keystore_unlock` — the Keychain DEK's `userPresence` ACL
   demands presence on every read, by design. Unsigned `tauri dev` binaries
   can't enforce the ACL, so they fall back to an in-process
   `LAContext.evaluatePolicy` gate over a plain keychain item (same UX,
   weaker enforcement, dev only — logged once at detection).
4. **Markdown rendering**: `react-markdown` + `remark-gfm` in `apps/native`
   (no markdown lib exists in the repo today); streamed chunks append to a
   single markdown document rendered incrementally.
5. **Liquid-glass private API**: `NSGlassEffectView` is undocumented —
   could break on future macOS (plugin no-ops/falls back, acceptable) and
   would block App Store distribution (we notarize/distribute outside the
   App Store anyway, like Glass).

## Out of scope for Phase 1

Audio capture/STT, live summary, preset editor, shortcut editor UI, Ollama
install/model management, response history navigation, auto-update,
non-macOS platforms, Firebase/web stack.
