# sherpa-onnx Local STT Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `"sherpa"` as a third STT engine — an in-process sherpa-onnx (SenseVoice int8 + silero VAD) provider usable by both meeting Listen and Ask-input Dictation, with a downloadable model catalog and Settings UI — while whisper and deepgram keep working unchanged.

**Architecture:** The official `sherpa-onnx` Rust crate is statically linked into the app (its build script downloads the pinned `-lib` archive; no sidecar, no `externalBin`). `SherpaProvider` implements `SttProvider`: each provider's worker thread owns a silero `VoiceActivityDetector` and submits completed speech segments to one app-wide `SherpaEngine` (single `OfflineRecognizer` behind a job queue) so Listen's two channels share ~450 MB RSS. The provider emits `Final`-only `TranscriptEvent`s at VAD boundaries. Models are a sha256-pinned multi-file catalog downloaded into `~/.marvis/models/sherpa/<dirname>/` by a `SherpaModelManager` mirroring `VoiceModelManager`.

**Tech Stack:** Rust, sherpa-onnx crate 1.13.8 (static), silero VAD, SenseVoice int8, Tauri 2 commands/events, React 19, TypeScript, Bun.

**Approved spec:** `docs/superpowers/specs/2026-09-23-sherpa-onnx-stt-design.md`

## Global Constraints

- Whisper stays a fully working provider — do not modify `stt/whisper.rs`, the whisper catalog, or `whisper_*` commands beyond what a task explicitly shows.
- Dictation is mic-only (`MicSource`, `SpeakerChannel::Me`) and transient — sherpa must not add persistence. Listen covers mic + `SystemAudioSource` (`Me` + `Them`).
- Events must never expose binary paths, provider keys, model URLs, raw audio, or ONNX internals — errors go through `sanitize_provider_error` / curated `&'static str` messages.
- sherpa-onnx emits `Final` events only — never synthesize `Interim` results.
- React components/hooks use arrow-function syntax; named exports for `src/components/*`; `Icon`-suffixed `lucide-react` names only.
- Follow existing conventions: `parking_lot::Mutex`, `mpsc` channels, `tauri::async_runtime`, compact `| --- |` markdown tables.

---

## File Map

### Rust files to create or modify

- Create: `apps/native/src-tauri/src/sherpa_models.rs` — SenseVoice file-set catalog, `SherpaModelManager` (download/cancel/remove/status), `sherpa:download-*` events.
- Create: `apps/native/src-tauri/src/stt/sherpa.rs` — `SherpaEngine` (shared recognizer), `SherpaProvider` (VAD worker), `sherpa_setup_error` support.
- Modify: `apps/native/src-tauri/Cargo.toml` — `sherpa-onnx = "=1.13.8"`, `sha2 = "0.10"`.
- Modify: `apps/native/src-tauri/src/paths.rs` — `sherpa_dir()`, `sherpa_models_dir()`.
- Modify: `apps/native/src-tauri/src/stt/mod.rs` — `mod sherpa`, `"sherpa"` arm in `make_stt_provider`, `sherpa_setup_error`, re-exports.
- Modify: `apps/native/src-tauri/src/config.rs` — accept `"sherpa"` provider + model validation + provider-switch default.
- Modify: `apps/native/src-tauri/src/dictation.rs` — `provider_name == "sherpa"` setup arm.
- Modify: `apps/native/src-tauri/src/listen.rs` — `provider_name == "sherpa"` setup arm.
- Modify: `apps/native/src-tauri/src/lib.rs` — `mod sherpa_models`, `AppState.sherpa_models`, four `sherpa_*` commands, handler registration, `for_test`.

### Frontend files to create or modify

- Create: `apps/native/src/components/prefs/VoiceModelGrid.tsx` — shared model-card grid (download/cancel/select/remove) used by the whisper and sherpa sections.
- Modify: `apps/native/src/lib/commands.ts` — `SherpaStatus`/`SherpaInstalledModel` types + four wrappers.
- Modify: `apps/native/src/lib/events.ts` — `sherpa:download-*` constants + payload types.
- Modify: `apps/native/src/components/prefs/VoiceSetup.tsx` — "Sherpa (local)" option, sherpa section, shared-grid extraction, unmount-cancel covers sherpa.

---

## Task 1: Add the sherpa-onnx dependency and sherpa model paths

**Files:**

- Modify: `apps/native/src-tauri/Cargo.toml`
- Modify: `apps/native/src-tauri/src/paths.rs`
- Test: `paths.rs` unit tests

- [ ] **Step 1: Write the failing path test.** In `paths.rs` `mod tests`, extend `files_live_under_marvis_root`:

```rust
        let sherpa = root.join("models").join("sherpa");
        let sherpa_models = sherpa.join("models");
        let existed = [sherpa.exists(), sherpa_models.exists()];
        assert_eq!(sherpa_dir(), sherpa);
        assert_eq!(sherpa_models_dir(), sherpa_models);
        assert_eq!([sherpa.exists(), sherpa_models.exists()], existed);
```

- [ ] **Step 2: Run it to verify failure.**

```bash
cd apps/native/src-tauri
cargo test paths:: -- --nocapture
```

Expected: FAIL — `sherpa_dir`/`sherpa_models_dir` do not exist.

- [ ] **Step 3: Add the helpers** in `paths.rs` after `whisper_models_dir`:

```rust
/// `~/.marvis/models/sherpa` — path only; callers create it when needed.
pub fn sherpa_dir() -> PathBuf {
    models_dir().join("sherpa")
}

/// `~/.marvis/models/sherpa/models` — path only; callers create it when needed.
pub fn sherpa_models_dir() -> PathBuf {
    sherpa_dir().join("models")
}
```

- [ ] **Step 4: Add the dependencies** in `apps/native/src-tauri/Cargo.toml` next to the existing `sha1`/`reqwest` lines:

```toml
sha2 = "0.10"
sherpa-onnx = "=1.13.8"
```

The `=1.13.8` pin locks `sherpa-onnx-sys` to its matching prebuilt `-lib`
archive. The build script downloads `sherpa-onnx-v<ver>-<target>-static-lib.tar.bz2`
from GitHub releases on first build (needs network once); set
`SHERPA_ONNX_LIB_DIR` to a pre-downloaded archive dir for offline/CI builds.

- [ ] **Step 5: Run the focused tests** — this also proves the static lib downloads and links:

```bash
cargo test paths:: -- --nocapture
```

Expected: PASS. (First run spends ~1 min fetching the static archive.)

- [ ] **Step 6: Commit.**

```bash
git add apps/native/src-tauri/Cargo.toml apps/native/src-tauri/Cargo.lock apps/native/src-tauri/src/paths.rs
git commit -m "feat(native): add sherpa-onnx dependency and model paths"
```

---

## Task 2: sherpa model catalog + download manager

**Files:**

- Create: `apps/native/src-tauri/src/sherpa_models.rs`
- Modify: `apps/native/src-tauri/src/lib.rs` — add `mod sherpa_models;` only (commands land in Task 5)
- Test: `sherpa_models.rs` unit tests

**Interfaces (produced — later tasks rely on these):**

```rust
pub enum SherpaModelId { SenseVoice }          // as_str() -> "sense-voice"
pub struct SherpaFileSpec { filename, url, bytes, sha256 }
pub struct SherpaCatalogEntry { id, dirname, label, description, files, source }
pub fn catalog() -> &'static [SherpaCatalogEntry]
pub fn entry_for_id(&str) -> Option<&'static SherpaCatalogEntry>
pub fn entry_for_value(&str) -> Option<&'static SherpaCatalogEntry>  // id or dirname
pub fn entry_dir(root: &Path, entry: &SherpaCatalogEntry) -> PathBuf
pub fn entry_installed_at(root: &Path, entry: &SherpaCatalogEntry) -> bool
pub struct SherpaInstalledModel { id, label, description, bytes, source, installed }
pub struct SherpaStatus { models: Vec<SherpaInstalledModel>, download: Option<SherpaDownloadProgress> }
pub struct SherpaDownloadProgress { model, received, total }
pub struct SherpaModelManager { new(), at(root), attach_app(AppHandle),
    status(), start_download(SherpaModelId), cancel_download() async,
    set_selected_model(Option<SherpaModelId>), remove_model(SherpaModelId) }
```

Errors reuse `crate::voice_models::VoiceDownloadError`.

