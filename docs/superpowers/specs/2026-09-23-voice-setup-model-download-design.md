# Marvis — Optional Voice Setup and Whisper Model Downloads

Date: 2026-09-23
Status: Approved design (pending written-spec review)
Branch: `feature/implement_voice_listening_part`
Related spec: `docs/superpowers/specs/2026-09-22-listen-phase-2-design.md`
Reference: `https://huggingface.co/ggerganov/whisper.cpp`

## Goal

Make the completed Listen feature configurable before or after onboarding. Users
may choose Deepgram or local Whisper.cpp, download curated Whisper model files
from Hugging Face, and select the installed model in both onboarding and
Settings. Voice remains optional: users can finish onboarding without configuring
it and configure it later.

## User decisions

| Question | Decision |
| --- | --- |
| Onboarding | Add an optional Voice step with `Set up voice` and `Skip for now`; skipping never blocks completion. |
| Providers | Deepgram hosted STT or local Whisper.cpp. |
| Whisper downloads | Download curated `tiny`, `base`, and `small` model files. |
| Whisper binary | Model files only; `whisper-cli` remains user-installed and detected by the existing resolution order. |
| Download source | Official Hugging Face `ggerganov/whisper.cpp` repository. |
| Shared UI | Reuse one Voice configuration component in onboarding and Settings. |

## Current-state context

The Listen Phase 2 implementation already provides:

- `ProvidersTab.tsx` → Providers → Voice with Deepgram/Whisper selection.
- Deepgram masked key entry backed by `keys.json`.
- Deepgram model selection through `models.stt_model`.
- Whisper binary/model detection through `whisper_status`.
- Rust `WhisperProvider` resolution under `~/.marvis/models/whisper/models/`.
- Optional Voice behavior in Listen: missing setup produces a durable error and an Open Settings action.

The current implementation intentionally says the app does not download Whisper
binaries or models. This feature changes only model-file provisioning; the
binary remains user-installed.

## User flow

### Onboarding

The existing chat/provider setup remains unchanged. Add an optional Voice step
before the completion screen:

1. The user chooses **Deepgram**, **Whisper local**, or **Skip for now**.
2. Deepgram shows the masked key status, password key input, and model selector.
3. Whisper shows `tiny`, `base`, and `small` cards with download/select actions,
   plus the detected `whisper-cli` requirement.
4. `Skip for now` leaves the existing Voice configuration unchanged and moves to
   the completion step.
5. Completing the step persists only valid provider/model choices; it does not
   make Voice required for `app.onboarding_done`.
6. After onboarding, the Listen button is available. If Voice is unconfigured,
   Listen shows setup guidance and **Open Settings**.

The onboarding step starts from existing configuration when available. A failed
key save, invalid model selection, or failed download keeps the user on the
step with an inline error; it never marks Voice as configured accidentally.

### Settings

The existing Providers → Voice tab becomes the long-lived source of truth:

- Provider can be changed at any time.
- Deepgram key and model are editable.
- Whisper model catalog, download state, installed state, selected state, and
  removal are visible.
- Downloads continue while navigating within or closing the Settings window.
- Reopening Settings calls the status command and resynchronizes progress/state.
- The active Whisper model cannot be removed until another model is selected or
  Whisper is no longer the configured provider.

Onboarding and Settings use the same `VoiceSetup` component and command/event
contracts. They differ only in surrounding shell and whether `Skip for now` is
shown.

## Curated Hugging Face catalog

Rust owns the catalog. The webview can request the catalog but cannot provide
arbitrary URLs.

| ID | Filename | Source | Approximate size | Published SHA-1 |
| --- | --- | --- | --- | --- |
| `tiny` | `ggml-tiny.bin` | `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin` | 75 MiB | `bd577a113a864445d4c299885e0cb97d4ba92b5f` |
| `base` | `ggml-base.bin` | `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin` | 142 MiB | `465707469ff3a37a2b9b8d8f89f2f99de7299dac` |
| `small` | `ggml-small.bin` | `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin` | 466 MiB | `55356645c2b361a969dfd0ef2c5a50d530afd8d5` |

The URLs are fixed HTTPS catalog constants in Rust. The source is the official
`ggerganov/whisper.cpp` Hugging Face repository, whose model README publishes
these SHA values. The implementation must use a cryptographic checksum that is
available in the Rust dependency set for downloaded-file verification; the
catalog's published SHA-1 values are the upstream identity values and must also
be represented. If the implementation adds SHA-256 catalog values, they must be
recorded as explicit constants generated from the exact files, never computed
from an unverified download and then trusted.

Models are multilingual because the selected files are the non-`.en` variants.
No arbitrary model URL, custom URL, or binary download is in scope.

## Shared Voice component

Create one reusable React component, used by both onboarding and Settings:

```text
VoiceSetup
├── provider choice: Deepgram | Whisper local
├── Deepgram configuration
│   ├── masked key status
│   ├── password key input
│   └── model selector/input
└── Whisper configuration
    ├── whisper-cli detection status
    ├── curated model cards
    ├── download/progress/cancel controls
    ├── installed model selection
    └── remove model action
```

### Deepgram

- Render only the masked key status returned by the existing keystore command.
- Password input is cleared after a successful save.
- Save/remove uses the existing `keystore_set_key`/`keystore_remove_key`
  contract and keeps the key in `keys.json`, never `config.toml`.
- Model selection writes `models.stt_model` through `config_set`.
- A missing key is a setup state, not an onboarding blocker.

### Whisper

Each catalog card shows:

- Tiny/Base/Small name and quality/latency description.
- Approximate size.
- `Hugging Face · ggerganov/whisper.cpp` source label.
- Installed/downloaded state.
- Selected state.
- Download progress while active.

