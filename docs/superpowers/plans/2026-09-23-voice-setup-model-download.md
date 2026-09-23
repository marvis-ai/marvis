# Optional Voice Setup and Whisper Model Downloads Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add optional Deepgram/Whisper voice configuration to onboarding and Settings, with Rust-owned downloads of curated Whisper.cpp models from Hugging Face.

**Architecture:** Rust owns the fixed Hugging Face catalog, HTTPS download, progress/cancellation, checksum verification, atomic installation, model removal, and all filesystem/network access. React uses one shared `VoiceSetup` component in onboarding and Settings; it receives masked key/model/download status and invokes typed commands. Onboarding may cancel an in-progress download when the Voice step is abandoned, while Settings downloads continue independently of the Settings window.

**Tech Stack:** Rust/Tauri 2, existing Tokio/reqwest/rusqlite/serde stack, SHA-1 verification against the official whisper.cpp model catalog, React 19/Vite, TypeScript, Tailwind 4, existing `@marvis/ui` components.

## Global Constraints

- Voice is optional; users can finish onboarding with `Skip for now`.
- Model files only are downloaded; `whisper-cli` remains user-installed and detected by the existing resolution order.
- Only these model IDs are supported: `tiny`, `base`, and `small`.
- Only fixed HTTPS URLs under `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/` are accepted.
- API keys remain in `keys.json` through existing keystore commands; they never enter `config.toml`, React events, logs, or downloaded files.
- The webview never receives PCM, transcripts from the downloader, audio bytes, or arbitrary download URLs.
- Temporary model files are mode `0600`, cleaned on every failed/cancelled path, and atomically renamed only after verification.
- Existing final model files are not damaged by failed downloads or cancellation.
- Only one model download may run at a time.
- Downloaded models are not automatically selected; selection writes `models.stt_model` explicitly.
- Use `keys.json`, not `keys.enc`.
- Preserve current Listen behavior: missing voice setup is recoverable through Settings, and card collapse does not stop an active session.
- Do not download arbitrary models, `.en` variants, quantized variants, medium/large models, or the Whisper binary.
- Follow the existing Rust/Tauri and prefs UI conventions; do not add a frontend HTTP client for model downloads.

---

## File Map

### Rust files to create

- `apps/native/src-tauri/src/voice_models.rs` — fixed catalog, model IDs, upstream filenames/URLs/digests, catalog/status DTOs, and download manager.

### Rust files to modify

- `apps/native/src-tauri/Cargo.toml` — add the smallest released checksum dependency needed for the published upstream SHA-1 values, if no existing crate provides it.
- `apps/native/src-tauri/src/lib.rs` — register voice model commands/events, own the download manager in `AppState`, and expose status/catalog/removal commands.
- `apps/native/src-tauri/src/paths.rs` — add a model temp path helper only if the existing Whisper paths cannot safely provide one.
- `apps/native/src-tauri/src/config.rs` — validate that Whisper `models.stt_model` is a supported catalog ID/filename when the provider is `whisper`.
- `apps/native/src-tauri/src/listen.rs` — preserve setup guidance when selected model is missing; do not start Whisper with an unavailable model.
- `apps/native/src-tauri/src/stt/whisper.rs` — consume the catalog/status model resolution instead of independently accepting arbitrary model filenames.

### Frontend files to create

- `apps/native/src/components/prefs/VoiceSetup.tsx` — shared provider/key/model/download UI used by Settings and onboarding.

### Frontend files to modify

- `apps/native/src/components/prefs/ProvidersTab.tsx` — replace inline Voice/STT UI with `VoiceSetup`.
- `apps/native/src/components/prefs/Onboarding.tsx` — add optional Voice step and use `VoiceSetup`.
- `apps/native/src/lib/commands.ts` — catalog/download/cancel/remove/status types and wrappers.
- `apps/native/src/lib/events.ts` — download progress/error event constants/types.
- `apps/native/src/components/ListenSection.tsx` — use the selected model/status consistently if the expanded status payload changes.
- `apps/native/src/lib/classes.ts` — only add shared styles when existing tokens cannot express the cards/progress controls.

