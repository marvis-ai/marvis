# Listen Phase 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add full Listen mode to the Marvis native app: dual-channel microphone/system-audio capture, selectable Deepgram or user-installed whisper-cli transcription, persisted me/them turns, live summaries, and transcript context for Ask.

**Architecture:** Rust owns audio capture, STT sessions, turn assembly, persistence, summaries, and all provider keys. The React webview receives only serialized transcript/status/summary events and invokes typed commands. Mic input uses cpal; system audio uses a second ScreenCaptureKit audio stream; whisper-cli is detected and invoked with short 0600 temporary WAV files because the selected sidecar approach is not in-process.

**Tech Stack:** Rust/Tauri 2, cpal, tokio/tokio-tungstenite, ScreenCaptureKit v10, rusqlite, reqwest/serde, React 19/Vite, TypeScript, Vitest where available.

## Global Constraints

- Use `keys.json`, not `keys.enc`; retrieve Deepgram credentials only through `Keystore::key("deepgram")`.
- API keys, PCM data, audio buffers, and screen frames never enter the webview or logs.
- Screen/audio capture and STT orchestration remain in `apps/native/src-tauri/src/`.
- macOS is the only supported platform; do not leak macOS types above the audio source abstraction.
- The app never downloads whisper.cpp binaries or models; users install `whisper-cli` and provide `ggml-*.bin` files.
- Whisper temporary WAVs are 0600 and deleted immediately after each CLI invocation.
- Use the existing provider failover chain for live summaries; summary failures never block transcription.
- Preserve the existing unified-card behavior: collapsing the card does not stop an active Listen session.
- Do not remove unrelated dead code or refactor adjacent modules.
- Every task ends with focused tests and a small imperative commit.

---

## File Map

### Rust files to create

- `apps/native/src-tauri/src/audio/mod.rs` — platform-neutral `PcmChunk`, `AudioSource`, and resampling helpers.
- `apps/native/src-tauri/src/audio/mic.rs` — cpal microphone source.
- `apps/native/src-tauri/src/audio/system.rs` — ScreenCaptureKit system-audio source.
- `apps/native/src-tauri/src/stt/mod.rs` — `SttProvider`, transcript event types, factory, and provider errors.
- `apps/native/src-tauri/src/stt/deepgram.rs` — Deepgram WebSocket session.
- `apps/native/src-tauri/src/stt/whisper.rs` — whisper-cli discovery, model discovery, WAV generation, and chunk runner.
- `apps/native/src-tauri/src/listen.rs` — `ListenService`, state, channel turn assembly, summary scheduling, and event emission.

### Rust files to modify

- `apps/native/src-tauri/Cargo.toml` — audio/WebSocket dependencies.
- `apps/native/src-tauri/src/lib.rs` — module registration, `AppState`, commands, command handler list, and Listen lifecycle.
- `apps/native/src-tauri/src/storage.rs` — transcript/summary schema and CRUD.
- `apps/native/src-tauri/src/paths.rs` — whisper model/binary/temp paths.
- `apps/native/src-tauri/src/config.rs` — validate writable STT settings.
- `apps/native/src-tauri/src/ask.rs` — inject active Listen transcript history into the system prompt.
- `apps/native/src-tauri/src/prompts.rs` — summary prompt and transcript formatting helper if needed.
- `apps/native/src-tauri/src/capture/macos.rs` — expose a reusable content filter/config constructor or shared system-audio setup without changing frame semantics.

### Frontend files to modify

- `apps/native/src/lib/commands.ts` — Listen and whisper status types/wrappers.
- `apps/native/src/lib/events.ts` — Listen event constants.
- `apps/native/src/components/ListenSection.tsx` — replace static stub with live transcript/insights UI.
- `apps/native/src/views/Bar.tsx` — mic start/stop behavior and active state.
- `apps/native/src/components/prefs/ProvidersTab.tsx` — replace Phase-2 placeholder with STT controls.
- `apps/native/src/lib/classes.ts` — only if an existing style token cannot express the new UI.
- `apps/native/src/index.css` — only wire existing waveform/iris animation classes; do not add idle animation.

---