Actions are `Download`, `Cancel`, `Use this model`, and `Remove`. The UI must
not offer a binary download; it explains that users install `whisper-cli`
separately. Downloaded files are not automatically selected.

## Rust command and event contract

Add typed commands:

```text
voice_models_catalog()
whisper_status()
whisper_download(model)
whisper_cancel_download()
whisper_remove_model(model)
```

`model` is a catalog ID (`tiny`, `base`, or `small`), not a URL or arbitrary
filename. Existing `whisper_status` is extended rather than duplicated.

Catalog response:

```json
[
  {
    "id": "base",
    "filename": "ggml-base.bin",
    "label": "Base",
    "description": "Balanced quality and download size",
    "bytes": 148000000,
    "source": "Hugging Face · ggerganov/whisper.cpp"
  }
]
```

Status response:

```json
{
  "binary": "/path/to/whisper-cli",
  "models": [
    {
      "id": "base",
      "filename": "ggml-base.bin",
      "installed": true,
      "bytes": 148000000
    }
  ],
  "download": {
    "model": "small",
    "received": 123456,
    "total": 488000000
  }
}
```

`download` is `null` when idle. Paths may be returned only in the existing
local Settings status surface; they never enter Listen events or external
provider requests. Download progress events contain only model ID and byte
counts:

```text
whisper:download-progress
{ "model": "small", "received": 123456, "total": 488000000 }
```

The final status command is authoritative after completion, cancellation, or
failure. A failed download emits a sanitized `whisper:download-error` message
without URLs containing credentials, filesystem secrets, or response bodies.

## Download lifecycle and safety

1. Validate the model ID against the fixed Rust catalog.
2. Reject a second concurrent download.
3. Create `~/.marvis/models/whisper/models/` with existing Marvis path/mode
   conventions.
4. Stream the fixed HTTPS Hugging Face response through the existing Rust HTTP
   stack.
5. Write `<filename>.tmp` with mode `0600`.
6. Emit throttled progress updates; do not flood the webview per network chunk.
7. Verify the complete file against the catalog's upstream identity/checksum
   contract and expected size bounds before installation.
8. Atomically rename the verified temporary file to the final filename.
9. Refresh and emit `whisper_status`.
10. Delete the temporary file on cancellation, network failure, checksum/size
    failure, or process exit.

Existing final model files must remain usable if a replacement download fails.
The app must never report a model as installed until verification and atomic
rename succeed. Download cancellation must not remove an existing final model.

The Whisper CLI resolution remains:

1. `PATH`
2. `/opt/homebrew/bin/whisper-cli`
3. `~/.marvis/models/whisper/bin/whisper-cli`

## Configuration and Listen behavior

- `models.stt_provider` remains `deepgram` or `whisper`.
- `models.stt_model` remains the Deepgram model ID or Whisper catalog ID/
  filename, with Rust validating that it maps to a valid configured model.
- Selecting a Whisper model before it is installed is rejected or remains an
  explicitly unavailable selection; it must not silently start Listen.
- If `whisper-cli` is missing, model downloads remain possible, but Listen
  reports that the binary must be installed separately.
- If a selected model is removed externally, `whisper_status` reports it as
  unavailable and Listen shows Settings guidance.
- Existing durable Listen setup errors and cold-open resync behavior remain.
- Deepgram keys remain masked and stored only through `keys.json`.

## Error handling

- Invalid model IDs are rejected before filesystem/network access.
- Only one download runs at a time; a second request returns a safe busy error.
- HTTP, disk, cancellation, checksum, and size errors leave no temporary
  partial model and do not damage an existing model.
- Network/provider error messages are sanitized before events/UI.
- `whisper_cancel_download` is idempotent when no download is active.
- Removing an installed model updates status; removing the active model is
  rejected until the user changes selection/provider.
- Closing Settings does not cancel a download. Onboarding leaving the Voice
  step cancels its in-progress download to avoid an invisible background task;
  this behavior is explicit and differs from Settings.

## Testing

Rust tests:

- Catalog has exactly `tiny`, `base`, and `small` with HTTPS URLs pointing to
  `huggingface.co/ggerganov/whisper.cpp`.
- Unknown IDs and arbitrary URL/filename input are rejected.
- Catalog entries expose the expected filenames and upstream checksums.
- Temporary files use mode `0600`.
- Progress payloads contain only model ID and byte counts.
- Checksum/size mismatch removes the temporary file.
- Cancellation removes the temporary file.
- Successful download atomically installs the model.
- Existing final files remain unchanged after failed replacement.
- `whisper_status` reports installed models/download state.
- Active-model removal is rejected.
- `config_set` accepts only valid provider/model combinations.
- Setup errors remain resynchronizable after a cold-open failure.

Frontend/build verification:

```text
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo check
bun x tsc --noEmit
bun run build
bun test --pass-with-no-tests
```

Manual verification:

1. Complete onboarding while choosing `Skip for now`.
2. Open Settings → Voice and configure Deepgram.
3. Open Settings → Voice and download Tiny from Hugging Face.
4. Cancel Base during download and verify no partial final model remains.
5. Select Tiny and start Listen with `whisper-cli` installed.
6. Reopen Settings during a download and verify progress/status resync.
7. Delete the selected model externally and verify setup guidance.
8. Confirm no API key, PCM data, or local audio path appears in webview
   events/logs.
9. Verify closing Settings does not cancel the download, while leaving the
   onboarding Voice step cancels its download.

## Out of scope

- Downloading/installing the `whisper-cli` binary.
- Arbitrary model URLs or user-provided Hugging Face repositories.
- Whisper `.en`, quantized, medium, large, or custom models.
- Background download management across app restarts.
- Automatic model selection after download.
- Voice becoming required for onboarding completion.
- Changes to Deepgram streaming/reconnect behavior beyond configuration.