### Documentation/tests

- Rust unit tests live beside `voice_models.rs`, `lib.rs`, `config.rs`, and existing Whisper tests.
- Frontend has no component test harness today; use pure helper tests only if the existing setup supports them, otherwise typecheck/build plus manual verification.

---

## Task 1: Define the fixed Hugging Face catalog and safe model identifiers

**Files:**

- Create: `apps/native/src-tauri/src/voice_models.rs`
- Modify: `apps/native/src-tauri/src/lib.rs` module declarations
- Modify: `apps/native/src-tauri/src/config.rs`
- Modify: `apps/native/src-tauri/src/stt/whisper.rs`
- Modify: `apps/native/src-tauri/Cargo.toml` only if checksum dependency is required
- Test: `voice_models.rs`, `config.rs`, and existing Whisper tests

**Interfaces:**

- Produces `ModelId::{Tiny, Base, Small}` with canonical IDs `tiny`, `base`, `small`.
- Produces `ModelCatalogEntry { id, filename, label, description, url, bytes, sha1 }`.
- Produces `catalog() -> &'static [ModelCatalogEntry]`, `entry_for_id(&str)`, and `entry_for_filename(&str)`.
- Produces serializable `VoiceModelInfo` and `VoiceModelsCatalog` DTOs for later commands.
- Whisper configuration accepts only a catalog ID or its canonical filename; no path separators or arbitrary model names.

- [ ] **Step 1: Write failing catalog/config tests.** Assert the catalog contains exactly `tiny`, `base`, and `small`; every URL is HTTPS and starts with the approved Hugging Face prefix; filenames are exactly `ggml-tiny.bin`, `ggml-base.bin`, and `ggml-small.bin`; upstream SHA-1 values are the published values from the approved repository; unknown IDs, URLs, path separators, and unsupported filenames are rejected.

- [ ] **Step 2: Run focused tests and verify failure.**

```bash
cd apps/native/src-tauri
cargo test voice_models -- --nocapture
cargo test config -- --nocapture
cargo test stt::whisper -- --nocapture
```

Expected: FAIL because the catalog and validation helpers do not exist.

- [ ] **Step 3: Add the catalog constants.** Use these exact entries:

```text
tiny  → ggml-tiny.bin  → 75 MiB  → bd577a113a864445d4c299885e0cb97d4ba92b5f
base  → ggml-base.bin  → 142 MiB → 465707469ff3a37a2b9b8d8f89f2f99de7299dac
small → ggml-small.bin → 466 MiB → 55356645c2b361a969dfd0ef2c5a50d530afd8d5
```

Use URLs of the form:

```text
https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-<id>.bin
```

The URL must be an internal constant, never derived from a webview argument.

- [ ] **Step 4: Implement ID/filename validation.** Normalize only whitespace and case where the existing config convention allows it; never normalize path separators into valid names. Return a safe configuration error for unknown IDs or filenames.

- [ ] **Step 5: Update Whisper resolution.** Make `WhisperProvider` resolve through `entry_for_id`/`entry_for_filename`, while preserving existing detection of the user-installed `whisper-cli` binary. A model absent from the models directory must return a setup error, not spawn the CLI.

- [ ] **Step 6: Run focused tests and clippy.**