- [ ] **Step 1: Write the failing catalog tests.** Create `sherpa_models.rs` with the test module only plus `mod sherpa_models;` in `lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_the_approved_sense_voice_file_set() {
        assert_eq!(catalog().len(), 1);
        let entry = &catalog()[0];
        assert_eq!(entry.id.as_str(), "sense-voice");
        assert_eq!(entry.dirname, "sense-voice");
        assert_eq!(
            entry.files.iter().map(|f| f.filename).collect::<Vec<_>>(),
            ["model.int8.onnx", "tokens.txt", "silero_vad.onnx"]
        );
        assert!(entry
            .files
            .iter()
            .all(|f| f.url.starts_with("https://") && f.sha256.len() == 64
                && f.sha256.chars().all(|c| c.is_ascii_hexdigit())));
    }

    #[test]
    fn lookup_rejects_arbitrary_ids_urls_and_paths() {
        assert_eq!(entry_for_id(" SenseVoice ").unwrap().id, SherpaModelId::SenseVoice);
        for value in [
            "https://example.com/model.onnx",
            "../sense-voice",
            "nested/model.int8.onnx",
            "tiny",
        ] {
            assert!(entry_for_value(value).is_none());
        }
    }

    #[test]
    fn installed_requires_every_file() {
        let root = temp_root();
        let entry = &catalog()[0];
        assert!(!entry_installed_at(&root, entry));
        let dir = entry_dir(&root, entry);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("model.int8.onnx"), b"x").unwrap();
        fs::write(dir.join("tokens.txt"), b"x").unwrap();
        assert!(!entry_installed_at(&root, entry)); // silero still missing
        fs::write(dir.join("silero_vad.onnx"), b"x").unwrap();
        assert!(entry_installed_at(&root, entry));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn status_dto_has_only_safe_public_fields() {
        let status = serde_json::to_value(SherpaStatus {
            models: vec![SherpaInstalledModel {
                id: "sense-voice",
                label: "SenseVoice",
                description: "d",
                bytes: 1,
                source: CATALOG_SOURCE,
                installed: true,
            }],
            download: None,
        })
        .unwrap();
        assert!(status.get("binary").is_none());
        assert_eq!(status["models"][0]["id"], "sense-voice");
        assert!(status["models"][0].get("url").is_none());
        assert!(status["models"][0].get("sha256").is_none());
    }
}
```

Plus the manager tests, ported from `voice_models.rs` lines 680–904 — one
`TcpListener` `fixture` per file (each file gets its own one-shot server since
files download sequentially), `test_files: Option<Vec<TestSource>>` overriding
`url`/`bytes`/`sha256` per index. Port these tests: `sync_start_download_uses_the_tauri_runtime`,
`lifecycle_verifies_cleans_preserves_and_serializes_downloads` (bad-sha256
preserves an existing file; bad-size aborts), `startup_reclaims_only_catalog_temps`
(inside `root/sense-voice/`), `active_model_removal_is_rejected`,
`removal_is_rejected_while_download_is_active`,
`cancellation_racing_final_install_has_one_authoritative_outcome`,
`cancellation_does_not_remove_preexisting_temp_file`. Assert installed state
via `entry_installed_at(&root, &catalog()[0])` — success requires ALL three
files renamed.

- [ ] **Step 2: Run to verify failure.**

```bash
cd apps/native/src-tauri
cargo test sherpa_models -- --nocapture
```

Expected: FAIL — module is test-only.

- [ ] **Step 3: Implement `sherpa_models.rs`.** Port `voice_models.rs` to a
  file-set model. Key deltas (write the full file; the whisper module is the
  template):

```rust
pub const CATALOG_SOURCE: &str = "Hugging Face · csukuangfj + k2-fsa/sherpa-onnx";

const SENSE_VOICE_FILES: &[SherpaFileSpec] = &[
    SherpaFileSpec {
        filename: "model.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09/resolve/main/model.int8.onnx",
        bytes: 237_115_547,
        sha256: "12ca1a2ae7ecf3e0019ef2822307ee0b5cadc9196569e379b4c4026f8205276d",
    },
    SherpaFileSpec {
        filename: "tokens.txt",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09/resolve/main/tokens.txt",
        bytes: 315_894,
        sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
    },
    SherpaFileSpec {
        filename: "silero_vad.onnx",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx",
        bytes: 643_854,
        sha256: "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6",
    },
];
```

Manager internals: `state { active: Option<ActiveDownload>, selected }`,
`cancel_gate`, `install_gate`, `app`, `#[cfg(test)] test_files:
Option<Vec<TestSource>>`. `start_download` spawns one task that downloads the
entry's files **sequentially** into `root/<dirname>/` — per file the whisper
`download_inner` flow (tmp `<filename>.tmp`, sha256 + ±10% size verify,
install gate, rename); progress events emit aggregate
`{ model: entry.id, received: completed + current, total: Σ entry.files.bytes }`.
`entry_installed_at` = all files present. `remove_model` = `remove_dir_all`
on the entry dir (NotFound → Ok; `Busy`/`ActiveModel` guards identical to
whisper). `reclaim_catalog_temps` removes only `root/<dirname>/<filename>.tmp`
for catalog files. Events: `sherpa:download-progress`, `sherpa:download-error`
with the same payload shapes as whisper's.

- [ ] **Step 4: Run focused tests.**

```bash
cargo test sherpa_models -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add apps/native/src-tauri/src/sherpa_models.rs apps/native/src-tauri/src/lib.rs
git commit -m "feat(native): add sherpa model catalog and download manager"
```

---

