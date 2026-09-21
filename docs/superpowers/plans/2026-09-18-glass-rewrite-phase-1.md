# Marvis Phase 1 — Glass Rewrite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the floating Marvis bar app — multi-window overlay, `keys.enc`
keystore, ScreenCaptureKit frame ring buffer, 4 LLM adapters with SSE, and the
end-to-end Ask flow — inside the existing Tauri 2 + React 19 scaffold.

**Architecture:** All logic in `apps/native/src-tauri/src/` (Rust owns core);
React webview renders one of four views per window via `?view=` and only ever
sees commands + events. State persists under `~/.marvis/`.

**Tech Stack:** Tauri 2, tokio, reqwest (rustls), rusqlite(bundled), argon2,
aes-gcm, screencapturekit(doom-fish), tauri-plugin-global-shortcut,
tauri-plugin-deep-link, tauri-plugin-liquid-glass, image, zeroize. React 19 +
Tailwind 4 + @marvis/ui + react-markdown.

**Spec:** `docs/superpowers/specs/2026-09-18-glass-rewrite-phase-1-design.md`

## Global Constraints

- **macOS only.** Every feature may assume `target_os = "macos"`.
- **bun** for all JS package ops — never npm/yarn/pnpm.
- **API keys never enter the webview** — commands/events carry masked status only.
- **Screen frames never touch disk or JS** — in-memory ring buffer: ≤120 frames,
  ≤64 MB, changed frames only (~60 s horizon).
- **`~/.marvis/` is the only data root** (dir `0700`): `keys.enc` (0600),
  `config.toml` (0644), `marvis.db` (0600), `models/` (Phase 2+).
- **Content protection always on** — `set_content_protected(true)` on every
  window; no toggle.
- Deep-link scheme is **`marvis://`** (`marvis://ask?text=` prefills Ask).
- Toggle-visibility hotkey default is **`Cmd+/`** (user-edited spec; NOT
  `Cmd+\`).
- LLM adapters in Rust: **openai, anthropic, gemini, ollama** — no Portkey,
  no provider SDKs in JS.
- Liquid glass via **`tauri-plugin-liquid-glass`** applied **from Rust**
  (`GlassMaterialVariant::Bubbles`), needs `app.macOSPrivateApi: true` +
  `transparent: true` + capability `liquid-glass:default`.
- Markdown: `react-markdown` + `remark-gfm`.
- Small focused commits, imperative message ("Add X"), one logical change
  per commit. Run `cargo test` in `apps/native/src-tauri` before each commit
  that touches Rust.

## File Structure

New Rust modules under `apps/native/src-tauri/src/`:

| File | Responsibility |
| --- | --- |
| `paths.rs` | `~/.marvis` root + file paths + dir perms |
| `config.rs` | `Config` load/save `config.toml`, defaults |
| `keystore.rs` | `keys.enc` Argon2id+AES-256-GCM, `Keyring` unlock state, masked status |
| `storage.rs` | rusqlite: schema, `sessions`, `ai_messages` |
| `llm/mod.rs` | `Provider` trait, `ProviderKind`, `ChatMessage`, factory, validation |
| `llm/{openai,anthropic,gemini,ollama}.rs` | request builders + SSE/NDJSON chunk parsers |
| `capture/mod.rs` | `FrameSource` trait, `Frame`, `RingBuffer` |
| `capture/macos.rs` | SCStream impl → channel → worker → ring |
| `permissions.rs` | screen-recording + mic status/request, open prefs |
| `prompts.rs` | system-prompt builder (ported Glass template) |
| `windows/mod.rs` | `WindowPool`, visibility orchestration, cmd handlers |
| `windows/layout.rs` | pure layout math: stack, snap, clamp, resize |
| `windows/movement.rs` | bounds animator (interpolated ticks) |
| `hotkey.rs` | global-shortcut registration + dispatch |
| `ask.rs` | Ask orchestrator: frame → messages → stream → events → persist |
| `deeplink.rs` | `marvis://` handler → focus/emit |
| `lib.rs` | AppState, plugin+command registration, startup sequence |

Webview (`apps/native/src/`): `App.tsx` (view router), `views/{Bar,AskPanel,ListenPanel,SettingsPanel}.tsx`, `lib/commands.ts`, `lib/events.ts`, `index.css` (Tailwind 4, transparent bg).

Modified: `apps/native/src-tauri/Cargo.toml`, `tauri.conf.json`,
`capabilities/default.json`, `apps/native/package.json`,
`apps/native/vite.config.ts`, `apps/native/src-tauri/src/lib.rs`.

---

### Task 1: Dependencies, plugins, and tauri.conf wiring

**Files:**

- Modify: `apps/native/src-tauri/Cargo.toml`
- Modify: `apps/native/src-tauri/tauri.conf.json`
- Modify: `apps/native/src-tauri/capabilities/default.json`
- Modify: `apps/native/package.json`
- Modify: `apps/native/vite.config.ts` (add tailwind plugin)

**Interfaces:**

- Produces: compiled deps for all later tasks; capability permissions
  `liquid-glass:default`, `core:event:default`, `deep-link:default`,
  `global-shortcut:default`.

- [ ] **Step 1: Update `apps/native/src-tauri/Cargo.toml` dependencies**

```toml
[dependencies]
tauri = { version = "2", features = ["macos-private-api"] }
tauri-plugin-opener = "2"
tauri-plugin-global-shortcut = "2"
tauri-plugin-deep-link = "2"
tauri-plugin-liquid-glass = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tokio-util = { version = "0.7", features = ["rt"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "stream"] }
futures-util = "0.3"
bytes = "1"
rusqlite = { version = "0.32", features = ["bundled"] }
argon2 = "0.5"
aes-gcm = "0.10"
rand = "0.9"
zeroize = { version = "1", features = ["derive"] }
base64 = "0.22"
toml = "0.8"
dirs = "6"
image = { version = "0.25", default-features = false, features = ["jpeg"] }
screencapturekit = "10"
thiserror = "2"
anyhow = "1"
parking_lot = "0.12"
log = "0.4"
env_logger = "0.11"
```

- [ ] **Step 2: Update `tauri.conf.json`** — single placeholder window (real
  windows are built in Rust), private API + identifier + deep-link scheme:

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Marvis",
  "version": "0.1.0",
  "identifier": "com.getmarvis.marvis",
  "build": {
    "beforeDevCommand": "bun run dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "bun run build",
    "frontendDist": "../dist"
  },
  "app": {
    "macOSPrivateApi": true,
    "windows": [],
    "security": { "csp": null }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [],
    "macOS": {
      "entitlements": null,
      "infoPlist": {
        "CFBundleURLTypes": [
          {
            "CFBundleURLName": "Marvis deep link",
            "CFBundleURLSchemes": ["marvis"]
          }
        ],
        "NSScreenCaptureUsageDescription": "Marvis captures your screen to ground answers.",
        "NSMicrophoneUsageDescription": "Marvis transcribes your microphone for meetings."
      }
    }
  }
}
```

  Note: `windows: []` means Tauri creates no windows at startup — `windows::create_all`
  (Task 10) builds them at runtime. If the schema rejects an empty windows
  array, keep one hidden window and close it in `setup`.

- [ ] **Step 3: Update `capabilities/default.json`**

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Default capability for all Marvis windows",
  "windows": ["*"],
  "permissions": [
    "core:default",
    "core:event:default",
    "core:window:default",
    "opener:default",
    "liquid-glass:default",
    "deep-link:default",
    "global-shortcut:default"
  ]
}
```

