# Marvis — sherpa-onnx Local STT Engine Design

Date: 2026-09-23
Status: Approved design (pending written-spec review)
Branch: `feature/implement_voice_listening_part`
Related specs: `docs/superpowers/specs/2026-09-23-bundled-whisper-cli-design.md`, `docs/superpowers/specs/2026-09-23-dictation-mode-design.md`, `docs/superpowers/specs/2026-09-23-voice-setup-model-download-design.md`

## Goal

Add `"sherpa"` as a third STT engine alongside the existing `whisper` (local
sidecar) and `deepgram` (hosted) providers. Sherpa runs **in-process** via the
official `sherpa-onnx` Rust crate (ONNX Runtime, statically linked) — no
sidecar binary, no `externalBin` changes, nothing extra to sign or notarize.
The engine serves **both** meeting Listen (mic + system audio) and Ask-input
Dictation (mic only) through the existing `SttProvider` trait. Whisper is not
replaced; it remains a fully working alternative.

## User decisions

| Question | Decision |
| --- | --- |
| Integration mode | In-process `sherpa-onnx` crate, static-linked (auto-downloads pinned prebuilt `-lib` archive at build time). No sidecar process. |
| ASR model | SenseVoice int8 only (`zh/en/ja/ko/yue` + punctuation), segmented by silero VAD. Streaming zipformer deferred. |
| TTS | Deferred — no TTS surface exists in the app yet; `OfflineTts` can be added later. |
| Whisper | Untouched — remains a selectable provider. |
| Events | `Final`-only `TranscriptEvent`s (SenseVoice is offline) — same UX contract as whisper, segmented on real speech boundaries instead of fixed 3 s windows. |

## Why sherpa-onnx (research summary)

- Full speech toolkit (offline/streaming ASR, TTS, VAD, diarization) vs
  VibeASR.cpp's ASR-only scope; TTS path stays open.
- Explicit zh+en model zoo: SenseVoice int8 ≈ 244 MB model file, ~450 MB RSS —
  fits the 4 GB laptop budget; VibeASR's ~1.58 GB model does not comfortably.
- Official `sherpa-onnx` crate on crates.io (maintained by the sherpa-onnx
  author); `OfflineRecognizer`, `VoiceActivityDetector`, `OfflineTts` are
  `Send`+`Sync` RAII wrappers over the C API.
- Prebuilt static libs for macOS arm64/x86_64 (and win/linux) each release;
  the crate build script fetches the matching archive — no vendored binaries.
- Best measured Chinese accuracy of the compared engines; `language="auto"`
  handles zh/en code-switching.

## Current-state context

- `stt::SttProvider` trait: `start(callback, error_callback)`,
  `enqueue(PcmChunk)`, `stop()`. Providers emit `TranscriptEvent { channel,
  text, finality }`.
- `make_stt_provider(provider, key, model, channel, bundled_whisper)` in
  `stt/mod.rs` matches on `"deepgram"` / `"whisper"`.
- `whisper_setup_error(status, model) -> Option<&'static str>` (curated
  messages, no paths) gates `listen.rs` (`provider_name == "whisper"`) and
  `dictation.rs` identically before `make_stt_provider`.
- `VoiceModelManager` (`voice_models.rs`) downloads pinned catalog files from
  Hugging Face into `~/.marvis/models/whisper/models/` with sha1+size
  verification, tmp+rename install, cancellation token, install gate, and
  `whisper:download-progress` / `whisper:download-error` events.
- `config.rs` validates `models.stt_provider` against `{deepgram, whisper}`
  and `models.stt_model` per provider; switching providers resets the model
  to a catalog default.
- `VoiceSetup.tsx` renders the provider `<select>` plus a whisper model-card
  grid (download/cancel/select/remove) driven by `whisper_*` commands.
- Dictation (`dictation.rs`) uses `MicSource` + `SpeakerChannel::Me` +
  `DraftAssembler`; Listen (`listen.rs`) uses `MicSource` +
  `SystemAudioSource` + `TurnAssembler`. Both consume the same provider
  contract — sherpa slots into both without either service caring.

## Architecture