```bash
cargo test voice_models -- --nocapture
cargo test config -- --nocapture
cargo test stt::whisper -- --nocapture
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS. Existing Whisper path-traversal tests must remain passing.

- [ ] **Step 7: Commit.**

```bash
git add apps/native/src-tauri/src/voice_models.rs apps/native/src-tauri/src/lib.rs apps/native/src-tauri/src/config.rs apps/native/src-tauri/src/stt/whisper.rs apps/native/src-tauri/Cargo.toml apps/native/src-tauri/Cargo.lock
git commit -m "feat(native): define curated Whisper model catalog"
```

---

## Task 2: Implement Rust download manager and status lifecycle

**Files:**

- Modify: `apps/native/src-tauri/src/voice_models.rs`
- Modify: `apps/native/src-tauri/src/lib.rs`
- Modify: `apps/native/src-tauri/src/paths.rs` only if needed
- Test: `voice_models.rs` unit tests and command DTO tests

**Interfaces:**

- `VoiceModelManager::new()` owns one active download slot and cancellation token.
- `VoiceModelManager::catalog_payload() -> Vec<VoiceModelCatalogPayload>` returns only safe catalog metadata.
- `VoiceModelManager::status() -> WhisperDownloadStatus` returns binary/model/download state.
- `VoiceModelManager::start_download(model: ModelId) -> Result<(), VoiceDownloadError>` starts one async download.
- `VoiceModelManager::cancel_download() -> Result<(), VoiceDownloadError>` is idempotent when idle.
- `VoiceModelManager::remove_model(model: ModelId) -> Result<(), VoiceDownloadError>` rejects the active selected model and removes only a validated catalog filename.

- [ ] **Step 1: Add failing filesystem lifecycle tests.** Use an injectable temporary root and local HTTP fixture/test server. Test 0600 temporary-file creation, progress DTO shape, successful verified install, checksum mismatch cleanup, cancellation cleanup, existing-final-file preservation, busy rejection, and active-model removal rejection.

- [ ] **Step 2: Run focused tests and verify failure.**

```bash
cargo test voice_models::tests::download -- --nocapture
```

Expected: FAIL because the manager/download functions do not exist.

- [ ] **Step 3: Add manager state and commands.** Store `VoiceModelManager` in `AppState`; add command handlers:

```text
voice_models_catalog()
whisper_status()
whisper_download(model: String)
whisper_cancel_download()
whisper_remove_model(model: String)
```

Validate model IDs before any filesystem or network operation. `whisper_status` must include detected binary, installed catalog models, and nullable active download progress.

- [ ] **Step 4: Implement streamed Hugging Face download.** Use the existing Rust HTTP client stack, request the fixed catalog URL, stream response bytes, write `<filename>.tmp` with mode `0600`, count bytes, and emit throttled `whisper:download-progress` events containing only `{model, received, total}`. Do not log response bodies or URLs containing secrets.

- [ ] **Step 5: Implement verification and atomic install.** Hash the complete temporary file with the selected checksum implementation and compare to the catalog digest. Enforce the catalog's expected-size sanity bound. On success, atomically rename the temp file to the final model filename. On every error/cancel path, remove only the temp file and leave any existing final file unchanged.

- [ ] **Step 6: Implement cancellation and single-download behavior.** Use a cancellation token owned by the manager. A second download returns a safe busy error. Cancellation is idempotent while idle and joins/cleans the active task before reporting idle status.

- [ ] **Step 7: Implement validated removal.** Refuse removal of the currently selected Whisper model. Accept only catalog IDs, remove only the corresponding model file, and return refreshed status. Removing a non-existent model is idempotent.

- [ ] **Step 8: Run focused tests and full Rust checks.**

```bash
cargo test voice_models -- --nocapture
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo check
```

Expected: PASS; no temp files remain in test roots after failed/cancelled downloads.

- [ ] **Step 9: Commit.**

```bash
git add apps/native/src-tauri/src/voice_models.rs apps/native/src-tauri/src/lib.rs apps/native/src-tauri/src/paths.rs
 git commit -m "feat(native): download Whisper models from Hugging Face"
