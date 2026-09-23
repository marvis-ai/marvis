# Task 4 implementation report

## Scope

Implemented the Task 4 bundled Whisper resource-context wiring from `task-4-brief.md`. The Tauri layer resolves the resource directory once during setup, derives the target-specific candidate with `paths::bundled_whisper_cli`, and passes only `Option<&Path>` across the Listen/STT boundary. No `AppHandle` or Tauri type was added to `ListenService` or the STT abstraction.

## Changes

- `apps/native/src-tauri/src/lib.rs:114-116,1494-1508`
  - Added `AppState::bundled_whisper`.
  - Resolves `handle.path().resource_dir()` during Tauri setup and derives the target-triple candidate.
  - `listen_start` passes the candidate to `ListenService::start`.
  - `whisper_status` and `whisper_remove_model` use bundled-aware status discovery.
- `apps/native/src-tauri/src/listen.rs:335-341,372-430`
  - Added path-only bundled context to `ListenService::start`.
  - Setup validation uses bundled-aware Whisper status, so a packaged app with only the bundled executable does not emit the misleading install error.
  - Provider construction receives the same context.
  - Existing sanitized setup events remain unchanged and contain no path/source fields.
- `apps/native/src-tauri/src/stt/whisper.rs:61-111,567-590`
  - `WhisperProvider::new` and status discovery accept the optional bundled candidate.
  - Bundled candidate wins before PATH/Homebrew/user fallback.
  - Added/extended tests for bundled precedence and source-aware availability.
- `apps/native/src-tauri/src/stt/mod.rs:36-53`
  - Threaded the path-only context through the provider factory.
- `apps/native/src-tauri/src/voice_models.rs:292-317`
  - Made model/download status discovery accept bundled context while leaving model download/install behavior independent of executable discovery.
- `apps/native/src-tauri/src/paths.rs:51-58,170-179`
  - Documented target-triple staging as the architecture safeguard and strengthened the test to assert the exact target-specific filename.
- `apps/native/src/lib/commands.ts:271-284`
  - Updated the TypeScript DTO to match the actual Rust payload: `binary_status.available` and nullable `binary_status.source` (`Bundled`, `Path`, `Homebrew`, `User`). Existing binary/model/download fields remain.
- `apps/native/src/components/prefs/VoiceSetup.tsx:101-106`
  - Updated the TypeScript fallback object for the new required DTO field.

## Architecture rationale

The bundled resolver only considers the exact `whisper-cli-{target-triple}` path produced by `paths::bundled_whisper_cli`. Packaging stages binaries under that exact target-specific name, and `paths` now has an explicit test asserting the complete filename. This prevents a neighboring architecture's staged binary from being selected. Runtime binary-format inspection was not added because the supported packaging contract already enforces target-triple staging and the path helper is intentionally filesystem/platform neutral.

## Verification

Focused Rust checks:

- `cargo test listen -- --nocapture` — pass (16 tests)
- `cargo test stt::whisper -- --nocapture` — pass (9 tests)
- `cargo test tests:: -- --nocapture` — pass as part of the focused invocation/full suite

Full Rust checks:

- `cargo test` — pass: 214 passed, 1 ignored
- `cargo clippy --all-targets --all-features -- -D warnings` — pass
- `cargo check` — pass

Frontend checks:

- Direct root `bun x tsc --noEmit` was not applicable because the monorepo root has no `tsconfig.json`; it printed TypeScript CLI help and exited nonzero.
- Equivalent configured check `bun run check-types` — pass.
- `bun run build` — pass (native Vite and web Next builds; native emitted only the pre-existing chunk-size warning).
- `bun test --pass-with-no-tests` — pass: 18 tests.

Diff review:

- `git diff --check` — pass.
- `cargo fmt --all -- --check` still reports pre-existing formatting differences in unrelated `llm/*` and `windows/movement.rs` files; those files were intentionally not included in this change.

## Commit

Committed as `1598ad9` (`Expose bundled Whisper runtime status`).
