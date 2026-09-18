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
| API-key management (AES-GCM, model selection, validation) | **1** | `keys.enc` + Argon2id passphrase instead of keychain (arch rule 1/4). |
| LLM providers: OpenAI, Anthropic, Gemini, Ollama | **1** | Rust `Provider` trait + SSE adapters. `openai-glass`/Portkey **dropped** (hosted service). |
| Global shortcuts | **1** | `tauri-plugin-global-shortcut`; keybinds in `config.toml`; editor UI in Phase 3. |
| macOS permissions (screen recording, mic, keychain gate) | **1** | Screen-recording + mic checks; keychain gate not applicable (keys.enc instead). |
| Session/message persistence (SQLite) | **1** | rusqlite; `sessions`, `ai_messages`, `provider_keys` (encrypted blobs live in `keys.enc`, not DB). |
| Deep links | **1** | `marvis://` scheme (replaces `pickleglass://`). Route: focus bar; `marvis://ask?text=` prefills Ask. No auth callbacks (no Firebase). |
| Listen (mic + system audio STT, debounced turns, WS session renewal) | 2 | Mic via cpal; system audio via ScreenCaptureKit audio or `SystemAudioDump`-style helper — decided in Phase 2 spec. |
| Live summary (every 5 turns → structured analysis) | 2 | Reuses Phase 1 LLM adapters. |
| Presets/prompt templates UI | 3 | Templates ship built-in from Phase 1 (Ask prompt); editor in Phase 3. |
| Local AI (Ollama service mgmt, whisper.cpp download) | 3 | Ollama *provider* works in Phase 1 (HTTP API); install/model-pull UX in Phase 3. |
| Settings panel UI (model select, key mgmt, shortcuts editor) | 1 minimal / 3 full | Phase 1: provider keys + model select + unlock. |
| Firebase auth + sync, embedded `pickleglass_web`, auto-updater, Windows/Linux | **dropped** | Conflicts with privacy-first local-only architecture. |

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
│   └── mod.rs        # keys.enc load/save, Argon2id derive, AES-256-GCM, unlock state
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
2. `Unset` (no `keys.enc`): UI prompts new passphrase → `keystore_init(passphrase)`
   → Argon2id(salt) → encrypt empty keyring → `keys.enc` (`salt|nonce|ct`, os
   0600). `Locked`: `keystore_unlock(passphrase)` → derive → decrypt → keyring in
   Rust memory (`Zeroizing`).
3. React only ever sees masked status: `openai: set ••••1234 | unset`.
4. `keystore_set_key(provider, key)` validates (test call) then re-encrypts file.

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
- Click-through toggle (`Cmd+M`): `set_ignore_cursor_events` on all windows.
- `Cmd+Arrow` step-move bar (children follow); `Cmd+Shift+Arrow` snap to
  display edge; display-remove → re-clamp to primary.
- Header-state gate (Glass parity): panels only exist once keystore is
  unlocked AND screen permission granted; otherwise bar renders the
  unlock/permission card and only `Cmd+\` stays registered.

### Hotkeys (defaults, configurable in config.toml)

| Action | macOS |
| --- | --- |
| toggleVisibility | `Cmd+\` |
| nextStep (Ask toggle/send screen-only) | `Cmd+Enter` |
| moveUp/Down/Left/Right | `Cmd+Arrows` |
| toggleClickThrough | `Cmd+M` |
| scrollUp/Down (ask panel) | `Cmd+Shift+Up/Down` |
| snap to edge L/R | `Cmd+Shift+Left/Right` (hardcoded, like Glass) |
| previous/next response | `Cmd+[` / `Cmd+]` (Phase 3 response history) |

### Deep links

- Register `marvis://` as default protocol client (Info.plist `CFBundleURLTypes`
  - `tauri-plugin-deep-link`).
- `marvis://ask?text=...` → focus bar, open ask panel, prefill/send.
- `marvis://<anything>` → focus bar. No auth handling (no Firebase).

## Command/event surface (Tauri)

Commands (webview → Rust):
`keystore_status`, `keystore_init`, `keystore_unlock`, `keystore_lock`,
`keystore_set_key`, `keystore_remove_key`, `model_validate_key`,
`model_get_selected`, `model_set_selected`, `model_list_available`,
`ask_send`, `ask_close`, `listen_stub` (Phase 2 placeholder),
`window_toggle_all`, `window_show_settings`, `window_hide_settings`,
`permissions_status`, `permissions_request_screen`, `permissions_open_prefs`,
`capture_status`, `session_list`, `session_get`, `session_delete`,
`config_get`, `config_set`.

Events (Rust → webview):
`keystore:changed`, `ask:state`, `ask:chunk`, `ask:done`, `ask:error`,
`capture:permission-needed`, `windows:visibility`, `shortcuts:updated`,
`deeplink:ask`.

Secrets and image bytes never appear in commands or events.

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

## keys.enc format (per rule 1)

```text
[16B salt][12B nonce][AES-256-GCM ciphertext || 16B GCM tag appended]
key = Argon2id(passphrase, salt, t=3, m=64MB, p=4)
plaintext = JSON: { "openai": "sk-...", "anthropic": "...", "gemini": "...", "deepgram": "..." }
```

Unlocked keyring lives in `Zeroizing<HashMap>` in app state; `keystore_lock`
drops it. No key material in logs, events, or config.toml.

## Persistence (SQLite, `marvis.db`)

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
  round-trip + wrong-passphrase + tamper detection, config load/save,
  prompt builder, per-provider SSE chunk parsing (recorded fixtures),
  layout math (stack, snap, clamp).
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
3. **Passphrase UX**: required each launch (nothing in keychain — deliberate).
   If that annoys, Phase 3 can add an optional keychain wrap of the passphrase
   — flag for later, not Phase 1.
4. **Markdown rendering**: `react-markdown` + `remark-gfm` in `apps/native`
   (no markdown lib exists in the repo today); streamed chunks append to a
   single markdown document rendered incrementally.

## Out of scope for Phase 1

Audio capture/STT, live summary, preset editor, shortcut editor UI, Ollama
install/model management, response history navigation, auto-update,
non-macOS platforms, Firebase/web stack.