```

---

## Task 3: Extend config and Listen setup validation

**Files:**

- Modify: `apps/native/src-tauri/src/config.rs`
- Modify: `apps/native/src-tauri/src/listen.rs`
- Modify: `apps/native/src-tauri/src/lib.rs`
- Test: config/listen/lib Rust tests

**Interfaces:**

- `models.stt_model` for Whisper accepts only a catalog ID/filename and must refer to an installed model before Listen starts.
- `listen_status`/setup errors continue to expose only sanitized `{message, needs_setup}`.
- Selecting Deepgram continues to accept its configured model IDs and masked `keys.json` key behavior.

- [ ] **Step 1: Write failing validation tests.** Cover selecting an unknown Whisper model, selecting a known-but-not-installed model, selecting an installed model, missing `whisper-cli`, and the existing missing-Deepgram-key setup error/resync path.

- [ ] **Step 2: Run focused tests and verify failure.**

```bash
cargo test config -- --nocapture
cargo test listen -- --nocapture
cargo test tests:: -- --nocapture
```

Expected: FAIL for the new unavailable-model cases.

- [ ] **Step 3: Implement provider-specific model validation.** Keep config serialization non-secret. Validate Whisper IDs against the catalog and defer filesystem availability checks to Listen start/status, so Settings can select a downloaded model without coupling config parsing to runtime filesystem state.

- [ ] **Step 4: Update Listen setup checks.** Before creating audio sources/workers, verify configured Whisper model exists and `whisper-cli` resolves. Return durable setup error with `needs_setup:true`; do not create a session or start capture on setup failure.

- [ ] **Step 5: Add status consistency tests.** Verify missing selected model reports Settings guidance, downloaded model plus missing binary reports binary guidance, and valid binary/model starts the existing Listen pipeline unchanged.

- [ ] **Step 6: Run checks and commit.**

```bash
cargo test config -- --nocapture
cargo test listen -- --nocapture
cargo test tests:: -- --nocapture
cargo clippy --all-targets --all-features -- -D warnings
```

Stage only the changed Rust files and commit:

```bash
git add apps/native/src-tauri/src/config.rs apps/native/src-tauri/src/listen.rs apps/native/src-tauri/src/lib.rs
git commit -m "feat(native): validate selected voice models"
```

---

## Task 4: Add typed frontend catalog/status/download contracts

**Files:**

- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/lib/events.ts`
- Test: TypeScript compilation and pure DTO helpers if available

**Interfaces:**

```ts
export interface VoiceModelCatalogEntry {
  id: 'tiny' | 'base' | 'small';
  filename: string;
  label: string;
  description: string;
  bytes: number;
  source: string;
}

export interface WhisperInstalledModel {
  id: string;
  filename: string;
  installed: boolean;
  bytes: number;
}

export interface WhisperDownload {
  model: string;
  received: number;
  total: number;
}

export interface WhisperStatus {
  binary: string | null;
  models: WhisperInstalledModel[];
  download: WhisperDownload | null;
}
```

Commands:

```ts
voiceModelsCatalog(): Promise<VoiceModelCatalogEntry[]>
whisperStatus(): Promise<WhisperStatus>
whisperDownload(model: string): Promise<void>
whisperCancelDownload(): Promise<void>
whisperRemoveModel(model: string): Promise<WhisperStatus>
```

- [ ] **Step 1: Add event/type contract tests or compile fixtures.** Assert download progress uses `{model, received, total}` and no URL/path/key fields are part of the event type.

- [ ] **Step 2: Implement wrappers.** Match Rust command names and argument keys exactly. Keep existing `whisperStatus` consumers source-compatible by adapting the extended model shape in one place.

- [ ] **Step 3: Add event constants.** Add `EV_WHISPER_DOWNLOAD_PROGRESS` and `EV_WHISPER_DOWNLOAD_ERROR` with documented payload types.

- [ ] **Step 4: Run frontend checks.**