## Task 3: SherpaProvider — shared engine + VAD worker

**Files:**

- Create: `apps/native/src-tauri/src/stt/sherpa.rs`
- Modify: `apps/native/src-tauri/src/stt/mod.rs`
- Test: `stt/sherpa.rs` + `stt/mod.rs` unit tests

**Interfaces (produced):**

```rust
pub struct SherpaProvider — impl SttProvider; SherpaProvider::new(model: &str, channel: SpeakerChannel)
pub(crate) fn sherpa_setup_error(model: &str) -> Option<&'static str>   // in stt/mod.rs
// make_stt_provider gains: "sherpa" => SherpaProvider::new(model, channel)
```

- [ ] **Step 1: Write the failing tests.** In `stt/mod.rs` tests add:

```rust
    #[test]
    fn sherpa_provider_satisfies_the_stt_contract() {
        fn assert_provider<T: SttProvider>() {}
        assert_provider::<SherpaProvider>();
    }
```

In `stt/sherpa.rs` tests:

```rust
    #[test]
    fn emit_segment_maps_clean_text_to_a_final_event() {
        let events = std::sync::Mutex::new(Vec::new());
        let errors = std::sync::Mutex::new(Vec::new());
        // Decode seam: emit_segment takes a decode closure so the VAD→event
        // mapping is testable without model files.
        let ok = emit_segment(
            &[0.0f32; 4],
            |_| Ok("  你好 world  ".to_string()),
            SpeakerChannel::Me,
            &|e| events.lock().unwrap().push(e),
            &|m| errors.lock().unwrap().push(m),
        );
        assert!(ok);
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].text, "你好 world");
        assert_eq!(events[0].finality, Finality::Final);
        assert_eq!(events[0].channel, SpeakerChannel::Me);
    }

    #[test]
    fn emit_segment_skips_blank_text_and_surfaces_decode_failure() {
        let events = std::sync::Mutex::new(Vec::new());
        let errors = std::sync::Mutex::new(Vec::new());
        // Blank decode text → ok, but no event.
        assert!(emit_segment(
            &[0.0f32; 4],
            |_| Ok("   ".to_string()),
            SpeakerChannel::Me,
            &|e| events.lock().unwrap().push(e),
            &|m| errors.lock().unwrap().push(m),
        ));
        assert!(events.lock().unwrap().is_empty());
        // Decode failure → one error_callback call, false (terminal).
        assert!(!emit_segment(
            &[0.0f32; 4],
            |_| Err("decode failed".to_string()),
            SpeakerChannel::Me,
            &|e| events.lock().unwrap().push(e),
            &|m| errors.lock().unwrap().push(m),
        ));
        assert_eq!(errors.lock().unwrap().len(), 1);
    }

    #[test]
    fn provider_reports_not_enqueued_before_start_and_stop_is_idempotent() {
        let mut p = SherpaProvider::new("sense-voice", SpeakerChannel::Me);
        assert!(!p.enqueue(PcmChunk { samples: vec![1], sample_rate: 16_000, channels: 1 }));
        p.stop(); // safe before start
    }

    #[test]
    fn start_fails_when_the_model_is_missing() {
        let mut p = SherpaProvider::new("sense-voice", SpeakerChannel::Me);
        // No model files on disk → engine init fails synchronously.
        assert!(p.start(Box::new(|_| {}), Box::new(|_| {})).is_err());
    }
```

`emit_segment` signature (the testable seam — the worker passes
`|s| engine.decode(s.to_vec())` as `decode`):

```rust
fn emit_segment(
    samples: &[f32],
    decode: impl Fn(&[f32]) -> Result<String, String>,
    channel: SpeakerChannel,
    callback: &(dyn Fn(TranscriptEvent) + Send + Sync),
    error_callback: &(dyn Fn(String) + Send + Sync),
) -> bool {
    match decode(samples) {
        Ok(text) => {
            let text = text.trim();
            if !text.is_empty() {
                callback(TranscriptEvent {
                    channel,
                    text: text.to_string(),
                    finality: Finality::Final,
                });
            }
            true
        }
        Err(message) => {
            error_callback(message);
            false
        }
    }
}
```

And `sherpa_setup_error` tests in `stt/mod.rs` (or `sherpa.rs`) covering:
unknown model → "not found" message; entry files absent → "not downloaded"
message; all files present in a temp root → `None` (use a
`sherpa_setup_error_at(root, model)` sibling taking the models dir).

- [ ] **Step 2: Run to verify failure.**

```bash
cargo test sherpa -- --nocapture
```

Expected: FAIL.

- [ ] **Step 3: Implement `stt/sherpa.rs`.** Core structure:

```rust
const SAMPLE_RATE: i32 = 16_000;
const WORKER_TICK: Duration = Duration::from_millis(100);
const VAD_BUFFER_SECONDS: f32 = 30.0;
const DECODE_TIMEOUT: Duration = Duration::from_secs(30);
const QUEUE_DEPTH: usize = 8;

struct DecodeJob { samples: Vec<f32>, reply: mpsc::Sender<Result<String, String>> }

/// One recognizer for the whole app — Listen runs two providers (me + them)
/// and a per-channel model would double the ~450MB working set.
pub struct SherpaEngine { jobs: mpsc::Sender<DecodeJob> }

static ENGINE: Mutex<Option<(PathBuf, Arc<SherpaEngine>)>> = Mutex::new(None);

fn engine_for(model_dir: &Path) -> Result<Arc<SherpaEngine>, String> {
    let mut slot = ENGINE.lock();
    if let Some((dir, engine)) = &*slot {
        if dir == model_dir {
            return Ok(engine.clone());
        }
    }
    let engine = Arc::new(SherpaEngine::spawn(model_dir)?);
    *slot = Some((model_dir.to_path_buf(), engine.clone()));
    Ok(engine)
}
```

`SherpaEngine::spawn`: spawn a thread that builds the recognizer via
`create_recognizer`, reports `Result<(), String>` on an init channel, then
loops `rx.recv()` → `create_stream` → `accept_waveform(SAMPLE_RATE, &samples)`
→ `decode` → `get_result().text`. `decode()` sends the job and
`reply.recv_timeout(DECODE_TIMEOUT)`s.

```rust
fn create_recognizer(model_dir: &Path) -> Result<OfflineRecognizer, String> {
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model_dir.join("model.int8.onnx").display().to_string()),
        language: Some("auto".into()),
        use_itn: true,
    };
    config.model_config.tokens = Some(model_dir.join("tokens.txt").display().to_string());
    config.model_config.num_threads = 2;
    OfflineRecognizer::create(&config)
        .ok_or_else(|| "the speech model could not be loaded".to_string())
}

fn vad_config(model_dir: &Path) -> VadModelConfig {
    let mut config = VadModelConfig::default();
    config.silero_vad = SileroVadModelConfig {
        model: Some(model_dir.join("silero_vad.onnx").display().to_string()),
        threshold: 0.5,
        min_silence_duration: 0.3,
        min_speech_duration: 0.25,
        window_size: 512,
        max_speech_duration: 20.0,
    };
    config.sample_rate = SAMPLE_RATE;
    config.num_threads = 1;
    config
}
```

`SherpaProvider::start`: resolve engine via `engine_for` (Err → `start` Err),
create the VAD **before** spawning (`create` returns `Option` → None is an
`anyhow` error), then the worker loop:

```text
rx.recv_timeout(WORKER_TICK):
  Ok(chunk)   → samples = i16→f32 (s/32768); vad.accept_waveform(&samples);
                drain: while !vad.is_empty() { seg=vad.front(); samples=seg.samples().to_vec();
                vad.pop(); emit_segment(...) — on decode Err, error_callback once + exit }
  Timeout     → continue
  Disconnect  → break
cancel set    → break
on exit       → vad.flush(); drain once more (trailing speech is decoded)
```

`enqueue` = `self.queue.try_send(chunk).is_ok()` (false when full/gone).
`stop` = cancel flag + drop sender + join worker. The engine stays warm for
process lifetime — a model switch replaces the registry slot and the old
recognizer drops.

- [ ] **Step 4: Wire `stt/mod.rs`.**

```rust
mod sherpa;
pub use sherpa::SherpaProvider;
```

In `make_stt_provider`, add before the `_` arm:

```rust
        "sherpa" => Ok(Box::new(SherpaProvider::new(model, channel))),
```

Add next to `whisper_setup_error`:

```rust
/// Sherpa setup validation shared by Listen and Dictation — a missing or
/// partially downloaded model is a user-fixable setup error. Messages are
/// curated; they never include paths or engine details.
pub(crate) fn sherpa_setup_error(model: &str) -> Option<&'static str> {
    sherpa_setup_error_at(&crate::paths::sherpa_models_dir(), model)
}

fn sherpa_setup_error_at(root: &std::path::Path, model: &str) -> Option<&'static str> {
    let Some(entry) = crate::sherpa_models::entry_for_value(model) else {
        return Some(
            "configured Sherpa model was not found; choose an installed model in Settings",
        );
    };
    (!crate::sherpa_models::entry_installed_at(root, entry)).then_some(
        "the SenseVoice model is not downloaded; download it in Settings",
    )
}
```

- [ ] **Step 5: Run focused tests.**