## Task 1: Add persistence primitives for transcripts and summaries

**Files:**

- Modify: `apps/native/src-tauri/src/storage.rs`
- Test: `apps/native/src-tauri/src/storage.rs` test module

**Interfaces:**

- Produces `Transcript` and `Summary` serializable row types.
- Produces `Db::transcript_add`, `Db::transcripts_for`, `Db::summary_add`, and `Db::summary_latest`.
- Later Listen code consumes `session_get_or_create_active("listen")` and these methods.

- [ ] **Step 1: Write failing schema and round-trip tests.** Add tests for transcript ordering, summary JSON fields, and cascade deletion of a listen session. The assertions should verify that `speaker` is retained as `me`/`them`, summaries deserialize their bullet/follow-up arrays, and deleting the parent session removes both child record types.

- [ ] **Step 2: Run the focused Rust tests and verify failure.**

```bash
cd apps/native/src-tauri
cargo test storage::tests::transcript -- --nocapture
cargo test storage::tests::summary -- --nocapture
```

Expected: FAIL because the schema, row types, and methods do not exist.

- [ ] **Step 3: Extend `SCHEMA` additively.** Add `transcripts` and `summaries` with foreign keys to `sessions(id) ON DELETE CASCADE`, `speaker`, `text`, `tldr`, JSON `bullets`, JSON `follow_ups`, optional `topic`, and Unix-second timestamps.

- [ ] **Step 4: Implement typed CRUD.** Use the existing `Mutex<Connection>` convention. `transcripts_for` must return newest/oldest consistently; expose oldest-first rows to match existing `ai_messages_for`. Apply an optional limit in SQL. Store arrays as JSON strings and return typed `Vec<String>` after deserialization; malformed stored JSON must return an error.

- [ ] **Step 5: Run all storage tests.**

```bash
cargo test storage -- --nocapture
```

Expected: PASS, including all pre-existing session/message tests.

- [ ] **Step 6: Commit.**

```bash
git add apps/native/src-tauri/src/storage.rs
git commit -m "feat(native): persist listen transcripts and summaries"
```

---

## Task 2: Add paths and validated STT configuration

**Files:**

- Modify: `apps/native/src-tauri/src/paths.rs`
- Modify: `apps/native/src-tauri/src/config.rs`
- Modify: `apps/native/src-tauri/src/lib.rs` (`config_set` validation only)
- Test: existing `paths.rs` and `config.rs` test modules

**Interfaces:**

- Produces `paths::whisper_dir`, `paths::whisper_bin_dir`, `paths::whisper_models_dir`, and `paths::audio_tmp_dir`.
- `Config.models.stt_provider` accepts only `deepgram` or `whisper`.
- `Config.models.stt_model` accepts a trimmed non-empty model identifier.

- [ ] **Step 1: Add failing config tests.** Cover accepted values `deepgram`, `whisper`, `nova-2`, and `ggml-base.bin`; reject unknown providers and blank model values through `config_set`.

- [ ] **Step 2: Run the focused tests and verify failure.**

```bash
cargo test config -- --nocapture
```

Expected: FAIL for the new validation cases.

- [ ] **Step 3: Add path helpers.** Return paths under `~/.marvis/models/whisper/{bin,models}` and `~/.marvis/tmp`; helpers must not eagerly create model directories. The Listen/whisper code creates only the temp directory when needed.

- [ ] **Step 4: Extend `config_set`.** Add explicit match arms for `models.stt_provider` and `models.stt_model`; trim values, validate provider membership, reject empty model names, save, and emit the existing full `config:changed` payload.

- [ ] **Step 5: Run config and path tests.**

```bash
cargo test config paths -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add apps/native/src-tauri/src/paths.rs apps/native/src-tauri/src/config.rs apps/native/src-tauri/src/lib.rs
git commit -m "feat(native): configure selectable listen STT providers"
```

---

## Task 3: Implement platform-neutral PCM and microphone capture

**Files:**

- Modify: `apps/native/src-tauri/Cargo.toml`
- Create: `apps/native/src-tauri/src/audio/mod.rs`
- Create: `apps/native/src-tauri/src/audio/mic.rs`
- Modify: `apps/native/src-tauri/src/lib.rs` module declarations
- Test: `apps/native/src-tauri/src/audio/mod.rs` test module