```bash
cd apps/native
bun x tsc --noEmit
bun run build
bun test --pass-with-no-tests
```

Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add apps/native/src/lib/commands.ts apps/native/src/lib/events.ts
git commit -m "feat(native): expose voice model download contracts"
```

---

## Task 5: Extract the shared VoiceSetup component

**Files:**

- Create: `apps/native/src/components/prefs/VoiceSetup.tsx`
- Modify: `apps/native/src/components/prefs/ProvidersTab.tsx`
- Modify: `apps/native/src/lib/classes.ts` only if needed
- Test: TypeScript/build and any available pure UI tests

**Interfaces:**

- Props include `data: PrefsData`, `showSkip?: boolean`, `onSkip?: () => void`, and `onContinue?: () => void`.
- The component consumes `voiceModelsCatalog`, `whisperStatus`, `whisperDownload`, `whisperCancelDownload`, `whisperRemoveModel`, `configSet`, and existing Deepgram keystore wrappers.
- Settings mode renders persistent controls; onboarding mode renders the same controls plus optional skip/continue actions.

- [ ] **Step 1: Extract current Deepgram/Whisper settings behavior without changing UX.** Move the existing Voice UI from `ProvidersTab.tsx` into `VoiceSetup.tsx`; keep the existing masked key behavior and provider/model config keys.

- [ ] **Step 2: Add catalog/model cards.** Render Tiny/Base/Small from the Rust catalog rather than hardcoded UI-only model assumptions. Show source, size, installed state, selected state, and binary detection.

- [ ] **Step 3: Wire download controls.** Subscribe to progress/error events, update progress by model ID, disable competing downloads, provide Cancel, refresh status after completion/failure, and never render the download URL as an editable field.

- [ ] **Step 4: Wire select/remove controls.** `Use this model` writes `models.stt_model`; Remove calls Rust and refreshes status. Disable remove for the active selected model and show why.

- [ ] **Step 5: Add clear missing-binary guidance.** State that the user must install `whisper-cli` separately; do not offer or imply a binary download.

- [ ] **Step 6: Run frontend checks.**

```bash
cd apps/native
bun x tsc --noEmit
bun run build
bun test --pass-with-no-tests
```

Expected: PASS.

- [ ] **Step 7: Commit.**

```bash
git add apps/native/src/components/prefs/VoiceSetup.tsx apps/native/src/components/prefs/ProvidersTab.tsx apps/native/src/lib/classes.ts
 git commit -m "feat(native): share voice setup controls"
```

---

## Task 6: Add the optional Voice onboarding step

**Files:**

- Modify: `apps/native/src/components/prefs/Onboarding.tsx`
- Modify: `apps/native/src/components/prefs/VoiceSetup.tsx` only if onboarding props need refinement
- Test: TypeScript/build and onboarding state-flow tests if available

**Interfaces:**

- Onboarding adds a Voice step before Done.
- `VoiceSetup` receives `showSkip={true}` and a skip callback that advances without writing provider/model changes.
- Existing chat BYOK step, screen access step, and `app.onboarding_done` completion semantics remain unchanged.

- [ ] **Step 1: Write the onboarding flow test/fixture.** Verify the wizard sequence includes Voice, `Skip for now` reaches Done, and skipping does not call `configSet('models.stt_provider', ...)` or mutate existing voice configuration.

- [ ] **Step 2: Add the Voice step.** Increase `STEP_LABELS`, render `VoiceSetup` for the new step, wire Back/Continue/Skip, and preserve the existing remount/fade behavior.

- [ ] **Step 3: Ensure download cancellation is onboarding-specific.** When the Voice step unmounts because the user skips, goes back, or completes, cancel an active download. Settings mode must not cancel downloads on unmount/window close.

- [ ] **Step 4: Preserve optional completion.** `finish()` continues to write only `app.onboarding_done`; no Voice provider/model is required for completion.

- [ ] **Step 5: Run frontend checks.**

```bash
cd apps/native
bun x tsc --noEmit
bun run build
bun test --pass-with-no-tests
```

Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add apps/native/src/components/prefs/Onboarding.tsx apps/native/src/components/prefs/VoiceSetup.tsx
 git commit -m "feat(native): add optional voice onboarding"
```

---

## Task 7: Integrate Listen status/model display and setup recovery

**Files:**

- Modify: `apps/native/src/components/ListenSection.tsx`
- Modify: `apps/native/src/views/Bar.tsx` only if setup/download status needs a mode transition
- Modify: `apps/native/src/lib/commands.ts` only if status types changed in prior tasks
- Test: TypeScript/build; pure state tests if available

- [ ] **Step 1: Verify the current Listen setup error path.** Ensure missing Deepgram key, missing Whisper model, and missing `whisper-cli` show durable error state and Open Settings, including cold-open resync.