- [ ] **Step 4: Update `apps/native/package.json`** — add deps:

```bash
cd apps/native && bun add @marvis/ui@'*' tailwindcss @tailwindcss/vite react-markdown remark-gfm
```

- [ ] **Step 5: Update `vite.config.ts`** — add tailwind:

```ts
import tailwindcss from "@tailwindcss/vite";
// inside plugins: [react(), tailwindcss()]
```

- [ ] **Step 6: Create `apps/native/src/index.css`** with Tailwind 4 import +
  transparent body (glass shows through):

```css
@import "tailwindcss";

html, body, #root {
  background: transparent;
  height: 100%;
  overflow: hidden;
}
```

  Import it from `main.tsx` (replace `App.css` import).

- [ ] **Step 7: Verify build**

Run: `cd apps/native/src-tauri && cargo check`
Expected: compiles (lib.rs still the greet stub — that's fine).

- [ ] **Step 8: Commit**

`git add -A && git commit -m "Add Phase 1 dependencies and plugin config"`

---

### Task 2: `paths` + `config` modules

**Files:**

- Create: `apps/native/src-tauri/src/paths.rs`
- Create: `apps/native/src-tauri/src/config.rs`

**Interfaces:**

- Produces:
  - `paths::root() -> PathBuf` (`~/.marvis`, mkdir 0700 on first call)
  - `paths::keys_file() / config_file() / db_file() / models_dir() -> PathBuf`
  - `Config { models: ModelPrefs, hotkeys: BTreeMap<String,String>, window: WindowPrefs }`
  - `ModelPrefs { llm_provider: String, llm_model: String, stt_provider: String, stt_model: String }`
  - `WindowPrefs { bar_x: Option<f64>, bar_y: Option<f64> }`
  - `config::load() -> Config`, `config::save(&Config)`, `config::default_hotkeys() -> BTreeMap<String,String>`

- [ ] **Step 1: Write failing test** in `config.rs` `#[cfg(test)]`:

```rust
#[test]
fn load_returns_defaults_when_file_missing() {
    let tmp = tempfile_dir();
    let cfg = Config::load_from(tmp.join("config.toml")).unwrap();
    assert_eq!(cfg.models.llm_provider, "openai");
    assert_eq!(cfg.hotkeys["toggle_visibility"], "Cmd+/");
}

#[test]
fn save_then_load_roundtrips() {
    let tmp = tempfile_dir();
    let mut cfg = Config::default();
    cfg.models.llm_provider = "anthropic".into();
    cfg.save_to(tmp.join("config.toml")).unwrap();
    let back = Config::load_from(tmp.join("config.toml")).unwrap();
    assert_eq!(back.models.llm_provider, "anthropic");
}
```

  (Use `std::env::temp_dir().join(format!("marvis-test-{}", std::process::id()))`
  helper named `tempfile_dir()` in the test module — no extra dep.)

- [ ] **Step 2: Run test** — `cargo test config` → FAIL (module missing).

- [ ] **Step 3: Implement `paths.rs`**

```rust
use std::path::PathBuf;

pub fn root() -> PathBuf {
    let dir = dirs::home_dir().expect("no home dir").join(".marvis");
    if !dir.exists() {
        std::fs::create_dir_all(&dir).expect("create ~/.marvis");
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    dir
}
pub fn keys_file() -> PathBuf { root().join("keys.enc") }
pub fn config_file() -> PathBuf { root().join("config.toml") }
pub fn db_file() -> PathBuf { root().join("marvis.db") }
pub fn models_dir() -> PathBuf { root().join("models") }
```

- [ ] **Step 4: Implement `config.rs`** — serde structs per spec `config.toml`
  shape; `load_from`/`save_to` take an explicit path (testable); `load()`/`save()`
  use `paths::config_file()`. `default_hotkeys()` returns the spec table
  (`toggle_visibility: "Cmd+/"`, `next_step: "Cmd+Enter"`, `move_up/down/left/right: "Cmd+Up/…"`,
  `toggle_click_through: "Cmd+M"`, `scroll_up/down: "Cmd+Shift+Up/…"`).

- [ ] **Step 5: Run** — `cargo test config` → PASS.

- [ ] **Step 6: Commit** — `"Add paths and config modules"`

---

### Task 3: `keystore` module (keys.enc)

**Files:**

- Create: `apps/native/src-tauri/src/keystore.rs`

**Interfaces:**

- Consumes: `paths::keys_file()`
- Produces:
  - `Keystore { state: KeystoreState }` where `KeystoreState = Locked | Unset | Unlocked(Keyring)`
  - `Keyring` = `HashMap<String, String>` wrapped so memory zeroes on drop
  - `Keystore::init(pass) / unlock(pass) / lock() / set_key(provider,key) / remove_key(provider) / masked_status() -> Vec<(String, Option<String>)> / key(provider) -> Option<String>`
  - errors: `KeystoreError::{WrongPassphrase, Corrupt, Io}`

- [ ] **Step 1: Failing tests** (in `keystore.rs` test module; injectable path
  via `Keystore::at(path)`):

```rust
#[test]
fn init_unlock_roundtrip() {
    let p = tmp_path("keys.enc");
    let mut ks = Keystore::at(p.clone());
    assert!(matches!(ks.state(), KeystoreState::Unset));
    ks.init("pw").unwrap();
    ks.set_key("openai", "sk-test-123").unwrap();
    ks.lock();
    assert!(matches!(ks.state(), KeystoreState::Locked));
    ks.unlock("pw").unwrap();
    assert_eq!(ks.key("openai").unwrap(), "sk-test-123");
}

#[test]
fn wrong_passphrase_fails() {
    let p = tmp_path("keys.enc");
    let mut ks = Keystore::at(p);
    ks.init("pw").unwrap();
    ks.lock();
    assert!(matches!(ks.unlock("nope"), Err(KeystoreError::WrongPassphrase)));
}

#[test]
fn tampered_file_fails() {
    let p = tmp_path("keys.enc");
    let mut ks = Keystore::at(p.clone());
    ks.init("pw").unwrap();
    let mut raw = std::fs::read(&p).unwrap();
    let n = raw.len(); raw[n-1] ^= 0xff;
    std::fs::write(&p, raw).unwrap();
    ks.lock();
    assert!(ks.unlock("pw").is_err());
}

#[test]
fn masked_status_never_leaks_key() {
    let p = tmp_path("keys.enc");
    let mut ks = Keystore::at(p);
    ks.init("pw").unwrap();
    ks.set_key("openai", "sk-secret").unwrap();
    let masked = ks.masked_status();
    assert_eq!(masked, vec![("openai".into(), Some("…cret".into()))]);
    assert!(!format!("{:?}", masked).contains("sk-secret"));
}
```

- [ ] **Step 2: Run** — `cargo test keystore` → FAIL.

- [ ] **Step 3: Implement** — `keys.enc` = `[16B salt][12B nonce][ct||tag]`;
  `Argon2::new(Argon2id, V0x13, Params::new(65536,3,4,Some(32)))`
  `hash_password_into`; `Aes256Gcm` encrypt of JSON map; `Zeroizing` key +
  keyring; `masked_status()` returns last-4 mask `…xxxx` (never full key);
  `set_key` rewrites file only when `Unlocked`; file perms 0600.

- [ ] **Step 4: Run** — `cargo test keystore` → PASS.

- [ ] **Step 5: Commit** — `"Add keys.enc keystore with Argon2id + AES-256-GCM"`

---

### Task 4: `storage` module (SQLite)

**Files:**

- Create: `apps/native/src-tauri/src/storage.rs`

**Interfaces:**

- Consumes: `paths::db_file()`
- Produces:
  - `Db::open() / Db::at(path) -> Db` (each method opens short-lived conn or holds `Mutex<Connection>`)
  - `session_get_or_create_active(kind: &str) -> i64`
  - `session_touch(id)`, `session_end(id)`, `session_list() -> Vec<Session>`, `session_delete(id)`
  - `ai_message_add(session_id, role, content)`, `ai_messages_for(session_id) -> Vec<AiMessage>`
  - `Session { id, kind, title, started_at, ended_at }`, `AiMessage { id, session_id, role, content, ts }`

- [ ] **Step 1: Failing tests** — `Db::at(tmp)`: `get_or_create_active` reuses
  open session, creates when ended; `ai_message_add` + `ai_messages_for`
  roundtrip; `session_delete` cascades messages.

- [ ] **Step 2: Run** — FAIL.

- [ ] **Step 3: Implement** — `PRAGMA journal_mode=WAL; foreign_keys=ON;`
  schema from spec (`sessions`, `ai_messages`, `ON DELETE CASCADE`); `Mutex<Connection>`;
  unix-epoch `ts` via `SystemTime`; file 0600.

- [ ] **Step 4: Run** — PASS.

- [ ] **Step 5: Commit** — `"Add SQLite storage for sessions and messages"`

---

### Task 5: `llm` core — Provider trait, types, factory

**Files:**

- Create: `apps/native/src-tauri/src/llm/mod.rs`

**Interfaces:**

- Produces:
  - `ProviderKind::{OpenAi, Anthropic, Gemini, Ollama}` + `from_str`/`as_str`
  - `ChatMessage { role: Role, content: Vec<ContentPart> }`, `ContentPart::Text(String) | ImageJpeg(Vec<u8>)`
  - `trait Provider: Send + Sync { async fn stream_chat(&self, msgs: &[ChatMessage], on_token: impl FnMut(&str) + Send) -> Result<String, LlmError>; async fn validate(&self) -> Result<(), LlmError>; }`
  - `LlmError::{Http(String), Auth, NoModel, MultimodalUnsupported, Network(reqwest::Error)}`; `LlmError::is_multimodal() -> bool`
  - `make_provider(kind, api_key: Option<String>, model: String) -> Box<dyn Provider>`

- [ ] **Step 1: Failing tests** — `ProviderKind::from_str("gemini")==Gemini`,
  `from_str("openai-glass")` rejects (dropped provider), `is_multimodal()` true
  for "image_url not supported"-style messages.

- [ ] **Step 2: Run** — FAIL.

- [ ] **Step 3: Implement** — trait + enum + factory dispatch to the four
  adapter modules (created next task as empty `unimplemented!`? No — create
  stubs returning `LlmError::NoModel` so factory compiles; adapters land in
  Task 6).

- [ ] **Step 4: Run** — PASS.

- [ ] **Step 5: Commit** — `"Add LLM provider trait and factory"`

---

### Task 6: Four LLM adapters + SSE parsers

**Files:**

- Create: `apps/native/src-tauri/src/llm/openai.rs`
- Create: `apps/native/src-tauri/src/llm/anthropic.rs`
- Create: `apps/native/src-tauri/src/llm/gemini.rs`
- Create: `apps/native/src-tauri/src/llm/ollama.rs`

**Interfaces:**

- Consumes: `Provider`, `ChatMessage`, `LlmError` from `llm/mod.rs`
- Produces: `OpenAiProvider { client, api_key, model }`, `AnthropicProvider`,
  `GeminiProvider`, `OllamaProvider` (api_key unused) — each `impl Provider`.
  Each exposes `parse_stream_line(line: &str) -> Option<String>` (token or none)
  for tests.

**Endpoints / shapes:**

- OpenAI: `POST https://api.openai.com/v1/chat/completions` bearer; body
  `{model, stream:true, temperature:0.7, max_tokens:2048, messages:[{role,content:[{type:"text",text}|{type:"image_url",image_url:{url:"data:image/jpeg;base64,…"}}]}]}`;
  SSE `data: {json}` → `choices[0].delta.content`; `data: [DONE]` ends.
  Validate: `GET /v1/models`.
- Anthropic: `POST https://api.anthropic.com/v1/messages` headers
  `x-api-key`, `anthropic-version: 2023-06-01`; body `{model, max_tokens:2048,
  stream:true, system:"…", messages:[{role:"user",content:[{type:"text",text}|{type:"image",source:{type:"base64",media_type:"image/jpeg",data}}]}]}`;
  SSE `data: {"type":"content_block_delta","delta":{"text":"x"}}`.
  Validate: `GET /v1/models`.
- Gemini: `POST https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent?alt=sse&key={key}`;
  body `{system_instruction:{parts:[{text}]}, contents:[{role:"user",parts:[{text},{inline_data:{mime_type:"image/jpeg",data}}]}]}`;
  SSE `data: {"candidates":[{"content":{"parts":[{"text":"x"}]}}]}`.
  Validate: `GET /v1beta/models?key=`.
- Ollama: `POST http://localhost:11434/api/chat` NDJSON (not SSE):
  `{model, stream:true, messages:[{role,content,images:[b64]}]}` → line
  `{"message":{"content":"x"},"done":false}`. Validate: `GET /api/tags`.

- [ ] **Step 1: Failing parser tests** — one fixture per provider:

```rust
#[test]
fn openai_parses_delta() {
    assert_eq!(OpenAiProvider::parse_stream_line(
        r#"data: {"choices":[{"delta":{"content":"Hel"}}]}"#),
        Some("Hel".into()));
    assert_eq!(OpenAiProvider::parse_stream_line("data: [DONE]"), None);
}
```

  Anthropic fixture: `data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi"}}`.
  Gemini fixture: `data: {"candidates":[{"content":{"parts":[{"text":"Yo"}]}}]}`.
  Ollama fixture: `{"message":{"role":"assistant","content":"Hey"},"done":false}`.
  Plus a "system message has system role" test for anthropic body build.

- [ ] **Step 2: Run** — FAIL.

- [ ] **Step 3: Implement** — shared private `stream_sse(req, parse, on_token)`
  helper in `llm/mod.rs`: send request → `bytes_stream()` → accumulate to
  `\n\n` boundaries → feed `data:` lines to `parse` → `on_token` + append
  `full`. Ollama uses same helper with NDJSON line parsing. `validate()` per
  provider. Map 401/403→`Auth`, body-parse of `error.message` containing
  image/vision keywords→`MultimodalUnsupported`.

- [ ] **Step 4: Run** — `cargo test llm` → PASS.

- [ ] **Step 5: Commit** — `"Add OpenAI/Anthropic/Gemini/Ollama adapters"`

---

### Task 7: `capture` — FrameSource trait + RingBuffer (+ macOS impl)

**Files:**

- Create: `apps/native/src-tauri/src/capture/mod.rs`
- Create: `apps/native/src-tauri/src/capture/macos.rs`

**Interfaces:**

- Produces:
  - `Frame { jpeg: Vec<u8>, width: u32, height: u32, ts: i64, hash: u64 }`
  - `RingBuffer { push(Frame), latest() -> Option<Frame>, len(), total_bytes() }` — caps: 120 frames / 64 MB
  - `trait FrameSource: Send + Sync { fn start(&self, on_frame: Box<dyn Fn(Frame) + Send>); fn stop(&self); }`
  - `MacosCapture::new() -> impl FrameSource` (SCStream)
  - `fn frame_hash(bgra: &[u8], stride: usize) -> u64` (strided FNV over every 4096th byte)

- [ ] **Step 1: Failing RingBuffer tests** — `fake_frame(hash, jpeg_len)` is a
  test helper building `Frame { jpeg: vec![0; jpeg_len], width: 4, height: 4,
  ts: 0, hash }`:

```rust
#[test]
fn evicts_oldest_past_frame_cap() {
    let mut rb = RingBuffer::new(120, 64*1024*1024);
    for i in 0..125 { rb.push(fake_frame(i, 100)); }
    assert_eq!(rb.len(), 120);
    assert_eq!(rb.latest().unwrap().hash, 124);
}
#[test]
fn evicts_oldest_past_byte_cap() {
    let mut rb = RingBuffer::new(120, 1_000);
    for i in 0..5 { rb.push(fake_frame(i, 400)); }
    assert!(rb.total_bytes() <= 1_000);
    assert_eq!(rb.len(), 2);
}
#[test]
fn identical_hash_means_changed_detection_works() {
    let a = vec![0xabu8; 1_000_000];
    assert_eq!(frame_hash(&a, 4), frame_hash(&a, 4));
    let mut b = a.clone(); b[500_000] = 0xcd;
    assert_ne!(frame_hash(&a, 4), frame_hash(&b, 4));
}
```

- [ ] **Step 2: Run** — FAIL.

- [ ] **Step 3: Implement `RingBuffer` + `frame_hash`** — `VecDeque`,
  `push` evicts front while `len>cap || bytes>byte_cap` (after push, keep at
  least 1); `hash` computed by caller.

- [ ] **Step 4: Run** — PASS.

- [ ] **Step 5: Implement `MacosCapture`** —
  `SCShareableContent::get()` → `displays()[0]` → `SCContentFilter` →
  `SCStreamConfiguration` (display native size, `PixelFormat::BGRA`,
  `shows_cursor:false`, `minimum_frame_interval` ≈ 250 ms) →
  `stream.add_output_handler(closure, Screen)` → closure extracts
  `image_buffer`, locks it, sends `(bgra_vec, w, h, bytes_per_row)` over
  `std::sync::mpsc::channel` → worker thread: `frame_hash` vs `last_hash`
  (skip if equal) → downscale to height ≤384 via `image::imageops::resize` →
  `JpegEncoder` q80 → `on_frame(Frame)`. `stop()` stops stream + joins worker.

- [ ] **Step 6: Manual verify** (no automated test — needs screen permission):
  temporary `src-tauri` example or `#[ignore]` test that prints frame count
  after 3 s; run once with `cargo test -- --ignored` and screen permission.

- [ ] **Step 7: Commit** — `"Add frame ring buffer and ScreenCaptureKit capture"`

---

### Task 8: `permissions` module

**Files:**

- Create: `apps/native/src-tauri/src/permissions.rs`

**Interfaces:**

- Produces:
  - `screen_status() -> bool` — `CGPreflightScreenCaptureAccess()`
  - `screen_request() -> bool` — `CGRequestScreenCaptureAccess()`
  - `mic_status() -> PermissionState` — `AVCaptureDevice authorizationStatus` for `AVMediaTypeAudio` (`NotDetermined|Denied|Authorized`)
  - `mic_request()` — `requestAccessForMediaType` (block on completion)
  - `open_prefs(section: &str)` — `open x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture` (or `Privacy_Microphone`)
  - `PermissionState` enum serialized for the bar

  Implement `CGPreflight…`/`CGRequest…` as `extern "C"` fns with
  `#[link(name="CoreGraphics", kind="framework")]`; mic via `objc2` +
  `objc2-av-foundation` crates (add to Cargo.toml here).

- [ ] **Step 1: Write a smoke test** — `screen_status()` returns a bool without
  panic (can't assert value in CI): `let _ = screen_status();`.

- [ ] **Step 2: Run** — FAIL (fn missing).

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** — PASS.

- [ ] **Step 5: Commit** — `"Add macOS permission checks"`

---

### Task 9: `prompts` module

**Files:**

- Create: `apps/native/src-tauri/src/prompts.rs`

**Interfaces:**

- Produces: `system_prompt(conversation_history: &str) -> String` — the Glass
  `pickle_glass_analysis` template with `{{CONVERSATION_HISTORY}}` substituted.

- [ ] **Step 1: Failing tests** — output contains `<core_identity>`,
  `<question_answering_priority>`; `system_prompt("me: hi")` embeds `me: hi`;
  empty history yields `No conversation history available.`

- [ ] **Step 2: Run** — FAIL.

- [ ] **Step 3: Implement** — copy the `pickle_glass_analysis` template
  **verbatim** from `~/Codes/OpenSource/glass/src/features/common/prompts/promptTemplates.js`
  lines 238–409 into a `const` (change "Pickle" identity strings to "Marvis");
  append `\n\nConversation so far:\n{{CONVERSATION_HISTORY}}` if the source
  lacks the placeholder; `system_prompt` substitutes.

- [ ] **Step 4: Run** — PASS.

- [ ] **Step 5: Commit** — `"Port system prompt template"`

---

### Task 10: `windows` — pool, layout, movement

**Files:**

- Create: `apps/native/src-tauri/src/windows/mod.rs`
- Create: `apps/native/src-tauri/src/windows/layout.rs`
- Create: `apps/native/src-tauri/src/windows/movement.rs`

**Interfaces:**

- Produces:
  - `Rect { x: f64, y: f64, w: f64, h: f64 }`
  - `layout::panel_rects(bar: Rect, visible: &BTreeSet<Panel>) -> BTreeMap<Panel, Rect>` — `ask` 600w centered under bar (y = bar.bottom+8); `listen` 400w left of ask if both visible else centered; `settings` 240w right-aligned under bar
  - `layout::clamp_to_work_area(Rect, work: Rect) -> Rect`
  - `layout::snap_edge(bar: Rect, dir: Dir, work: Rect) -> (f64,f64)` (12 px margin)
  - `Panel::{Ask, Listen, Settings}`; `WindowPool::create_bar_only(&AppHandle)` and
    `WindowPool::create_feature_windows(&AppHandle)` — builds `bar` at startup,
    panels lazily when the app gate opens (Task 14); all windows via
    `WebviewWindowBuilder` + `?view=` urls with spec flags; applies liquid
    glass `Bubbles` + `set_content_protected(true)`, registers in pool
  - `WindowPool::{show(panel), hide(panel), toggle_all(), set_click_through(bool), move_bar_step(Dir), snap_edge(Dir), adjust_height(name: &str, px: f64), position_bar_at_startup()}` — `name` is the window label string (`"ask"|"listen"|"settings"`), resolved to `Panel` internally so the command handler passes it through unmodified
  - `movement::animate(window, to: Rect, dur: Duration)` — 60 Hz lerp via `tauri::async_runtime` task + `AtomicBool` cancel

- [ ] **Step 1: Failing layout tests** — centered ask under bar; listen offsets
  left when both visible; snap_right lands `x = work.max_x - w - 12`; clamp
  keeps rect inside work area.

- [ ] **Step 2: Run** — FAIL.

- [ ] **Step 3: Implement `layout.rs`** (pure fns) — PASS tests.

- [ ] **Step 4: Implement `movement.rs`** — `animate` spawns task lerping
  `set_position`/`set_size` over ~180 ms ease-out (`f(t)=1-(1-t)^3`).

- [ ] **Step 5: Implement `WindowPool`** — `create_bar_only` per spec
  geometry (bar 353×47 centered, 21 px below work-area top unless
  `config.window.bar_x/y` set); `create_feature_windows` builds the three
  panels hidden; `show(panel)` = set bounds start-offset → show → fade-in
  animate + animate others to new layout; `hide` reverse; `toggle_all`
  remembers last-visible set; `adjust_height` clamps ≤900 (ask/listen) /
  ≤400 (settings) and animates bounds keeping top anchored; drag handled by
  `data-tauri-drag-region` on the bar.

- [ ] **Step 6: Manual verify** — `cargo check`; windows appear in `tauri dev`
  once lib.rs wires it (next tasks).

- [ ] **Step 7: Commit** — `"Add window pool, layout math, and movement animator"`

---

### Task 11: `hotkey` module

**Files:**

- Create: `apps/native/src-tauri/src/hotkey.rs`

**Interfaces:**

- Consumes: `config.hotkeys` map, `WindowPool`, `ask` (for `next_step`)
- Produces:
  - `register_all(app, keybinds: &BTreeMap<String,String>)` — parses each
    `"Cmd+/"`-style string → `tauri_plugin_global_shortcut::Shortcut`, registers
    via `app.global_shortcut().register`
  - `fn accelerator_for(action: &str) -> Option<Shortcut>` (mapping incl.
    `Cmd+/` → `"Cmd+Slash"`, `Cmd+[` → `"Cmd+BracketLeft"`, `Cmd+]` → `BracketRight`)
  - `register_all` also registers the two hardcoded bindings (Glass parity):
    `Cmd+Shift+<n>` → move bar to display n (via `app.available_monitors()`)
    and `Cmd+Shift+Left/Right` → `pool.snap_edge(dir)`
  - dispatch table action→closure: toggle→`pool.toggle_all()`, next_step→
    `ask::send_screen_only()`, arrows→`move_bar_step`, click-through→
    `pool.set_click_through(toggle)`, scroll→emit `ask:scroll{up|down}` to ask win,
    `Cmd+Shift+S`→`ask::send_screen_only()`
  - `register_limited()` variant used while gated (Task 14): only
    `toggle_visibility` + edge/display shortcuts until app reaches `main`
    state; call `register_all` once the gate opens (Glass
    `reregister-shortcuts` parity)

- [ ] **Step 1: Failing test** — `accelerator_for("toggle_visibility")` parses
  `Cmd+/` without error; unknown action → `None`; `Cmd+[` maps to BracketLeft.

- [ ] **Step 2: Run** — FAIL.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** — PASS.

- [ ] **Step 5: Commit** — `"Register global hotkeys from config"`

---

### Task 12: `ask` orchestrator

**Files:**

- Create: `apps/native/src-tauri/src/ask.rs`

**Interfaces:**

- Consumes: `RingBuffer::latest()`, `keystore.key(provider)`, `make_provider`,
  `prompts::system_prompt`, `Db`, `WindowPool::show(Ask)`
- Produces:
  - `AskService { state: AskState, cancel: CancellationToken }` in app state
  - `AskState::{Idle, Loading, Streaming}` + `current_question`, `current_response`
  - `AskService::send(app, text)` — emits `ask:state`, gets frame (may be
    `None` → text-only), builds messages, streams → `app.emit_to("ask",
    "ask:chunk", token)` per token → `ask:done{full}`; persists user+assistant
    rows; on `MultimodalUnsupported` retry text-only once
  - `AskService::send_screen_only(app)` — same with `text = ""` prompt
    "Describe what is on my screen and how you can help."
  - `AskService::close(app)` — cancel token, reset state, hide panel
  - events: `ask:state{loading|streaming|idle}`, `ask:chunk{text}`, `ask:done`,
    `ask:error{message, needs_unlock?}`

- [ ] **Step 1: Failing tests** — inject `MockProvider` (yields canned tokens,
  or `MultimodalUnsupported` on first call): streaming emits ordered chunks +
  persists both messages; fallback retried text-only; abort stops stream.

  (Make `send` generic over `&dyn Provider` internally so tests don't need
  keystore — `send_with(provider, db, emitter, text)`.)

- [ ] **Step 2: Run** — FAIL.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** — PASS.

- [ ] **Step 5: Commit** — `"Add ask orchestration with streaming and fallback"`

---

### Task 13: `deeplink` module

**Files:**

- Create: `apps/native/src-tauri/src/deeplink.rs`

**Interfaces:**

- Produces: `deeplink::init(app)` — `app.deep_link().on_open_url(|event| …)`;
  parse `marvis://ask?text=…` → `ask::send(text)` (after unlock gate); any other
  `marvis://*` → focus bar. Non-macOS-safe to keep (macOS-only anyway).

- [ ] **Step 1: Write parse test** — `route("marvis://ask?text=hello%20world")`
  → `Action::Ask("hello world")`; `route("marvis://focus")` → `Action::Focus`;
  non-`marvis` scheme → `Action::Ignore`.

- [ ] **Step 2: Run** — FAIL.

- [ ] **Step 3: Implement** — pure `route(url: &str) -> Action` + `init` wiring.

- [ ] **Step 4: Run** — PASS.

- [ ] **Step 5: Commit** — `"Add marvis:// deep-link routing"`

---

### Task 14: `lib.rs` — AppState, commands, startup

**Files:**

- Modify: `apps/native/src-tauri/src/lib.rs` (replace greet stub)

**Interfaces:**

- Produces: `AppState { keystore: Mutex<Keystore>, config: Mutex<Config>,
  db: Db, ring: RingBuffer (Arc<Mutex>), capture: Option<MacosCapture>,
  ask: Mutex<AskService>, pool: WindowPool, click_through: AtomicBool }` and
  all commands from the spec surface:

```rust
keystore_status, keystore_init, keystore_unlock, keystore_lock,
keystore_set_key, keystore_remove_key,            // set_key → validate via provider first
model_validate_key, model_get_selected, model_set_selected, model_list_available,
ask_send, ask_close,
window_toggle_all, window_show_settings, window_hide_settings, window_adjust_height,
permissions_status, permissions_request_screen, permissions_request_mic, permissions_open_prefs,
capture_status,
session_list, session_get, session_delete,
config_get, config_set,
quit_application
```

- `model_list_available` returns static per-provider model lists
  (openai: `gpt-4o, gpt-4o-mini, o4-mini`; anthropic: `claude-sonnet-4-5,
  claude-opus-4-1`; gemini: `gemini-2.5-pro, gemini-2.5-flash`; ollama: `GET
  /api/tags` dynamic).
- `keystore_status` returns `{ state: "Unset"|"Locked"|"Unlocked", keys:
  keystore.masked_status() }` — the masked shape Bar/SettingsPanel render.
- `keystore_set_key` flow: `model_validate_key(provider,key)` first →
  `set_key` → emit `keystore:changed`.
- **App gate (Glass `handleHeaderStateChanged` parity):**
  `fn app_gate(state: &AppState) -> Gate` where
  `Gate::{NeedsUnlock, NeedsPermission, Main}` =
  `Unset|Locked → NeedsUnlock`, `Unlocked && !screen_status() →
  NeedsPermission`, else `Main`.
  - `transition_gate(app)`: recompute; on entering `Main` for the first
    time → `pool.create_feature_windows()` + `hotkey::register_all` +
    `capture.start()`; emits `app:state{gate}` either way.
  - Called after `keystore_init`, `keystore_unlock`, and
    `permissions_request_screen` commands.
- Startup `setup`: `paths::root()` → `Db::open` → `config::load` →
  `pool.create_bar_only()` → `deeplink::init` → `hotkey::register_limited` →
  `transition_gate(app)` (opens panels + starts capture if already `Main`,
  e.g. re-run after permission granted) → `app.emit("app:state", gate)`.

- [ ] **Step 1: Compile check + command smoke test** — `#[tauri::command]`
  fns registered in `generate_handler!`; a test that `config_get` returns
  defaults against a temp `AppState` (construct `AppState::for_test(tmp)`).

- [ ] **Step 2: Run** — `cargo check` then `cargo test` → green.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Commit** — `"Wire AppState, commands, and startup sequence"`

---

### Task 15: Webview scaffold — view router, commands.ts, Bar

**Files:**

- Modify: `apps/native/src/App.tsx`, `apps/native/src/main.tsx`
- Create: `apps/native/src/lib/commands.ts`, `apps/native/src/lib/events.ts`
- Create: `apps/native/src/views/Bar.tsx`
- Create: `apps/native/src/components/*` (use `@marvis/ui` primitives)

**Interfaces:**

- Consumes: every command/event name from Task 14.
- `App.tsx`: `const view = new URLSearchParams(location.search).get("view")`
  → `<Bar/> | <AskPanel/> | <ListenPanel/> | <SettingsPanel/>`.

- [ ] **Step 1: `lib/commands.ts`** — typed wrappers:

```ts
import { invoke } from "@tauri-apps/api/core";
export const keystoreStatus = () => invoke<KeystoreStatus>("keystore_status");
export const keystoreUnlock = (pass: string) => invoke("keystore_unlock", { pass });
export const askSend = (text: string) => invoke("ask_send", { text });
// …one per command; define KeystoreStatus/ModelPrefs/Session/PermissionState types
```

- [ ] **Step 2: `lib/events.ts`** — `useTauriEvent<T>(name, cb)` hook wrapping
  `@tauri-apps/api/event.listen` with cleanup.

- [ ] **Step 3: `Bar.tsx`** — three states driven by `app:state` +
  `keystore:changed` + `permissions_status`:
  - `Unset|Locked` → passphrase card (input + Unlock/Set button, error on
    wrong passphrase)
  - screen permission false → "Grant screen recording" card →
    `permissions_request_screen` then `permissions_open_prefs`
  - else main bar (fixed 353×47): drag region (`data-tauri-drag-region`),
    logo mark, text input (Enter → `askSend`), Listen toggle button
    (`disabled`, "soon"), settings gear → `window_show_settings`
- [ ] **Step 4: Manual verify** — `bun run build:dev`: bar renders centered
  under menu bar, drag works, unlock card → main transition works.

- [ ] **Step 5: Commit** — `"Add webview scaffold and Bar view"`

---

### Task 16: `AskPanel` view

**Files:**

- Create: `apps/native/src/views/AskPanel.tsx`

**Interfaces:**

- Consumes: `ask:state`, `ask:chunk`, `ask:done`, `ask:error`, `ask:scroll` events;
  `ask_close`, `window_adjust_height` commands.

- [ ] **Step 1: Implement** — on mount `document.body.classList.add("ask")`;
  listens `ask:state{current_question}` (render q), accumulate `ask:chunk`
  into `response` string → `<ReactMarkdown remarkPlugins={[remarkGfm]}>`;
  spinner while `loading`; error banner on `ask:error` (+ "unlock needed"
  button when `needs_unlock`); `ask:scroll{dir}` → scroll container;
  close ✕ → `ask_close`. ResizeObserver on content root → throttled
  `invoke("window_adjust_height", { name:"ask", height })` (cap 900).
- [ ] **Step 2: Manual verify** — ask streams into a growing panel.

- [ ] **Step 3: Commit** — `"Add AskPanel streaming view"`

---

### Task 17: `SettingsPanel` + `ListenPanel` stub

**Files:**

- Create: `apps/native/src/views/SettingsPanel.tsx`
- Create: `apps/native/src/views/ListenPanel.tsx`

**Interfaces:**

- `SettingsPanel`: provider rows (openai/anthropic/gemini/ollama +
  deepgram-disabled-phase2) — masked status from `keystore_status`, input +
  Validate&Save (`keystore_set_key`), Remove; LLM model dropdown
  (`model_list_available` → `model_set_selected`); "Lock keys" button →
  `keystore_lock`; hide on blur → `window_hide_settings`.
- `ListenPanel`: stub text "Listen arrives in Phase 2".

- [ ] **Step 1: Implement.**

- [ ] **Step 2: Manual verify** — set/remove key updates masked status;
  model select persists to `config.toml`.

- [ ] **Step 3: Commit** — `"Add settings panel and listen stub"`

---

### Task 18: End-to-end manual pass

**Files:** none (verification + fix commits as needed)

- [ ] **Step 1:** `bun run build:dev` → bar appears, content-protected
  (screenshot the bar region → not captured).
- [ ] **Step 2:** Unlock card → set passphrase → main bar.
- [ ] **Step 3:** Settings → save an OpenAI key → masked status shows.
- [ ] **Step 4:** Ask a question → panel animates open, streams markdown,
  grows with content, `Cmd+/` hides all, `Cmd+/` restores.
- [ ] **Step 5:** `Cmd+Arrows` move; `Cmd+Shift+Left` snaps left;
  `Cmd+M` click-through; `Cmd+Shift+S` screen-only ask.
- [ ] **Step 6:** `open "marvis://ask?text=hello"` → bar focuses + ask fires.
- [ ] **Step 7:** `sqlite3 ~/.marvis/marvis.db "select * from ai_messages"`
  shows persisted exchange; `keys.enc` is binary, `config.toml` has model.
- [ ] **Step 8:** `cargo test` green; fix whatever breaks, commit fixes.

---

## Execution notes

- Order is strict: 1→18. Tasks 2–9 are pure-logic and fully unit-tested;
  10–14 wire into Tauri; 15–17 are views; 18 is manual.
- The `screencapturekit` closure API is the one spec risk — if
  `image_buffer()` access differs in v10, adapt `MacosCapture` to the crate's
  documented `IOSurface`/`CVPixelBuffer` accessors; the `Frame`/`RingBuffer`
  contract is unaffected.