```text
MicSource / SystemAudioSource ──► PcmChunk (i16, 16 kHz)
        │
        ▼  enqueue (per provider instance, per channel)
SherpaProvider worker thread
        │  i16 → f32 (/32768)
        ▼
VoiceActivityDetector (silero, per channel — VAD state is per-stream)
        │  completed SpeechSegment (min_silence 0.3 s … max_speech 20 s)
        ▼  job { samples, reply } over mpsc
SherpaEngine thread (ONE per model dir, app-wide)
        │  OfflineRecognizer::create(config) — SenseVoice int8
        │  create_stream → accept_waveform → decode → get_result().text
        ▼
callback(TranscriptEvent { channel, text, finality: Final })
        │
        ▼
DraftAssembler (dictation) / TurnAssembler (listen)
```

**Shared engine.** `ListenService` creates two providers (Me + Them). A
per-provider `OfflineRecognizer` would double the ~450 MB working set; instead
one `SherpaEngine` owns the recognizer on a dedicated thread and providers
submit `Job { samples, reply }` work items. A global single-slot registry
(`engine_for(model_dir)`) hands out `Arc<SherpaEngine>`; a different model dir
replaces the entry (dropping the old recognizer frees its RSS on model
switch). The engine is created eagerly inside `SherpaProvider::start` via an
init oneshot, so model-load failure surfaces synchronously as a setup error.

**VAD per channel.** Each provider worker owns its own
`VoiceActivityDetector` (silero_vad.onnx ~2 MB, num_threads 1) — VAD state is
per-stream and must not be shared. On `stop()`, `vad.flush()` drains the
trailing speech segment so the last words are not lost.

**No interim results.** SenseVoice is an offline model; every completed VAD
segment yields one `Final` event. Latency is bounded by `max_speech_duration`
(20 s) + decode time (~0.3–0.4× RTF), typically ≪ whisper's fixed window +
process spawn.

## Components