**Interfaces:**

- `PcmChunk { samples: Vec<i16>, sample_rate: u32, channels: u16 }` is always normalized to 16,000 Hz, mono before leaving an `AudioSource`.
- `AudioSource: Send` exposes `start(Sender<PcmChunk>) -> anyhow::Result<()>`, `stop()`, and `is_running()`.
- `MicSource` implements `AudioSource` without exposing cpal types.

- [ ] **Step 1: Add only the required dependency.** Use the package manager rather than hand-editing versions when possible:

```bash
cd apps/native/src-tauri
cargo add cpal
```

If the repository's lockfile resolves a compatible released version, keep that version; do not loosen existing security settings.

- [ ] **Step 2: Write failing pure PCM tests.** Test mono conversion from interleaved stereo, float-to-i16 clamping, and resampling a short known waveform to 16 kHz. Keep tests independent of an actual microphone device.

- [ ] **Step 3: Run the focused tests and verify failure.**

```bash
cargo test audio:: -- --nocapture
```

Expected: FAIL because the module/helpers are not implemented.

- [ ] **Step 4: Implement bounded PCM conversion.** Normalize native cpal sample formats to f32, average channels, resample to 16 kHz with a deterministic helper, clamp to `i16`, and send bounded chunks. A full input queue drops newest chunks with a warning rather than blocking the audio callback.

- [ ] **Step 5: Implement `MicSource`.** Select the default input device/config, install the matching cpal callback, convert callback data, and run stream teardown on `stop`. Callback errors must send a status error through the source's internal channel; no panic or secret logging.

- [ ] **Step 6: Run pure tests plus compilation.**

```bash
cargo test audio:: -- --nocapture
cargo check
```

Expected: PASS and successful macOS compilation.

- [ ] **Step 7: Commit.**

```bash
git add apps/native/src-tauri/Cargo.toml apps/native/src-tauri/Cargo.lock apps/native/src-tauri/src/audio apps/native/src-tauri/src/lib.rs
git commit -m "feat(native): add normalized microphone PCM source"
```

---

## Task 4: Add ScreenCaptureKit system-audio source

**Files:**

- Create: `apps/native/src-tauri/src/audio/system.rs`
- Modify: `apps/native/src-tauri/src/capture/macos.rs`
- Modify: `apps/native/src-tauri/src/audio/mod.rs`
- Test: pure audio extraction/conversion helpers in `audio/system.rs`

**Interfaces:**

- `SystemAudioSource` implements the same `AudioSource` trait as `MicSource`.
- It accepts a reusable `SCContentFilter` or a constructor-owned filter internally; callers do not see ScreenCaptureKit types.
- It emits normalized 16 kHz mono `PcmChunk`s.

- [ ] **Step 1: Write failing extraction tests.** Cover interleaved and non-interleaved sample buffers, empty buffers, and conversion from the `AudioBufferList` byte layout exposed by screencapturekit v10.

- [ ] **Step 2: Inspect and use the existing capture filter.** Refactor only the minimal shared construction needed so frame capture and system audio use the same primary-display filter and own-process exclusion. Preserve existing screen-frame tests and behavior.

- [ ] **Step 3: Configure the audio stream.** Use `SCStreamConfiguration::new().with_captures_audio(true).with_excludes_current_process_audio(true)` and register `SCStreamOutputType::Audio`; do not register a screen output in this source. Extract bytes through `CMSampleBufferExt::audio_buffer_list`, convert supported PCM formats, and enqueue bounded chunks.

- [ ] **Step 4: Implement stop/join semantics.** Stop the stream before dropping it, close the channel, and join the worker so no callback uses freed state.

- [ ] **Step 5: Run focused tests and check.**

```bash
cargo test audio:: capture:: -- --nocapture
cargo check
```

Expected: PASS. Manual audio delivery is deferred to the integration task because it requires a running authorized macOS app.

- [ ] **Step 6: Commit.**

```bash
git add apps/native/src-tauri/src/audio apps/native/src-tauri/src/capture/macos.rs
git commit -m "feat(native): capture system audio through ScreenCaptureKit"
```