```bash
cargo test sherpa -- --nocapture
cargo test stt -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add apps/native/src-tauri/src/stt/sherpa.rs apps/native/src-tauri/src/stt/mod.rs
git commit -m "feat(native): add sherpa-onnx STT provider"
```

---

## Task 4: Config accepts the sherpa provider and model

**Files:**

- Modify: `apps/native/src-tauri/src/config.rs`
- Test: `config.rs` unit tests

- [ ] **Step 1: Write the failing tests** in `config.rs` tests:

```rust
    #[test]
    fn stt_provider_validation_accepts_sherpa() {
        assert!(validate_stt_provider("sherpa").is_ok());
        assert!(validate_stt_provider("assemblyai").is_err());
    }

    #[test]
    fn sherpa_model_validation_uses_the_sherpa_catalog() {
        assert_eq!(validate_sherpa_model(" sense-voice ").unwrap(), "sense-voice");
        assert!(validate_sherpa_model("ggml-base.bin").is_err());
        assert!(validate_sherpa_model("../x").is_err());
    }

    #[test]
    fn provider_switch_to_sherpa_defaults_the_model() {
        let mut models = ModelPrefs { stt_provider: "deepgram".into(), stt_model: "nova-2".into() };
        apply_stt_config(&mut models, "models.stt_provider", &serde_json::json!("sherpa")).unwrap();
        assert_eq!(models.stt_provider, "sherpa");
        assert_eq!(models.stt_model, "sense-voice");
    }

    #[test]
    fn stt_model_write_validates_against_the_active_provider() {
        let mut models = ModelPrefs { stt_provider: "sherpa".into(), stt_model: "sense-voice".into() };
        assert!(apply_stt_config(&mut models, "models.stt_model", &serde_json::json!("tiny")).is_err());
        assert_eq!(models.stt_model, "sense-voice");
    }
```

- [ ] **Step 2: Run to verify failure.**

```bash
cargo test config -- --nocapture
```

Expected: FAIL.

- [ ] **Step 3: Implement.** In `validate_stt_provider` change the match list to
`"deepgram" | "whisper" | "sherpa"`. Add:

```rust
pub(crate) fn validate_sherpa_model(value: &str) -> Result<String, String> {
    crate::sherpa_models::entry_for_value(value.trim())
        .map(|entry| entry.id.as_str().to_string())
        .ok_or_else(|| format!("unknown Sherpa model {value:?}"))
}
```

In `validate_stt_model_for_provider` add the sherpa arm:

```rust
    if provider == "whisper" {
        validate_whisper_model(value)
    } else if provider == "sherpa" {
        validate_sherpa_model(value)
    } else {
        validate_stt_model(value)
    }
```

In `apply_stt_config`'s `models.stt_provider` arm, after the existing whisper
reset add:

```rust
            if provider == "sherpa"
                && crate::sherpa_models::entry_for_value(&models.stt_model).is_none()
            {
                models.stt_model = "sense-voice".to_string();
            }
```

- [ ] **Step 4: Run focused tests.** `cargo test config -- --nocapture` → PASS.

- [ ] **Step 5: Commit.**

```bash
git add apps/native/src-tauri/src/config.rs
git commit -m "feat(native): accept sherpa in STT config validation"
```

---

## Task 5: Commands, AppState, and the Listen/Dictation setup arms

**Files:**

- Modify: `apps/native/src-tauri/src/lib.rs`
- Modify: `apps/native/src-tauri/src/dictation.rs`
- Modify: `apps/native/src-tauri/src/listen.rs`
- Test: `lib.rs` command-contract tests + existing listen/dictation tests

**Interfaces (consumes):** `sherpa_models::{entry_for_id, entry_for_value, SherpaModelManager, SherpaStatus}`, `stt::sherpa_setup_error` — all from Tasks 2–3.

- [ ] **Step 1: Write the failing tests.** Extend the command-contract test in
`lib.rs`:

```rust
        assert!(source.contains("sherpa_status,"));
        assert!(source.contains("sherpa_download,"));
        assert!(source.contains("sherpa_cancel_download,"));
        assert!(source.contains("sherpa_remove_model,"));
```

Add a setup-error regression test asserting both services route sherpa
through the curated check:

```rust
    #[test]
    fn sherpa_setup_check_is_wired_into_both_speech_services() {
        assert!(include_str!("dictation.rs").contains("sherpa_setup_error(&model)"));
        assert!(include_str!("listen.rs").contains("sherpa_setup_error(&model)"));
    }
```

- [ ] **Step 2: Run to verify failure.**

```bash
cargo test --lib -- --nocapture
```

Expected: FAIL (commands missing; arms missing).

