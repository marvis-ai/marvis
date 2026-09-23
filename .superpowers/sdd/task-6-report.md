# Task 6 Verification Report

Date: 2026-09-23
Branch: `feature/implement_voice_listening_part`
Starting commit: `4f971dd`

## Result

Task 6 verification found and fixed one feature-caused packaging/runtime mismatch. Tauri packages the target-suffixed staged sidecar as `Contents/MacOS/whisper-cli`, while development staging uses `whisper-cli-aarch64-apple-darwin`/`whisper-cli-x86_64-apple-darwin`. Runtime now prefers the target-specific development name and falls back to the packaged `whisper-cli` name. The fix is commit `6fee9d1` (`fix(native): resolve packaged Whisper sidecar name`).

The arm64 source build and arm64 Tauri app bundle were verified. The x86_64 CI artifact, a second-architecture app, release signing, and interactive GUI/model/transcription checks were not fully available in this environment and are explicitly not claimed as passed below.

## Automated commands and results

All commands were run from the checked-out repository unless a working directory is shown.

| Command | Result |
| --- | --- |
| `bash scripts/build-whisper-cli.sh --check-only --target aarch64-apple-darwin` (from `apps/native/src-tauri`) | PASS. Reported `whisper.cpp v1.9.2 metadata and output names validated`. |
| `bash scripts/build-whisper-cli.sh --check-only --target x86_64-apple-darwin` | PASS for metadata/target naming only; no artifact was present at that point. |
| `cargo test` (from `apps/native/src-tauri`) | PASS: 214 passed, 0 failed, 1 ignored; doc test ignored. Existing warning: `block v0.1.6` future incompatibility. |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS. Existing `block v0.1.6` future-incompatibility warning. |
| `cargo check` | PASS. Existing `block v0.1.6` future-incompatibility warning. |
| `bun x tsc --noEmit` | FAIL/NOT APPLICABLE as written: root has no `tsconfig.json`; Bun printed TypeScript help and exited 1. This is a repository command/setup issue, not a feature type error. |
| `bun x tsc -p apps/native/tsconfig.json --noEmit` | PASS. |
| `bun run build` | PASS: Turbo built `@marvis/native` and `@marvis/web`. Existing Vite warning about a chunk over 500 kB and plugin timing output. |
| `bun test --pass-with-no-tests` | PASS: 18 tests passed, 0 failed. |
| `git diff --check` | PASS after the focused fix. |
| `bash src-tauri/scripts/check-task-2-packaging.sh --fixture` (from `apps/native`) | PASS: config and deterministic app-layout fixture validated. |
| `bash scripts/build-whisper-cli.sh --check-only --require-staged --target aarch64-apple-darwin` before source build | Expected FAIL: missing artifact. This correctly proves staged-artifact enforcement. |
| `bash scripts/build-whisper-cli.sh --target aarch64-apple-darwin` | PASS on local arm64 macOS. Cloned exact `v1.9.2`, configured Release + `WHISPER_BUILD_EXAMPLES=ON` + `WHISPER_BUILD_TESTS=OFF` + `GGML_METAL=ON` + `CMAKE_OSX_ARCHITECTURES=arm64`, built executable, checksum, and validation. Warnings: missing OpenMP/ccache, CMake deprecation, macOS 27 Metal deprecations, and one compiler warning in upstream `stb_vorbis.c`. |
| `file binaries/whisper-cli-aarch64-apple-darwin` | PASS: `Mach-O 64-bit executable arm64`. |
| `bash scripts/build-whisper-cli.sh --check-only --require-staged --target aarch64-apple-darwin` after build | PASS, including executable, architecture, and SHA-256 validation. |
| `bash scripts/check-task-2-packaging.sh --target aarch64-apple-darwin` | PASS. |
| `bun x tauri build --target aarch64-apple-darwin` before fix | Build PASS, but inspection exposed the feature bug: app contained `Contents/MacOS/whisper-cli`, not the runtime's expected target-suffixed filename. |
| `bun x tauri build --target aarch64-apple-darwin` after fix | PASS. Built `Marvis.app` and DMG. Rust release build repeated the `block v0.1.6` warning; frontend repeated the Vite chunk warning. |
| `file target/aarch64-apple-darwin/release/bundle/macos/Marvis.app/Contents/MacOS/Marvis` | PASS: arm64 Mach-O. |
| `file target/aarch64-apple-darwin/release/bundle/macos/Marvis.app/Contents/MacOS/whisper-cli` | PASS: arm64 Mach-O. |
| executable bit check on packaged `whisper-cli` | PASS: executable. |
| `codesign --verify --deep --strict --verbose=2 Marvis.app` | FAIL: `code has no resources but signature indicates they must be present`. Signing is not configured/valid for this local artifact; no signing fix was attempted. |
| Invalid staging fixture (`ASCII text`, executable, matching checksum) passed to `--stage` | PASS safety behavior: rejected with architecture mismatch before staging. |