---

## Task 5: Define STT abstraction and implement whisper-cli detection/chunking

**Files:**

- Create: `apps/native/src-tauri/src/stt/mod.rs`
- Create: `apps/native/src-tauri/src/stt/whisper.rs`
- Modify: `apps/native/src-tauri/src/lib.rs` module declarations
- Test: `stt/mod.rs` and `stt/whisper.rs` test modules

**Interfaces:**

- `TranscriptEvent { text: String, finality: Finality }`, with `Finality::Interim | Final`.
- `SttProvider: Send` supports starting with a callback, non-blocking PCM enqueue, and stop.
- `WhisperStatus { binary: Option<String>, models: Vec<String> }` is serializable and contains paths/model names only.
- `WhisperProvider::discover()` and `WhisperProvider::status()` do not start a process or read keys.

- [ ] **Step 1: Write failing parser/discovery tests.** Test whisper output cleanup: plain text accepted; empty output, `[BLANK_AUDIO]`, and `(silence)` dropped; binary resolution order; only `ggml-*.bin` regular files are listed.

- [ ] **Step 2: Run focused tests and verify failure.**

```bash
cargo test stt::whisper -- --nocapture
```

Expected: FAIL because the provider does not exist.

- [ ] **Step 3: Implement discovery.** Search `PATH` for `whisper-cli`, then `/opt/homebrew/bin/whisper-cli`, then `paths::whisper_bin_dir()/whisper-cli`; inspect only `paths::whisper_models_dir()` for models. Do not download or install anything.

- [ ] **Step 4: Implement secure WAV writing.** Convert normalized `i16` PCM to a mono 16 kHz PCM WAV in `~/.marvis/tmp`, create the file with mode 0600, and return a cleanup guard/path. Ensure cleanup runs on success, parse error, spawn failure, and cancellation.

- [ ] **Step 5: Implement chunk runner.** Buffer roughly three seconds of PCM, skip windows below the RMS silence threshold, spawn `whisper-cli -m <model> -f <wav> --no-timestamps --output-txt`, capture stdout, normalize output, and emit final-only events. Bound child output and terminate it on stop.

- [ ] **Step 6: Run tests and inspect dependency/build state.**

```bash
cargo test stt:: -- --nocapture
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS with no whisper download behavior.

- [ ] **Step 7: Commit.**

```bash
git add apps/native/src-tauri/src/stt apps/native/src-tauri/src/lib.rs
git commit -m "feat(native): support user-installed whisper-cli STT"
```

---

## Task 6: Implement Deepgram WebSocket STT

**Files:**

- Modify: `apps/native/src-tauri/Cargo.toml`
- Create: `apps/native/src-tauri/src/stt/deepgram.rs`
- Modify: `apps/native/src-tauri/src/stt/mod.rs`
- Test: `stt/deepgram.rs` parser tests and mocked-session tests

**Interfaces:**

- `DeepgramProvider::new(key: String, model: String, channel: SpeakerChannel)`.
- It emits `Interim` for non-final alternatives and `Final` for `is_final=true` responses.
- It reconnects up to three times with backoff and renews around 20 minutes without changing channel identity.

- [ ] **Step 1: Add WebSocket dependencies using Cargo.** Use the existing Tokio runtime and a released `tokio-tungstenite` version with TLS support compatible with the app's rustls stack.

```bash
cd apps/native/src-tauri
cargo add tokio-tungstenite --features rustls-tls-native-roots
```

- [ ] **Step 2: Write failing Deepgram JSON parser tests.** Include interim, final, empty transcript, malformed JSON, and provider error payloads. Assert no API key is present in parser errors or event payloads.

- [ ] **Step 3: Run focused tests and verify failure.**

```bash
cargo test stt::deepgram -- --nocapture
```

Expected: FAIL before the parser/session exists.

- [ ] **Step 4: Implement authenticated session.** Connect to the Deepgram WSS endpoint with model, linear16, 16 kHz, mono, interim results, punctuation, and smart formatting. Send `Audio` binary frames and parse JSON text frames. Auth must come from the constructor argument obtained by the caller from `Keystore::key("deepgram")`; never persist or log it.

- [ ] **Step 5: Implement keepalive, renewal, cancellation, and bounded reconnect.** Send Deepgram keepalive control messages during idle periods; close and reopen around 20 minutes; retry transport failure at most three times with bounded exponential backoff; emit a terminal provider error after retries.

- [ ] **Step 6: Run STT tests and clippy.**

```bash
cargo test stt:: -- --nocapture
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS.