| Piece | Location | Notes |
| --- | --- | --- |
| Dependency | `apps/native/src-tauri/Cargo.toml` | `sherpa-onnx = "1.13.8"` (static default; `SHERPA_ONNX_LIB_DIR` env override for offline/CI builds) |
| Paths | `paths.rs` | `sherpa_dir()` → `~/.marvis/models/sherpa`; `sherpa_models_dir()` → `…/sherpa/models` |
| Catalog + manager | `sherpa_models.rs` (new) | `SherpaCatalogEntry { id, dirname, label, description, files: &[SherpaFileSpec], source }`; per-entry dir `models/sherpa/<dirname>/`; sha256-pinned files, tmp+rename, cancel token, install gate, `sherpa:download-*` events |
| Provider | `stt/sherpa.rs` (new) | `SherpaProvider`, shared `SherpaEngine`, VAD loop, `sherpa_setup_error` |
| Provider wiring | `stt/mod.rs` | `mod sherpa`, `"sherpa"` arm in `make_stt_provider`, re-export |
| Config | `config.rs` | `validate_stt_provider` += `"sherpa"`; model default `"sense-voice"` on provider switch; `validate_sherpa_model` catalog check |
| Services | `dictation.rs`, `listen.rs` | `provider_name == "sherpa"` → `sherpa_setup_error` arm (same shape as whisper's) |
| Commands | `lib.rs` | `sherpa_status`, `sherpa_download`, `sherpa_cancel_download`, `sherpa_remove_model`; `AppState.sherpa_models`; handler registration; `for_test` update |
| Frontend | `commands.ts`, `events.ts`, `VoiceSetup.tsx` | `sherpa*` wrappers + event constants + payload types; provider option "Sherpa (local)"; shared model-card grid extracted for both engines |

## Model catalog

One entry, `id = "sense-voice"`, `dirname = "sense-voice"`:

| File | Source | Size |
| --- | --- | --- |
| `model.int8.onnx` | `huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09` | ~244 MB |
| `tokens.txt` | same HF repo | ~0.3 MB |
| `silero_vad.onnx` | `github.com/k2-fsa/sherpa-onnx` release `asr-models` | ~2.3 MB |

Each file carries a pinned `url`, display `bytes`, and `sha256` (digests
computed once during implementation and pasted into the catalog — same pattern
as the whisper sha1 pins). Installed = all entry files present; remove deletes
the entry dir. Download progress aggregates across the file set.

## Commands and events

```text
sherpa_status          -> SherpaStatus { models: [{id,label,description,bytes,source,installed}], download }
sherpa_download        -> () | error      (model: catalog id)
sherpa_cancel_download -> () | error
sherpa_remove_model    -> SherpaStatus    (rejects removal of the selected model)

sherpa:download-progress -> { model, received, total }
sherpa:download-error    -> { model, message }
```

Unlike whisper there is no binary-status field — the engine is statically
linked into the app. `sherpa_remove_model` applies the same ActiveModel
protection via `set_selected_model` when `stt_provider == "sherpa"`.

## sherpa_setup_error

`pub(crate) fn sherpa_setup_error(model: &str) -> Option<&'static str>`:

- `entry_for_value(model)` unknown → `"configured Sherpa model was not found; choose an installed model in Settings"`
- entry files missing → `"the SenseVoice model is not downloaded; download it in Settings"`
- else `None`

Curated messages only — no paths, URLs, or engine internals in events, matching
the whisper contract. `DictationService::start` and `ListenService::start`
gain a `provider_name == "sherpa"` arm calling it before `make_stt_provider`.

## VAD / recognizer configuration

```text
VAD (silero):   threshold 0.5 · min_silence 0.3 s · min_speech 0.25 s
                max_speech 20 s · window 512 · num_threads 1
Recognizer:     sense_voice { model, language "auto", use_itn true }
                tokens.txt · num_threads 2 · sample_rate 16000
```

`use_itn` gives punctuation + inverse text normalization ("50%" not
"百分之五十") — right for dictation. `language "auto"` covers zh/en
code-switching.

## Error handling

- Recognizer/VAD create failure → `start()` returns `Err` synchronously →
  service `fail()` path emits sanitized `*:error` with `needs_setup` where
  appropriate.
- Decode failure on the engine thread → `error_callback` once (terminal;
  worker exits) — mirrors deepgram's terminal-error semantics.
- Provider errors are sanitized through `sanitize_provider_error` before any
  event — no model paths or ONNX internals reach the frontend.
- `stop()` sets cancel + joins the worker; the engine thread itself stays
  warm for process lifetime (fast restart; ~450 MB RSS tradeoff noted).

## Security / privacy

- Fully offline at inference time — no network after model download.
- Model downloads are pinned-URL + sha256-verified, same trust model as the
  whisper catalog.
- Events carry curated strings only.
- Licenses: sherpa-onnx Apache-2.0, SenseVoice MIT, silero VAD MIT — all
  permit bundling/downloading.

## Testing

- **Unit (Rust):** catalog shape + file specs (https URLs, 64-hex sha256);
  `entry_for_value` rejects paths/URLs; `sherpa_setup_error` per state;
  config validation + provider-switch default; `SherpaProvider` satisfies
  `SttProvider: Send`; segment→event mapping (empty text filtered, channel
  stamped, `Final` finality) via an injected decode seam.
- **Command contract:** extend the `lib.rs` source tests so `sherpa_*`
  commands must be registered in `invoke_handler`.
- **Frontend:** `bun x tsc --noEmit`, `bun test`.
- **Manual:** download SenseVoice in Settings → select Sherpa → dictate zh +
  en into the Ask input → run Listen on mic + system audio → confirm `me`/
  `them` turns → check RSS (~+500 MB) → confirm whisper + deepgram still work.

## Known risks

1. First `cargo build` needs network for the ~17 MB static-lib fetch;
   `SHERPA_ONNX_LIB_DIR` supports offline CI.
2. App binary grows ~15–25 MB (static onnxruntime).
3. Engine stays warm once loaded — ~450–500 MB resident after first use until
   quit. Acceptable on 4 GB but worth noting; dropping on `stop()` is a
   possible follow-up.
4. SenseVoice is offline-only → no interim partials; a streaming zipformer
   catalog entry is a future addition that needs no provider-contract change.
5. sherpa-onnx version pinning: `=1.13.8` locks the sys crate to its matching
   `-lib` archive; upgrades are deliberate.
