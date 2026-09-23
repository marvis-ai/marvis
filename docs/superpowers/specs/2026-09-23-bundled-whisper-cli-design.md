# Marvis — Bundled Whisper CLI Design

Date: 2026-09-23
Status: Approved design (pending written-spec review)
Branch: `feature/implement_voice_listening_part`
Related spec: `docs/superpowers/specs/2026-09-23-voice-setup-model-download-design.md`

## Goal

Bundle a pinned, architecture-appropriate `whisper-cli` executable with the
Marvis macOS app so local Whisper works after a model download without asking
users to install Homebrew or manually locate the CLI. Preserve PATH, Homebrew,
and user-local fallbacks for development and advanced users.

## User decisions

| Question | Decision |
| --- | --- |
| Installation model | Bundle `whisper-cli` with the Marvis app; do not download the CLI at model-download time. |
| Binary source | Pinned prebuilt official whisper.cpp release binaries. |
| Architectures | Bundle arm64 and x86_64 macOS binaries; resolve the one matching the running app architecture. |
| Model relationship | Model download remains a separate Hugging Face operation. The bundled CLI is already available when the model is downloaded. |
| Fallbacks | Preserve PATH, `/opt/homebrew/bin/whisper-cli`, and `~/.marvis/models/whisper/bin/whisper-cli` fallback resolution. |

## Current-state context

The Listen implementation currently resolves `whisper-cli` in this order:

1. `PATH`
2. `/opt/homebrew/bin/whisper-cli`
3. `~/.marvis/models/whisper/bin/whisper-cli`

If no binary is found, Listen emits the setup error:

```text
whisper-cli was not found; install it and try again
```

The optional Voice setup/model-download feature already downloads curated
Whisper model data into `~/.marvis/models/whisper/models/`. This design changes
binary provisioning only. Models remain user data and are never executed.

## Packaging

Add pinned prebuilt binaries to the native packaging input:

```text
apps/native/src-tauri/
├── binaries/
│   ├── whisper-cli-aarch64-apple-darwin
│   └── whisper-cli-x86_64-apple-darwin
```

The exact whisper.cpp release/version and SHA-256 checksums are pinned in the
release configuration. No unversioned `main` branch artifact is accepted. The
release preparation flow must verify each binary checksum before packaging.

The binaries are bundled as Tauri external binaries/resources and included in
the signed macOS `.app`. They must:

- Be executable.
- Match their declared target architecture.
- Be code-signed as part of the Marvis app signing flow.
- Never be downloaded by the running application.
- Never be copied into `~/.marvis/models/whisper/bin/`.

The runtime resource path must be derived from the Tauri app/resource
configuration, not guessed from the current working directory.

## Runtime resolution

`WhisperProvider::discover()` becomes source-aware and resolves in this order:

1. Bundled app binary matching the current macOS architecture.
2. An executable `whisper-cli` found in `PATH`.
3. `/opt/homebrew/bin/whisper-cli`.
4. `~/.marvis/models/whisper/bin/whisper-cli`.

The bundled binary is preferred in production so the app is deterministic. The
fallbacks remain for development, custom builds, and advanced users who need a
custom whisper.cpp binary.

Resolution must reject a missing, non-regular, or non-executable file. It must
not launch a binary during discovery.

The resolver should return source-aware internal status:

```rust
enum WhisperBinarySource {
    Bundled,
    Path,
    Homebrew,
    User,
}
```

The public Settings status may expose only the safe source label and local
binary availability. Listen events expose neither a path nor an architecture.

## Development behavior

Development builds use the same resolution logic:

- If a bundled development binary is available, it is preferred.
- Otherwise PATH/Homebrew/user-local fallbacks are checked.
- If none exists, the Voice UI explains that the current development build has
  no usable Whisper CLI; this is a setup state, not a panic.

The app must not silently build whisper.cpp at runtime. Building from source is
not part of this feature.

## Model/download behavior

The bundled CLI is available independently of model download. When the user
clicks **Download** for Tiny/Base/Small:

1. Rust validates the fixed catalog model ID.
2. Rust confirms binary availability for status/reporting but does not execute
   the binary during download.
3. Rust downloads and verifies the model as defined by the related model
   download spec.
4. The verified model is atomically installed.
5. The user explicitly selects the model.
6. Listen starts using the bundled CLI plus the selected model.

If the bundled binary is unavailable in a development build, model download
may still succeed, but Listen remains unavailable with a safe setup message.

## Settings UX

The Voice settings surface reports one of:

- **Bundled with Marvis** — the expected packaged binary was found.
- **Custom whisper-cli detected** — a PATH/Homebrew/user-local fallback was
  found.
- **Whisper CLI unavailable** — no usable binary was found.

The UI must not show a false “install whisper-cli” message when the bundled
binary is present. It may show development guidance when no binary exists.

The actual local binary path may remain available in the existing local Settings
status DTO if needed for diagnostics, but it must not appear in Listen events,
summary payloads, provider requests, or logs by default.

## Error handling and privacy

- Missing bundle resource → safe setup state; no panic.
- Wrong architecture → safe setup state; no attempt to execute it.
- Missing execute permission → safe setup state; no attempt to execute it.
- Process launch failure → sanitized Listen runtime error; no crash.
- Process exit failure → existing Whisper provider failure handling.
- Model download failures remain separate from binary resolution failures.
- Binary paths, URLs, API keys, PCM, transcripts, and response bodies never
  enter public Listen events.
- Downloaded model files remain data files and are never executed.

## Testing

Rust tests:

- Bundled binary is preferred over PATH/Homebrew/user-local binaries.
- PATH/Homebrew/user-local fallback resolution still works when bundle is
  absent.
- Missing, non-regular, non-executable, and wrong-architecture candidates
  are rejected safely.
- Source-aware status serializes only the documented safe fields.
- Synchronous and async Whisper startup use the resolved bundled path without
  panicking.
- Model download does not execute or mutate the bundled binary.
- Missing bundled binary produces a setup result rather than a panic.

Packaging/build verification:

```text
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo check
bun x tsc --noEmit
bun run build
bun test --pass-with-no-tests
```

Manual macOS verification:

1. Build a development app with no `whisper-cli` in PATH.
2. Verify a bundled development binary is detected when present.
3. Build/package the arm64 `.app` and verify the bundled arm64 binary is used.
4. Build/package the x86_64 artifact or verify the x86_64 resolver path.
5. Open Voice Settings and confirm `Bundled with Marvis`.
6. Download Tiny from Hugging Face.
7. Select Tiny and start Listen.
8. Confirm no `whisper-cli was not found` error appears.
9. Confirm transcription launches the bundled binary.
10. Remove/disable the bundled candidate in a test build and confirm fallback
    resolution works.
11. Confirm invalid/incompatible binaries do not launch or crash the app.
12. Confirm app updates replace the bundled binary while preserving models in
    `~/.marvis/models/whisper/models/`.

## Out of scope

- Downloading the CLI binary at runtime.
- Building whisper.cpp from source on the user's machine.
- Automatic model selection.
- Installing binaries into `~/.marvis/models/whisper/bin/`.
- Windows/Linux support.
- Changing Deepgram or model-download provider behavior beyond the bundled CLI
  status/resolution integration.