- [ ] **Step 7: Commit.**

```bash
git add apps/native/src-tauri/Cargo.toml apps/native/src-tauri/Cargo.lock apps/native/src-tauri/src/stt
git commit -m "feat(native): stream Deepgram listen transcription"
```

---

## Task 7: Implement Listen turn assembly and summary service

**Files:**

- Create: `apps/native/src-tauri/src/listen.rs`
- Modify: `apps/native/src-tauri/src/prompts.rs`
- Test: `listen.rs` test module

**Interfaces:**

- `ListenService::new()` owns lifecycle state and cancellation.
- `ListenService::start(...)`, `stop(...)`, `status()`, and `current_history()` are thread-safe.
- Pure `TurnAssembler` accepts `TranscriptEvent`s tagged with `SpeakerChannel` and emits closed turns.
- Summary events contain `tldr`, `bullets`, `follow_ups`, and `topic` only.

- [ ] **Step 1: Write failing pure turn tests.** Verify interim replacement, final closure after 1.5 seconds, channel-switch closure, empty-text dropping, and that turns 5 and 10 trigger exactly one summary boundary.

- [ ] **Step 2: Run focused tests and verify failure.**

```bash
cargo test listen::turn -- --nocapture
```

Expected: FAIL before `TurnAssembler` exists.

- [ ] **Step 3: Implement `TurnAssembler`.** Keep separate pending text/timers for `me` and `them`; interim events replace the pending visible string; finals append/close according to the silence/channel-switch rules. Emit a closed turn with speaker, trimmed text, and timestamp.

- [ ] **Step 4: Implement Listen lifecycle.** Start one audio source and one STT provider per available channel, choose the configured provider, and use `keystore.key("deepgram")` only for Deepgram. Missing mic permission reports `mic:false` and continues with system audio. Missing configured provider reports `listen:error` with `needs_setup`.

- [ ] **Step 5: Implement persistence/events.** On closed turn, insert into `transcripts`, emit `listen:turn`, and maintain an in-memory last-20 history. On stop, cancel providers, stop sources, flush pending turns, and end the active listen session.

- [ ] **Step 6: Implement summary generation.** Add a structured summary prompt matching Glass's Summary Overview, Key Topic, Extended Explanation, and Suggested Questions format. Use `provider_candidates`, call the existing `Provider::stream_chat` with a sink, parse the response into bounded arrays, persist through `summary_add`, and emit `listen:summary`. Failures log a warning and do not change transcript state.

- [ ] **Step 7: Run Listen unit tests.**

```bash
cargo test listen -- --nocapture
```

Expected: PASS, including cadence, history, and parse-failure tests.

- [ ] **Step 8: Commit.**

```bash
git add apps/native/src-tauri/src/listen.rs apps/native/src-tauri/src/prompts.rs
git commit -m "feat(native): assemble listen turns and live summaries"
```

---

## Task 8: Wire Listen into AppState, commands, events, and Ask context

**Files:**