## Focused verification fix

Files changed in commit `6fee9d1`:

- `apps/native/src-tauri/src/lib.rs:1497-1508`: runtime chooses the target-specific sidecar when present (development staging), otherwise the packaged `whisper-cli` sidecar emitted by Tauri.
- `apps/native/src-tauri/src/paths.rs:61-67`: adds the packaged sidecar path helper; the path unit test at `:179-192` covers both target-specific and packaged names.
- `apps/native/src-tauri/src/lib.rs:115`: updates the state comment to cover both layouts.

No workflow or packaging configuration was changed. The existing source/build validation remains in `apps/native/src-tauri/scripts/build-whisper-cli.sh:61-78` and Tauri declares `externalBin` at `apps/native/src-tauri/tauri.conf.json:17-20`.

## Manual macOS matrix

| Check | Status | Evidence / limitation |
| --- | --- | --- |
| Development build, no `whisper-cli` in PATH, staged bundled binary preferred | PASS at resolver level; not interactive GUI tested | Rust tests `stt::whisper::tests::bundled_binary_wins_and_reports_source` and `fallback_order_and_invalid_candidates_are_source_safe` passed. |
| Development build with no bundled binary; PATH/Homebrew/user-local fallback | PASS at resolver level; not interactive GUI tested | Same resolver tests cover PATH and user fallback ordering/invalid candidates. Homebrew paths are covered by implementation/tests but were not modified. |
| Packaged arm64 `.app` selects bundled arm64 binary | PASS for artifact/package inspection after fix | App and sidecar both reported arm64; packaged sidecar is `Contents/MacOS/whisper-cli`; runtime fallback now points to that name. No GUI launch was performed. |
| Packaged x86_64 artifact selects x86_64 binary | NOT RUN | Current host is arm64 macOS; no Intel runner/artifact was available locally. Workflow declares `macos-15-intel`, but GitHub Actions was not executed from this checkout. |
| Voice Settings reports `Bundled with Marvis` | NOT RUN interactively | Source/runtime status implementation and Rust tests passed; no interactive packaged app session was available. |
| Download Tiny, select it, start Listen | NOT RUN | Requires interactive app, microphone permissions, and model download/network. Model lifecycle tests passed. |
| Transcription launches bundled binary | NOT RUN end-to-end | No GUI/audio session or downloaded model was available. Resolver and provider tests passed; invalid binaries are rejected before launch. |
| Invalid/incompatible binary rejected without launch/crash | PASS for staging validation and resolver safety | Invalid executable staging was rejected by `file` architecture validation; Rust tests cover non-executable/invalid fallback candidates. No crash test launched an incompatible process. |
| Model updates preserve `~/.marvis/models/whisper/models/` | PASS automated; NOT manually interactive | `voice_models` lifecycle/preservation tests passed in `cargo test`; no live download session was run. |
| Public Listen events exclude binary path/API key/URL/PCM/transcript | PASS automated/source-level coverage; NOT manually observed | Rust listen/status/privacy-related tests passed and public DTO tests passed. No live event stream was captured. |

## Unavailable or pre-existing concerns

- The complete pinned GitHub Actions workflow was not run as a remote CI job. The local arm64 source-build script completed successfully and validated v1.9.2, flags, executable, architecture, and SHA-256. The x86_64 job remains unverified here.
- No x86_64 macOS host/artifact was available, so cross-architecture packaging cannot be claimed.
- Local `codesign --verify` failed because the generated app's signature/resources are not valid for strict verification. Release signing/notarization was not attempted and remains a release-environment check.
- The exact requested root `bun x tsc --noEmit` command is not runnable because this repository has no root TypeScript project configuration; the native project-specific equivalent passed, and `bun run build` typechecked native and web packages successfully.
- Existing warnings: Rust `block v0.1.6` future incompatibility; Vite bundle chunk over 500 kB; upstream whisper.cpp CMake/OpenMP/ccache and macOS SDK deprecation warnings; upstream `stb_vorbis.c` compiler warning.
- Generated artifacts, build output, and the locally staged binary are ignored by the repository and were not committed.

## Commits

- `6fee9d1 fix(native): resolve packaged Whisper sidecar name`
- The report is a separate verification document and should be committed separately after review.
