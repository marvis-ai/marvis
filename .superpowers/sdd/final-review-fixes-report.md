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
