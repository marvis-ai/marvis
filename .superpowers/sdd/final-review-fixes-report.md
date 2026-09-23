# Final Whole-Branch Review Fixes

Date: 2026-09-23

## Important findings fixed

### 1. Runtime bundled Mach-O architecture validation

`apps/native/src-tauri/src/stt/whisper.rs` now validates a bundled candidate with
`file -b` on macOS after checking regular-file and executable metadata. The
expected architecture is compiled from the current Rust target (`arm64` for
Apple Silicon and `x86_64` for Intel). A missing, non-Mach-O, or wrong-slice
candidate is rejected before it can be selected or launched, allowing the
existing PATH/Homebrew/user fallback or the existing sanitized setup error to
apply. Non-macOS builds retain the previous metadata-only behavior.

Focused coverage includes the wrong-architecture/non-Mach-O bundled candidate
case on macOS and keeps bundled preference/status coverage working by using the
current test executable as a valid Mach-O fixture. No binary path, key, URL,
audio, or transcript was added to app events.

### 2. Target-aware packaging validation

`apps/native/src-tauri/scripts/fixtures/task-2-app-layout.json` and
`check-task-2-packaging.sh` now validate both exact target layouts:
`aarch64-apple-darwin`/`arm64` and `x86_64-apple-darwin`/`x86_64`. The existing
build script already validates the selected target's `file` architecture and
SHA-256; the deterministic fixture now prevents an arm64-only regression.

### 3. Checked-in release artifact consumer path

`.github/workflows/whisper-cli.yml` now has arm64 and x86_64 packaging jobs.
Each job downloads the exact matching artifact, stages it with its exact
checksum, runs target-aware packaging validation, invokes the Tauri release
build for that target, and runs strict app verification. The jobs require the
real `APPLE_SIGNING_IDENTITY` secret and fail with an explicit release
prerequisite message when it is absent; no signing credential is fabricated.

`apps/native/src-tauri/scripts/verify-release-app.sh` checks app and bundled
Whisper executable modes and target architecture, then requires
`codesign --verify --deep --strict`. It never claims an unsigned or invalid app
is a release artifact.

### 4. Release operator documentation

`apps/native/README.md` documents exact `gh run download` → stage/checksum
validation → target-aware Tauri build → strict signature/architecture
verification commands. It explains the required signing identity and that
staging/build paths remain ignored. Generated Whisper binaries were not added
to Git.

## Verification

Passed:

- `bash -n` for build, packaging-check, and release-verification scripts.
- `bash src-tauri/scripts/check-task-2-packaging.sh --fixture`.
- `cargo test` — 215 passed, 1 ignored; doc test ignored.
- `cargo clippy --all-targets --all-features -- -D warnings`.
- `cargo check`.
- `bun x tsc --noEmit` from `apps/native`.
- `bun run build` from `apps/native`.
- `bun test --pass-with-no-tests` from `apps/native` (no tests found).
- `git diff --check`.
- Workflow YAML parse validation with PyYAML.

The full macOS release packaging jobs and strict signing verification were not
run locally because they require CI artifact downloads and a real Apple signing
identity. The checked-in workflow fails explicitly rather than claiming those
checks passed. Existing warnings remain: `block v0.1.6` future-incompatibility
and the existing Vite large-chunk warning.

## Final bundled Whisper review fixes

Date: 2026-09-23

### Findings fixed

1. **Architecture validation covers every candidate source.**
   - `apps/native/src-tauri/src/stt/whisper.rs:377-450` now applies the macOS Mach-O architecture check through `is_usable_candidate` to bundled, PATH, Homebrew, and user-local candidates. Non-macOS builds retain the prior metadata-only behavior because `architecture_matches` returns true outside macOS.
   - The resolver still only inspects metadata and never launches a candidate. Focused tests cover an invalid bundled executable and a wrong-architecture executable in the PATH fallback position, which is skipped in favor of a valid user candidate.

2. **Signing and release workflow semantics are explicit and fail closed.**
   - `.github/workflows/whisper-cli.yml:3-11,65-145` is manual-only. App packaging requires the explicit `package_app` workflow input; the default run builds CLI artifacts without producing an app.
   - Package jobs require both `APPLE_SIGNING_IDENTITY` and a matching pre-provisioned codesigning identity in the runner keychain. The workflow does not provision certificates, embed credentials, or weaken strict verification. Job names no longer claim output is signed; `verify-release-app.sh` remains the final strict `codesign --verify --deep --strict` gate.
   - `apps/native/README.md:112-130` documents the prerequisite and states that no signed release is claimed when the gate or strict verification fails.

3. **Checksum provenance is distinguished without invented hashes.**
   - `apps/native/README.md:122-130` explicitly distinguishes generated `.sha256` files from trusted release identity. Existing SHA-256 verification remains unchanged; release operators must retain exact workflow artifact provenance and record verified hashes in release notes or a controlled release manifest. No expected values were guessed or weakened.

4. **Model download does not execute downloaded payloads.**
   - `apps/native/src-tauri/src/voice_models.rs:817-840` adds `downloaded_model_is_never_executed`, downloading a shell-shaped fixture and asserting exact bytes are installed while its marker is never created. Existing privacy-safe DTO/event behavior remains unchanged.

### Verification

- `cd apps/native/src-tauri && cargo fmt --all && cargo test` — PASS: 216 passed, 1 ignored; doc-test ignored. Existing `block v0.1.6` future-incompatibility warning remains.
- `cd apps/native/src-tauri && cargo clippy --all-targets --all-features -- -D warnings` — PASS.
- `cd apps/native/src-tauri && cargo check` — PASS; existing future-incompatibility warning remains.
- `cd apps/native && bun x tsc --noEmit && bun run build && bun test --pass-with-no-tests` — PASS; existing Vite large-chunk warning and no-frontend-tests message remain.
- `cd apps/native/src-tauri && bash -n scripts/build-whisper-cli.sh scripts/check-task-2-packaging.sh scripts/verify-release-app.sh && bash scripts/check-task-2-packaging.sh --fixture` — PASS.
- `git diff --check` — PASS.

### Scope and release limitations

No signing certificate or remote GitHub Actions run was available in this environment, so signed release output and x86_64 packaging remain unclaimed. The workflow now makes those prerequisites explicit and fails closed. The local macOS host-specific architecture tests and existing target-aware packaging assertions remain intact.