- Modify: `apps/native/src-tauri/src/lib.rs`
- Modify: `apps/native/src-tauri/src/ask.rs`
- Modify: `apps/native/src-tauri/src/storage.rs` only if a query helper is still required
- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/lib/events.ts`
- Test: Rust command/config tests and TypeScript typecheck

**Interfaces:**

- Commands: `listen_start`, `listen_stop`, `listen_status`, `whisper_status`; remove `listen_stub` from registration.
- Events: `listen:state`, `listen:turn`, `listen:summary`, `listen:error` emitted to `bar`.
- `listen_status` returns `{ state, provider, session_id, turns, mic }`.
- `whisper_status` returns `{ binary: string | null, models: string[] }`.

- [ ] **Step 1: Add failing command/config tests.** Assert `models.stt_provider` and `models.stt_model` are writable, `listen_stub` is no longer part of the command contract, and status payloads serialize with the documented field names.

- [ ] **Step 2: Add `listen:state` constants and TS payload types.** Keep event names centralized in `events.ts`, matching Rust exactly. Add typed wrappers with Tauri argument names matching Rust parameters.

- [ ] **Step 3: Add `listen: ListenService` to `AppState`.** Initialize it in production and `for_test`; ensure the service is stopped during application teardown and when the app leaves its main gate only if the user explicitly stops Listen. Do not stop it merely because the card collapses.

- [ ] **Step 4: Add commands and handler registration.** Implement gate checking, mic permission request through `spawn_blocking`, source/provider construction, and status/error serialization. Register all new commands and delete `listen_stub` from `generate_handler!`.

- [ ] **Step 5: Inject transcript context into Ask.** In `ask.rs::send_chain`, read the active listen history before `build_messages`; pass a formatted `me: ...`/`them: ...` tail to `system_prompt(history)`. Preserve the existing ask history and current image behavior. Add a pure test proving the system message contains the transcript tail and no placeholder when history exists.

- [ ] **Step 6: Run Rust and frontend checks.**

```bash
cd apps/native/src-tauri
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cd ../..
bun run check-types
```

Expected: PASS. Any existing unrelated warning must be reported, not suppressed.

- [ ] **Step 7: Commit.**

```bash
git add apps/native/src-tauri/src/lib.rs apps/native/src-tauri/src/ask.rs apps/native/src/lib/commands.ts apps/native/src/lib/events.ts
 git commit -m "feat(native): wire listen commands and ask context"
```

---

## Task 9: Replace the Listen stub UI and wire the bar toggle

**Files:**

- Modify: `apps/native/src/components/ListenSection.tsx`
- Modify: `apps/native/src/views/Bar.tsx`
- Modify: `apps/native/src/index.css` only for existing animation classes
- Test: frontend component tests if the native Vitest harness supports them; otherwise typecheck/build plus manual verification

**Interfaces:**

- ListenSection consumes `listen:*` events and `listen_status`, `session_list`, `session_get`, and `whisper_status` as needed.
- Bar mic control invokes `listen_start` when idle and `listen_stop` when listening.
- Card remains open after stop so the completed transcript remains visible.

- [ ] **Step 1: Add/extend frontend test fixtures.** Define payload fixtures for active state, interim/final turns, summary, setup error, and mic-denied system-only mode. Tests must assert interim text is replaced by the final event and summary content renders.

- [ ] **Step 2: Implement ListenSection state/resync.** On mount, call `listenStatus`; load the active listen session's persisted transcript/summary rows; subscribe to all Listen events; preserve the last transcript and latest insights during provider errors.

- [ ] **Step 3: Implement transcript and insights rendering.** Render speaker labels (`me`/`them`), dim interim text, final text, TLDR, bounded bullets, follow-up chips, provider/model chip, error row, and Stop button using existing `classes.ts` tokens and ChatSection density.

- [ ] **Step 4: Wire real waveform/iris state.** Existing waveform bars and `--animate-iris-listen` may animate only while `listen:state.state === "listening"`; do not animate the static idle/error state.

- [ ] **Step 5: Update Bar mic behavior.** Existing `listenWanted` state should be driven by Listen state: idle click starts and opens listen; active click stops; Ask loading clears listen mode; card close must not stop the Rust session unless the explicit Stop action is used.

- [ ] **Step 6: Run frontend verification.**

```bash
cd apps/native
bun run test
bun run build
bun run check-types
```

Expected: PASS. If no component harness exists, record that limitation and rely on build/typecheck plus the manual matrix in Task 11.

- [ ] **Step 7: Commit.**

```bash
git add apps/native/src/components/ListenSection.tsx apps/native/src/views/Bar.tsx apps/native/src/index.css
 git commit -m "feat(native): render live listen transcript and insights"
```

---

## Task 10: Add STT settings UI and whisper status surface

**Files:**

- Modify: `apps/native/src/components/prefs/ProvidersTab.tsx`
- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/lib/events.ts` only if status events are needed
- Test: frontend typecheck/build and any existing prefs tests

