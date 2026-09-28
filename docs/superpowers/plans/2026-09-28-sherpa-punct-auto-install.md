# Sherpa Punctuation Auto-Install Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: execute inline in this
> session. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** New users never have to think about `punct-en` — when SenseVoice
is installed and sherpa is the active provider, the punctuation+casing
model downloads itself. The "Punctuation & casing" card stays as
progress/remove UI only, never a required click.

**Why this shape:** `refresh_speech_setup` (lib.rs) already runs after
every completed model download and every config write, and has
`AppState.sherpa_models` in reach — one `ensure` call there heals both
new installs (fires the moment the SenseVoice download finishes) and
pre-change installs (fires on the next config save / app run). No
frontend chain logic needed: the card's existing `sherpa:download-progress`
handler renders the auto-download for free.

**Rejected alternative — bundling punct files into `SENSE_VOICE_FILES`:**
keeps `entry_installed_at` honest only via a new `required` flag on
`SherpaFileSpec`, forces a `model.int8.onnx` filename collision inside
the sense-voice dir, loses standalone Remove, and still needs a heal
path for existing installs. A separate entry + auto-start is less code.

**Constraints / gotchas:**

- Silent network fetch is fine — the model is ~7.6 MB, catalog-pinned,
  and the user already chose sherpa + downloaded SenseVoice.
- Attempt the auto-download **at most once per app run**: a failing
  fetch must not retry-loop on every config write. The card's Download
  button remains the manual retry.
- Never block `refresh_speech_setup`: `ensure_punct` is fire-and-forget,
  `Busy` is a no-op, errors surface only through the existing
  `sherpa:download-error` event.
- Diarization (`speaker-id`) is **not** auto-downloaded — it gates the
  voiceprint feature and stays an explicit opt-in.
- Arrow-function components, named exports, `| --- | --- |` tables,
  `bun` for scripts (project rules).

## File Map

- Modify: `apps/native/src-tauri/src/sherpa_models.rs` —
  `ManagerState.punct_attempted: bool`, `ensure_punct()`, one test.
- Modify: `apps/native/src-tauri/src/lib.rs` — one line inside
  `refresh_speech_setup` calling `state.sherpa_models.ensure_punct(&config)`.
- Modify: `apps/native/src/components/prefs/VoiceSetup.tsx` — punct card
  copy says it installs itself with SenseVoice; no logic changes.

### Task 1: `SherpaModelManager::ensure_punct` + heal wiring

**Files:**

- Modify: `apps/native/src-tauri/src/sherpa_models.rs`
- Modify: `apps/native/src-tauri/src/lib.rs`

- [ ] **Step 1:** `ManagerState` gains `punct_attempted: bool` (default
  `false`). Next to it, on `SherpaModelManager`:

```rust
/// Download the punctuation add-on once per run when sherpa is the
/// active provider and its STT model is installed but punct is missing.
/// Silent best-effort: `Busy`/errors surface only as `sherpa:*` events.
pub fn ensure_punct(&self, provider: &str, stt_model: &str) {
    if provider != "sherpa" {
        return;
    }
    let Some(stt) = stt_entry_for_value(stt_model) else {
        return;
    };
    if !entry_installed_at(&self.root, stt) {
        return;
    }
    let punct = entry_for_id(SherpaModelId::PunctEn.as_str()).expect("catalog");
    if entry_installed_at(&self.root, punct) {
        return;
    }
    {
        let mut state = self.state.lock();
        if state.active.is_some() || state.punct_attempted {
            return;
        }
        state.punct_attempted = true;
    }
    let _ = self.start_download(SherpaModelId::PunctEn);
}
```

(Once-per-run flag lives under the same `state` lock so it can't race
the `active` check.)

- [ ] **Step 2:** `lib.rs::refresh_speech_setup` — right after
  `let sherpa_root = paths::sherpa_models_dir();` add:

```rust
state
    .sherpa_models
    .ensure_punct(&config.models.stt_provider, &config.models.stt_model);
```

- [ ] **Step 3:** Test in `sherpa_models::tests` — `with_test_files`
  already accepts a model id, so seed a temp root where sense-voice is
  installed and punct fixtures are wired:

```rust
#[tokio::test]
async fn ensure_punct_downloads_once_when_stt_installed() {
    let root = temp_root();
    // "Installed" stt entry: all its catalog files present.
    let stt_dir = entry_dir(&root, &catalog()[0]);
    fs::create_dir_all(&stt_dir).unwrap();
    for f in catalog()[0].files {
        fs::write(stt_dir.join(f.filename), b"x").unwrap();
    }
    let sources = fixture_set(&[b"punct model".to_vec(), b"bpe vocab".to_vec()], Duration::ZERO);
    let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::PunctEn, sources);
    manager.ensure_punct("sherpa", "sense-voice");
    while manager.status().download.is_some() {
        tokio::task::yield_now().await;
    }
    assert!(entry_installed_at(&root, entry_for_id("punct-en").unwrap()));
    // Second call is a no-op — no Busy error, nothing re-queued.
    manager.ensure_punct("sherpa", "sense-voice");
    assert!(manager.status().download.is_none());
    let _ = fs::remove_dir_all(root);
}
```

Plus negative arms in the same test or a second one: provider
`"whisper"` → no download; stt model not installed → no download.

- [ ] **Step 4:** `cargo test --lib sherpa` — all green.

### Task 2: VoiceSetup copy — card describes auto-install

**Files:**

- Modify: `apps/native/src/components/prefs/VoiceSetup.tsx`

- [ ] **Step 1:** Punct card note becomes self-installing language
  (download starts on its own once SenseVoice is in — the card is
  progress + optional Remove, not a required click):

```tsx
<p className={PROV_NOTE}>
  {punctModel.description} Downloads automatically with SenseVoice —
  remove it only if you prefer the raw transcript.
</p>
```

- [ ] **Step 2:** `bun run check-types` clean; eyeball the card under
  the sherpa provider in `bun run dev` if convenient.

### Task 3: end-to-end heal check

- [ ] **Step 1:** With the app built (`cargo build` or `tauri dev`):
  rename `~/.marvis/models/sherpa/models/punct-en` aside, flip any
  setting (fires `refresh_speech_setup`), confirm the dir is re-created
  and `sherpa_status` reports `punct-en` installed — no clicks.
- [ ] **Step 2:** Restore `punct-en`, flip another setting, confirm no
  second download starts (`status().download` stays `None`).