- [ ] **Step 3: Add the service arms.** In `dictation.rs` `start()`, immediately
after the existing `provider_name == "whisper"` block (~line 215):

```rust
        if provider_name == "sherpa" {
            // Same curated setup-error contract as whisper — a missing model
            // download is a Settings fix, not a panic.
            if let Some(message) = sherpa_setup_error(&model) {
                return Err(self.fail(epoch, &provider_name, message, true, &emit));
            }
        }
```

Add `sherpa_setup_error` to the `crate::stt::{…}` import.

In `listen.rs` `start()`, immediately after its `provider_name == "whisper"`
block (~line 371), add the same arm writing the identical `ListenStatus`
error + `ListenEvent::Error` emit + `anyhow::bail!` shape the whisper arm
uses (copy that block's body verbatim, only the check call differs).

- [ ] **Step 4: Add AppState + commands in `lib.rs`.**

```rust
mod sherpa_models;
```

`AppState` gains `sherpa_models: sherpa_models::SherpaModelManager`. In
`run()` setup, next to the `voice_models` wiring:

```rust
            let sherpa_models = sherpa_models::SherpaModelManager::new();
            sherpa_models.attach_app(handle.clone());
```

and `sherpa_models,` in the `manage(AppState { … })` literal. In `for_test`:
`SherpaModelManager::at(root.join("models").join("sherpa").join("models"))`.

Commands (next to the `whisper_*` block):

```rust
#[tauri::command]
fn sherpa_status(state: State<'_, AppState>) -> sherpa_models::SherpaStatus {
    state.sherpa_models.status()
}

#[tauri::command]
fn sherpa_download(state: State<'_, AppState>, model: String) -> Result<(), String> {
    let entry = sherpa_models::entry_for_id(&model)
        .ok_or_else(|| "Unknown voice model".to_string())?;
    state
        .sherpa_models
        .start_download(entry.id)
        .map_err(safe_voice_error)
}

#[tauri::command]
async fn sherpa_cancel_download(state: State<'_, AppState>) -> Result<(), String> {
    state
        .sherpa_models
        .cancel_download()
        .await
        .map_err(safe_voice_error)
}

#[tauri::command]
fn sherpa_remove_model(
    state: State<'_, AppState>,
    model: String,
) -> Result<sherpa_models::SherpaStatus, String> {
    let entry = sherpa_models::entry_for_id(&model)
        .ok_or_else(|| "Unknown voice model".to_string())?;
    let selected = if state.config.lock().models.stt_provider == "sherpa" {
        sherpa_models::entry_for_value(&state.config.lock().models.stt_model).map(|e| e.id)
    } else {
        None
    };
    state.sherpa_models.set_selected_model(selected);
    state
        .sherpa_models
        .remove_model(entry.id)
        .map_err(safe_voice_error)?;
    Ok(state.sherpa_models.status())
}
```

Register all four in `invoke_handler` after `whisper_remove_model`.

- [ ] **Step 5: Run the Rust suite.**

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS, no warnings.

- [ ] **Step 6: Commit.**

```bash
git add apps/native/src-tauri/src/lib.rs apps/native/src-tauri/src/dictation.rs apps/native/src-tauri/src/listen.rs
git commit -m "feat(native): wire sherpa provider into commands and speech services"
```

---

## Task 6: Frontend — commands, events, shared model grid, provider option

**Files:**

- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/lib/events.ts`
- Create: `apps/native/src/components/prefs/VoiceModelGrid.tsx`
- Modify: `apps/native/src/components/prefs/VoiceSetup.tsx`
- Test: `bun x tsc --noEmit`, `bun test`

- [ ] **Step 1: Add the contracts.** In `commands.ts`, after the whisper block:

```ts
export interface SherpaInstalledModel {
  id: string;
  label: string;
  description: string;
  bytes: number;
  source: string;
  installed: boolean;
}

export interface SherpaStatus {
  models: SherpaInstalledModel[];
  download: WhisperDownload | null;
}

export const sherpaStatus = () => invoke<SherpaStatus>('sherpa_status');
export const sherpaDownload = (model: string) =>
  invoke<void>('sherpa_download', { model });
export const sherpaCancelDownload = () =>
  invoke<void>('sherpa_cancel_download');
export const sherpaRemoveModel = (model: string) =>
  invoke<SherpaStatus>('sherpa_remove_model', { model });
```

In `events.ts`, after the whisper constants:

```ts
export const EV_SHERPA_DOWNLOAD_PROGRESS = 'sherpa:download-progress';
export const EV_SHERPA_DOWNLOAD_ERROR = 'sherpa:download-error';
export type SherpaDownloadProgressPayload = WhisperDownloadProgressPayload;
export type SherpaDownloadErrorPayload = WhisperDownloadErrorPayload;
```

- [ ] **Step 2: Extract `VoiceModelGrid`** — the whisper card grid in
`VoiceSetup.tsx:381-484` becomes a reusable component (AGENTS rule 4; arrow
function, named export). Props:

```tsx
export interface VoiceModelEntry {
  id: string;
  label: string;
  description: string;
  bytes: number;
  source: string;
}

export interface VoiceModelGridProps {
  entries: VoiceModelEntry[];
  isInstalled: (id: string) => boolean;
  isSelected: (id: string) => boolean;
  progress: Record<string, { received: number; total: number }>;
  activeDownload: string | null;
  onDownload: (id: string) => void;
  onCancel: () => void;
  onSelect: (id: string) => void;
  onRemove: (id: string) => void;
}
```

Move the card JSX verbatim into it (`formatBytes` moves too). Whisper call
site maps `catalog` → `entries`, `whisper.models` → `isInstalled`,
`provider==='whisper' && model===id` → `isSelected`, existing handlers
unchanged.

- [ ] **Step 3: Add the sherpa section in `VoiceSetup.tsx`.**
  - Provider `<select>`: add `<option value='sherpa'>Sherpa (local)</option>`.
  - Branch `{provider === 'deepgram' ? … : provider === 'whisper' ? …whisper grid… : …sherpa grid…}`.
  - Mirror the whisper state: `sherpa` status state, `refreshSherpa` calling
    `sherpaStatus`, `useTauriEvent` subs on the two sherpa events (same
    handlers, different setters), `pendingDownloadRef` shared.
  - Sherpa note text: `'Download the SenseVoice model below — it runs fully
    on-device; no separate binary is needed.'`
  - Extend the `cancelOnUnmount` effect to also `void sherpaCancelDownload()`
    when a sherpa download is active/pending.
  - The provider status dot: green when `sherpa` selected AND its model is
    installed (mirror the whisper condition).

- [ ] **Step 4: Run frontend checks.**

```bash
cd apps/native
bun x tsc --noEmit
bun test --pass-with-no-tests
```

Expected: clean.

- [ ] **Step 5: Commit.**

```bash
git add apps/native/src/lib/commands.ts apps/native/src/lib/events.ts apps/native/src/components/prefs/VoiceModelGrid.tsx apps/native/src/components/prefs/VoiceSetup.tsx
git commit -m "feat(native): add sherpa provider to voice setup"
```

---

## Task 7: End-to-end verification and hardening

**Files:** modify only what test/manual findings require.

- [ ] **Step 1: Full automated verification.**

```bash
cd apps/native/src-tauri
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cd ..
bun run build
bun test --pass-with-no-tests
git diff --check
```

- [ ] **Step 2: Manual sherpa dictation smoke.** `bun run build:dev` → Settings
→ Voice → provider `Sherpa (local)` → Download (~238 MB) → `Use this model` →
open Ask input → Dictate in English and in Chinese → live draft segments
appear at phrase boundaries → Stop leaves text, no auto-submit.

- [ ] **Step 3: Manual sherpa Listen smoke.** Collapse to idle bar → Listen →
speak + play audio on system → `me`/`them` turns appear in `ListenSection`
and persist; `listen:state` provider shows `sherpa`.

- [ ] **Step 4: Memory check.** With Listen running on sherpa, check the app
process RSS in Activity Monitor — expect roughly +450–550 MB over baseline,
and confirm it does NOT double versus a dictation-only session (the shared
engine is the difference).

- [ ] **Step 5: Regression.** Switch provider back to Whisper (still works,
existing models listed) and Deepgram (key flow intact); `dictation_*` and
`listen_*` mutual exclusion unchanged.

- [ ] **Step 6: Commit any hardening fixes.**

```bash
git add -A
git commit -m "fix(native): harden sherpa-onnx provider"
```

---

## Known Risks

1. **First-build network:** `sherpa-onnx-sys` downloads the ~17 MB static
   archive on first `cargo build`/`cargo test`; `SHERPA_ONNX_LIB_DIR` is the
   offline/CI escape hatch.
2. **Binary size:** static onnxruntime adds ~15–25 MB to the app binary.
3. **Warm engine:** the recognizer stays resident (~450–500 MB) once loaded
   until quit — chosen for zero reload latency; dropping on provider stop is
   a possible follow-up if 4 GB pressure proves tight.
4. **No interim results:** SenseVoice is offline; final-only segments are the
   same UX contract as today but with VAD boundaries. A streaming zipformer
   catalog entry is a future addition requiring no provider-contract change.
5. **Model download:** three files (~238 MB total); HF connectivity required
   at download time (same as whisper models today).