**Interfaces:**

- Providers tab reads `Config.models.stt_provider/stt_model`.
- Deepgram selection displays a model input/datalist and uses the existing `deepgram` key status from keystore UI.
- Whisper selection calls `whisper_status`, lists detected model filenames, and displays setup guidance without offering a download.

- [ ] **Step 1: Replace the Phase-2 placeholder.** Remove the disabled “Deepgram Nova-2 lands with Phase 2” card and add provider selection for `deepgram`/`whisper`.

- [ ] **Step 2: Wire validated config writes.** On provider/model changes, call `configSet('models.stt_provider', ...)` or `configSet('models.stt_model', ...)`; use the returned full `Config` through the existing `data.setConfig` pattern.

- [ ] **Step 3: Add whisper status display.** Show the discovered binary path and models; when missing, state exactly that users must install `whisper-cli` and place a `ggml-*.bin` model under `~/.marvis/models/whisper/models/`. Do not create a download button.

- [ ] **Step 4: Verify the prefs surface.**

```bash
cd apps/native
bun run check-types
bun run build
```

Expected: PASS with no plaintext API key rendered by the new UI.

- [ ] **Step 5: Commit.**

```bash
git add apps/native/src/components/prefs/ProvidersTab.tsx apps/native/src/lib/commands.ts
 git commit -m "feat(native): add listen provider settings"
```

---

## Task 11: Full verification and manual macOS matrix

**Files:**

- Modify: only files required to fix failures caused by Tasks 1–10.
- Test: repository test/build commands below.

- [ ] **Step 1: Run all Rust verification.**

```bash
cd apps/native/src-tauri
cargo fmt -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: all pass. If formatting fails, run `cargo fmt`, review the diff, and commit the formatting only when it belongs to Listen changes.

- [ ] **Step 2: Run native frontend verification.**

```bash
cd apps/native
bun run test
bun run check-types
bun run build
```

Expected: all pass.

- [ ] **Step 3: Run the relevant monorepo checks.**

```bash
cd ../..
bun run lint
bun run check-types
```

Expected: pass without modifying unrelated packages.

- [ ] **Step 4: Run the manual matrix on macOS.**

1. With no mic permission, start Deepgram Listen: verify system audio continues and `mic:false` appears.
2. Grant mic permission, start Deepgram: verify `me` and `them` channels, interim repaint, final turn persistence, and stop behavior.
3. Remove the Deepgram key: verify setup error and no key in the UI event payload/logs.
4. Select whisper with no binary/model: verify detection hint and no download attempt.
5. Install `whisper-cli` and place a `ggml-base.bin`: verify final-only whisper turns and temporary WAV cleanup.
6. Let a summary boundary occur: verify Insights updates and malformed summary output does not erase the transcript.
7. Collapse the card while listening: verify capture continues; reopen and resync transcript/status.
8. Ask a chat question during/after Listen: verify the system prompt uses the `me:`/`them:` tail.
9. Revoke screen permission while Listen is active: verify Listen remains independent of screen-frame capture.

- [ ] **Step 5: Review the final diff and status.**

```bash
git status --short
git diff develop...HEAD --stat
git diff develop...HEAD --check
```

Expected: only planned Listen files/docs changed; no secrets, temporary audio, generated artifacts, or unrelated formatting.

- [ ] **Step 6: Commit any final fixes separately.**

```bash
git add <only-fixed-files>
git commit -m "fix(native): harden listen verification findings"
```

---

## Coverage Check

- Audio capture: Tasks 3–4.
- Deepgram and whisper selection: Tasks 5–6 and 10.
- User-only whisper provisioning/detection: Tasks 2 and 5.
- Turn debounce and me/them persistence: Tasks 1 and 7.
- Live summaries every five turns: Tasks 7–8.
- Ask transcript context: Task 8.
- Commands/events/AppState lifecycle: Task 8.
- Listen UI/bar behavior: Task 9.
- STT settings UI: Task 10.
- Testing/manual verification: Task 11.

No spec requirements are intentionally unassigned.