- [ ] **Step 2: Use the authoritative configured model.** The status chip must show the configured Deepgram model or selected Whisper filename/ID, never the first discovered model.

- [ ] **Step 3: Keep collapse/reopen behavior.** An active Listen session remains visible as Listen after card collapse/reopen; explicit Stop still ends the session.

- [ ] **Step 4: Run checks.**

```bash
cd apps/native
bun x tsc --noEmit
bun run build
bun test --pass-with-no-tests
```

Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add apps/native/src/components/ListenSection.tsx apps/native/src/views/Bar.tsx apps/native/src/lib/commands.ts
 git commit -m "fix(native): keep Listen model and setup status authoritative"
```

---

## Task 8: Full verification and manual matrix

**Files:**

- Modify: only files required to fix failures caused by Tasks 1–7.
- Test: all Rust/frontend checks and manual macOS verification.

- [ ] **Step 1: Run Rust verification.**

```bash
cd apps/native/src-tauri
cargo fmt -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo check
```

Expected: tests, clippy, and check pass. If repository-wide formatting reports pre-existing unrelated files, document the exact files and do not reformat drive-by code.

- [ ] **Step 2: Run frontend verification.**

```bash
cd apps/native
bun x tsc --noEmit
bun run build
bun test --pass-with-no-tests
```

Expected: PASS; no frontend component test harness is available unless the repository gains one during implementation.

- [ ] **Step 3: Inspect hygiene.**

```bash
cd ../..
git diff --check
git status --short
git diff develop...HEAD --stat
```

Confirm no keys, PCM/audio files, partial `.tmp` models, generated bundles, or arbitrary URLs were committed.

- [ ] **Step 4: Run manual onboarding/Settings matrix.**

1. Start onboarding and skip Voice; complete onboarding successfully.
2. Open Settings → Voice and select Deepgram; save a masked key and choose `nova-2`.
3. Select Whisper; verify `whisper-cli` missing guidance.
4. Download Tiny from Hugging Face; verify progress, installed state, and source label.
5. Cancel Base mid-download; verify no final or partial replacement remains.
6. Download/select Base; start Listen and verify the selected model chip.
7. Navigate away/close Settings during a download; verify it continues.
8. Reopen Settings and verify download status resync.
9. Delete the selected model externally; verify Listen shows Settings guidance.
10. Attempt to remove the active model; verify the action is blocked.
11. Complete onboarding Voice setup, then skip a later download; verify optional completion and cleanup behavior.
12. Inspect events/logs for absence of API keys, PCM, transcript payloads from downloads, and local audio paths.

- [ ] **Step 5: Document unavailable manual checks honestly.** If no authorized macOS GUI/provider environment is available, mark the affected checks not run and retain the exact reason; do not claim them as passed.

- [ ] **Step 6: Commit focused verification fixes only.**

```bash
git add apps/native/src-tauri/src/voice_models.rs apps/native/src-tauri/src/lib.rs apps/native/src-tauri/src/config.rs apps/native/src-tauri/src/listen.rs apps/native/src-tauri/src/stt/whisper.rs apps/native/src/components/prefs/VoiceSetup.tsx apps/native/src/components/prefs/ProvidersTab.tsx apps/native/src/components/prefs/Onboarding.tsx apps/native/src/components/ListenSection.tsx apps/native/src/views/Bar.tsx apps/native/src/lib/commands.ts apps/native/src/lib/events.ts
git commit -m "fix(native): harden voice setup verification findings"
```

---

## Coverage Check

- Optional onboarding Voice step: Task 6.
- Shared onboarding/Settings UI: Task 5.
- Deepgram key/model configuration: Task 5.
- Hugging Face catalog and exact curated models: Task 1.
- Rust-owned model download/progress/cancellation/removal: Task 2.
- Model validation and Listen setup errors: Task 3.
- Typed Tauri commands/events: Task 4.
- Authoritative Listen model/setup display: Task 7.
- Privacy, atomicity, checksum, and cleanup tests: Tasks 1–3.
- Full automated/manual verification: Task 8.

No spec requirement is intentionally unassigned.
