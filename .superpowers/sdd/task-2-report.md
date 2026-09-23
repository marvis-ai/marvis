# Task 2 Report: Architecture-specific Whisper CLI packaging

## Status

Implemented the Tauri external-binary declaration and CI-artifact staging validation without changing the runtime Rust/UI resolver.

## Changes

- `apps/native/src-tauri/tauri.conf.json:17-25`
  - Added `bundle.externalBin` with the Tauri 2 base path `binaries/whisper-cli`.
  - Tauri will select the target-triple file (`whisper-cli-aarch64-apple-darwin` or `whisper-cli-x86_64-apple-darwin`) during packaging.
  - Existing bundle activation, all-target setting, macOS private API, entitlements, and Info.plist settings remain unchanged.

- `apps/native/src-tauri/scripts/build-whisper-cli.sh:16-21,61-102,106-153`
  - Added `--stage ARTIFACT --target TARGET [--checksum CHECKSUM]`.
  - Validates the input artifact exists, is executable, reports the expected architecture with `file`, and matches the SHA-256 digest from the supplied sidecar.
  - Copies a validated artifact into `src-tauri/binaries/whisper-cli-TARGET`, normalizes executable mode, writes a staged checksum, and validates the staged result again.
  - Rejects missing artifacts, wrong architectures, missing checksums, bad checksums, unsupported targets, and invalid option combinations before packaging.
  - Checksum validation compares digest values rather than sidecar filenames, so downloaded CI sidecars remain valid after relocation.

Generated binaries remain excluded by the existing `apps/native/src-tauri/binaries/.gitignore` (`*`, with only `.gitignore` and `.gitkeep` allowed).

## Verification

Passed:

- `bash -n apps/native/src-tauri/scripts/build-whisper-cli.sh`
- `python3 -m json.tool apps/native/src-tauri/tauri.conf.json`
- `git diff --check`
- JSON assertions for `bundle.externalBin`, `app.macOSPrivateApi`, and the existing macOS Info.plist setting.
- Staging integration check using a temporary executable, checksum sidecar, and mocked arm64 Mach-O `file` output: staged file was executable and checksum-validated.
- Wrong-architecture staging rejection check.
- Missing-artifact staging rejection check.
- Confirmed no generated files remained tracked or present in `src-tauri/binaries` after testing.

Attempted per the brief:

- `cd apps/native/src-tauri && cargo test --lib package_config -- --nocapture`
  - The Tauri build script correctly failed before compilation because the current target artifact was absent: `resource path binaries/whisper-cli-aarch64-apple-darwin doesn't exist`. This confirms the new external-binary declaration enforces staged current-target presence before a Tauri build/test can proceed.

## Limitations / concerns

- A real staged macOS executable was not available in this environment, so an actual `.app` bundle inspection with `find`, `file`, and `test -x` could not be completed. The temporary staging test exercised the same validation and layout path with mocked architecture output.
- No `package_config` Rust test module existed, and this task intentionally did not add runtime Rust/UI resolver changes. The requested cargo command therefore reaches Tauri's resource-presence validation rather than a test function.

## Task 2 review fixes

- Added the committed `apps/native/src-tauri/scripts/check-task-2-packaging.sh` check and `scripts/fixtures/task-2-app-layout.json` deterministic app-layout fixture. The check fails with an explicit message if `bundle.externalBin` no longer contains `binaries/whisper-cli`; its staged mode requires the current target artifact, executable mode, and matching `.sha256` sidecar through `build-whisper-cli.sh --require-staged`.
- Added `check:task-2` (config plus deterministic fixture) and `check:task-2:staged` (release staging preflight) scripts to `apps/native/package.json`.
- Updated `build.rs` so debug cargo builds/tests override only `externalBin` through `TAURI_CONFIG`, preserving local `cargo test`/`cargo check` without generated binaries. Release builds and the checked-in Tauri config remain strict; `tauri build` still requires the staged current-target artifact.
- Real `.app` inspection was unavailable because this environment has no generated macOS Whisper executable. The committed fixture validates the expected bundle-relative path, arm64 architecture, and executable bit without committing a fake executable. It is not a claim that macOS packaging passed.

### Review-fix verification

- `bash -n` passed for both packaging scripts.
- `bun run check:task-2` passed; config-removal and missing staged artifact checks failed with the intended messages.
- `cargo test --lib` — **PASS** (212 passed, 1 ignored); `cargo check` — **PASS**. `cargo check --release` correctly **FAILS** without the staged current-target artifact (`resource path binaries/whisper-cli-aarch64-apple-darwin doesn't exist`).
- `bun x tsc --noEmit`, `bun run build`, and `bun test --pass-with-no-tests` — **PASS** (the existing large-chunk warning remains; no frontend tests were found).
- `python3 -m json.tool`, both `bash -n` checks, and `git diff --check` — **PASS**. Generated binaries remain absent and ignored.
