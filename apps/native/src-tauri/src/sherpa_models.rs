use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{async_runtime::JoinHandle, AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use crate::paths;
use crate::voice_models::{DownloadErrorPayload, VoiceDownloadError};

/// `sherpa:download-*` — mirrors `EV_SHERPA_*` in `src/lib/events.ts`.
const EV_SHERPA_DOWNLOAD_PROGRESS: &str = "sherpa:download-progress";
const EV_SHERPA_DOWNLOAD_ERROR: &str = "sherpa:download-error";
const SIZE_TOLERANCE_PERCENT: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SherpaModelId {
    SenseVoice,
    SpeakerEmbedding,
    PunctEn,
    PunctZh,
}
impl SherpaModelId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SenseVoice => "sense-voice",
            Self::SpeakerEmbedding => "speaker-id",
            Self::PunctEn => "punct-en",
            Self::PunctZh => "punct-zh",
        }
    }
}

/// What a catalog entry is for. STT entries can be selected as the
/// transcription model; `SpeakerEmbedding` entries only feed speaker
/// diarization and `Punctuation` entries only post-process sherpa
/// transcripts — neither may appear as a selectable STT model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SherpaModelKind {
    Stt,
    SpeakerEmbedding,
    Punctuation,
}
impl SherpaModelKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stt => "stt",
            Self::SpeakerEmbedding => "speaker-embedding",
            Self::Punctuation => "punctuation",
        }
    }
}

/// One pinned file of a catalog entry — the sherpa "model" is a set of files
/// (recognizer, tokens, VAD) installed together under `root/<dirname>/`.
#[derive(Debug, Clone, Copy)]
pub struct SherpaFileSpec {
    pub filename: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct SherpaCatalogEntry {
    pub id: SherpaModelId,
    pub dirname: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub files: &'static [SherpaFileSpec],
    pub kind: SherpaModelKind,
}

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

const SPEAKER_EMBEDDING_FILES: &[SherpaFileSpec] = &[SherpaFileSpec {
    filename: "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx",
    bytes: 28_281_164,
    sha256: "aa3cfc16963a10586a9393f5035d6d6b57e98d358b347f80c2a30bf4f00ceba2",
}];

// sherpa-onnx-online-punct-en-2024-08-06 — CNN-BiLSTM punctuation + casing
// head from the `punctuation-models` GitHub release. Hugging Face hosts only
// community mirrors of the loose files; both pinned digests were verified
// byte-identical to the official release tarball.
const PUNCT_EN_FILES: &[SherpaFileSpec] = &[
    SherpaFileSpec {
        filename: "model.int8.onnx",
        url: "https://huggingface.co/lorneluo/sherpa-onnx-online-punct-en-2024-08-06/resolve/main/model.int8.onnx",
        bytes: 7_490_500,
        sha256: "9d611f445fe4a46186080fe161be6059d87d72eb88d3a8cb00c1a06e83a6067e",
    },
    SherpaFileSpec {
        filename: "bpe.vocab",
        url: "https://huggingface.co/lorneluo/sherpa-onnx-online-punct-en-2024-08-06/resolve/main/bpe.vocab",
        bytes: 149_430,
        sha256: "e118b7ad88c54db562517df49e1cffd4836d166c34fb190fd311d7f34eb238f5",
    },
];

// sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8 —
// CT-Transformer punctuation for zh + en text. SenseVoice's `use_itn` is
// a no-op in the pinned 2025-09-09 export (upstream k2-fsa/sherpa-onnx
// #2742), so zh segments reach the transcript with no punctuation at all
// without this. The pinned digest was verified byte-identical to the
// official `punctuation-models` release tarball.
const PUNCT_ZH_FILES: &[SherpaFileSpec] = &[SherpaFileSpec {
    filename: "model.int8.onnx",
    url: "https://huggingface.co/lorneluo/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8/resolve/main/model.int8.onnx",
    bytes: 75_519_198,
    sha256: "65a3fb9f5ad7bfb96bf69e0dc4481df97f6ee60513c1d94ce981ba6effd524b1",
}];

const CATALOG: [SherpaCatalogEntry; 4] = [
    SherpaCatalogEntry {
        id: SherpaModelId::SenseVoice,
        dirname: "sense-voice",
        label: "SenseVoice",
        description: "Transcribes Chinese, English, Japanese, Korean, and Cantonese.",
        files: SENSE_VOICE_FILES,
        kind: SherpaModelKind::Stt,
    },
    SherpaCatalogEntry {
        id: SherpaModelId::SpeakerEmbedding,
        dirname: "speaker-id",
        label: "Speaker ID",
        description: "Recognizes different voices so each speaker gets a label.",
        files: SPEAKER_EMBEDDING_FILES,
        kind: SherpaModelKind::SpeakerEmbedding,
    },
    SherpaCatalogEntry {
        id: SherpaModelId::PunctEn,
        dirname: "punct-en",
        label: "Punctuation (English)",
        description: "Restores English punctuation and capitalization.",
        files: PUNCT_EN_FILES,
        kind: SherpaModelKind::Punctuation,
    },
    SherpaCatalogEntry {
        id: SherpaModelId::PunctZh,
        dirname: "punct-zh",
        label: "Punctuation (中文)",
        description: "Restores Chinese punctuation in Chinese and mixed text.",
        files: PUNCT_ZH_FILES,
        kind: SherpaModelKind::Punctuation,
    },
];

pub const fn catalog() -> &'static [SherpaCatalogEntry] {
    &CATALOG
}
/// Match a catalog id/dirname loosely — case-insensitive and indifferent to
/// `-`/`_`/space separators — so the user-facing "SenseVoice" resolves to
/// "sense-voice". Every other character must match exactly, so URLs, paths,
/// and unrelated names still fail.
fn name_eq(name: &str, value: &str) -> bool {
    let normalize = |s: &str| -> String {
        s.trim()
            .chars()
            .filter(|c| !matches!(c, '-' | '_' | ' '))
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    normalize(name) == normalize(value)
}
pub fn entry_for_id(value: &str) -> Option<&'static SherpaCatalogEntry> {
    CATALOG.iter().find(|e| name_eq(e.id.as_str(), value))
}
fn entry_for_dirname(value: &str) -> Option<&'static SherpaCatalogEntry> {
    CATALOG.iter().find(|e| name_eq(e.dirname, value))
}
/// Resolve a user-facing value to a catalog entry by id or dirname — never a
/// path or URL, so downloads and deletes can only touch the pinned set.
pub fn entry_for_value(value: &str) -> Option<&'static SherpaCatalogEntry> {
    entry_for_id(value).or_else(|| entry_for_dirname(value))
}
/// STT-selectable subset of `entry_for_value` — non-transcription entries
/// (speaker embeddings) resolve to `None` here so they can never be stored
/// as `models.stt_model`.
pub fn stt_entry_for_value(value: &str) -> Option<&'static SherpaCatalogEntry> {
    entry_for_value(value).filter(|entry| entry.kind == SherpaModelKind::Stt)
}
/// `root/<dirname>/` — the directory that owns every file of an entry.
pub fn entry_dir(root: &Path, entry: &SherpaCatalogEntry) -> PathBuf {
    root.join(entry.dirname)
}
/// Path to the speaker-embedding ONNX when its catalog entry is installed.
pub fn speaker_embedding_model_path(root: &Path) -> Option<PathBuf> {
    let entry = entry_for_id(SherpaModelId::SpeakerEmbedding.as_str())?;
    let path = entry_dir(root, entry).join(entry.files[0].filename);
    path.is_file().then_some(path)
}
/// Paths `(cnn_bilstm, bpe_vocab)` of the English punctuation model when its
/// catalog entry is installed — `None` keeps sherpa transcripts unmodified.
fn punctuation_en_paths(root: &Path) -> Option<(PathBuf, PathBuf)> {
    let entry = entry_for_id(SherpaModelId::PunctEn.as_str())?;
    let dir = entry_dir(root, entry);
    entry_installed_at(root, entry).then(|| (dir.join("model.int8.onnx"), dir.join("bpe.vocab")))
}
/// Path of the zh-en CT-Transformer punctuator when its catalog entry is
/// installed — `None` leaves zh segments as the recognizer emits them.
fn punctuation_zh_path(root: &Path) -> Option<PathBuf> {
    let entry = entry_for_id(SherpaModelId::PunctZh.as_str())?;
    let path = entry_dir(root, entry).join(entry.files[0].filename);
    entry_installed_at(root, entry).then_some(path)
}
/// The optional punctuation add-ons' installed paths — `en` is the
/// CNN-BiLSTM `(model, vocab)` pair, `zh` the CT-Transformer model. A
/// `None` side passes that script's text through unmodified.
pub struct PunctuationPaths {
    pub en: Option<(PathBuf, PathBuf)>,
    pub zh: Option<PathBuf>,
}
/// Installed punctuation paths for the speech worker — one lookup keeps
/// the en/zh pair symmetric.
pub fn punctuation_paths(root: &Path) -> PunctuationPaths {
    PunctuationPaths {
        en: punctuation_en_paths(root),
        zh: punctuation_zh_path(root),
    }
}
/// An entry is installed only when its entire file set is present.
pub fn entry_installed_at(root: &Path, entry: &SherpaCatalogEntry) -> bool {
    let dir = entry_dir(root, entry);
    entry.files.iter().all(|f| dir.join(f.filename).is_file())
}
/// Display size of an entry: the sum of its catalog `bytes`.
fn entry_bytes(entry: &SherpaCatalogEntry) -> u64 {
    entry.files.iter().map(|f| f.bytes).sum()
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SherpaDownloadProgress {
    pub model: &'static str,
    pub received: u64,
    pub total: u64,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SherpaInstalledModel {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub bytes: u64,
    pub kind: &'static str,
    pub installed: bool,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SherpaStatus {
    pub models: Vec<SherpaInstalledModel>,
    pub download: Option<SherpaDownloadProgress>,
}

struct ActiveDownload {
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
    progress: Arc<Mutex<SherpaDownloadProgress>>,
}
struct ManagerState {
    active: Option<ActiveDownload>,
    selected: Option<SherpaModelId>,
    /// Punctuation entries already auto-attempted this run — one shot
    /// each so a failing add-on doesn't retry on every refresh.
    punct_attempted: Vec<SherpaModelId>,
}
pub struct SherpaModelManager {
    root: PathBuf,
    client: reqwest::Client,
    state: Arc<Mutex<ManagerState>>,
    app: Mutex<Option<AppHandle>>,
    cancel_gate: tokio::sync::Mutex<()>,
    install_gate: Arc<tokio::sync::Mutex<()>>,
    #[cfg(test)]
    test_files: Option<Vec<TestSource>>,
}

#[cfg(test)]
#[derive(Clone)]
struct TestSource {
    url: String,
    bytes: u64,
    sha256: String,
}
impl Default for SherpaModelManager {
    fn default() -> Self {
        Self::new()
    }
}

fn reclaim_catalog_temps(root: &Path) {
    for entry in catalog() {
        let dir = entry_dir(root, entry);
        for file in entry.files {
            let tmp = dir.join(format!("{}.tmp", file.filename));
            // Only exact catalog-owned names are ever reclaimed; unrelated temp
            // files remain untouched. A stale catalog temp cannot be resumed
            // safely because its download identity is not persisted.
            if tmp.is_file() {
                let _ = fs::remove_file(tmp);
            }
        }
    }
}

fn expected_size_bounds(display_bytes: u64) -> (u64, u64) {
    let margin = display_bytes.saturating_mul(SIZE_TOLERANCE_PERCENT) / 100;
    (
        display_bytes.saturating_sub(margin),
        display_bytes.saturating_add(margin),
    )
}

impl SherpaModelManager {
    pub fn new() -> Self {
        Self::at(paths::sherpa_models_dir())
    }
    pub fn at(root: PathBuf) -> Self {
        reclaim_catalog_temps(&root);
        Self {
            root,
            client: reqwest::Client::new(),
            state: Arc::new(Mutex::new(ManagerState {
                active: None,
                selected: None,
                punct_attempted: Vec::new(),
            })),
            app: Mutex::new(None),
            cancel_gate: tokio::sync::Mutex::new(()),
            install_gate: Arc::new(tokio::sync::Mutex::new(())),
            #[cfg(test)]
            test_files: None,
        }
    }
    #[cfg(test)]
    fn with_test_files(root: PathBuf, model: SherpaModelId, files: Vec<TestSource>) -> Self {
        // `download()` consumes the override positionally, one `TestSource`
        // per file of the downloaded entry — a short vec would silently fall
        // back to the pinned production URLs, so require a complete set for
        // the entry this test intends to download.
        assert_eq!(
            files.len(),
            entry_for_id(model.as_str()).map_or(0, |entry| entry.files.len()),
            "test override must supply one TestSource per file of the downloaded entry"
        );
        let mut manager = Self::at(root);
        manager.test_files = Some(files);
        manager
    }
    pub fn attach_app(&self, app: AppHandle) {
        *self.app.lock() = Some(app);
    }
    fn reap(&self) {
        if self
            .state
            .lock()
            .active
            .as_ref()
            .and_then(|a| a.task.as_ref())
            .is_some_and(|task| task.inner().is_finished())
        {
            self.state.lock().active = None;
        }
    }
    pub fn status(&self) -> SherpaStatus {
        self.reap();
        let download = self
            .state
            .lock()
            .active
            .as_ref()
            .map(|a| a.progress.lock().clone());
        SherpaStatus {
            models: catalog()
                .iter()
                .map(|e| SherpaInstalledModel {
                    id: e.id.as_str(),
                    label: e.label,
                    description: e.description,
                    bytes: entry_bytes(e),
                    kind: e.kind.as_str(),
                    installed: entry_installed_at(&self.root, e),
                })
                .collect(),
            download,
        }
    }
    pub fn start_download(&self, model: SherpaModelId) -> Result<(), VoiceDownloadError> {
        self.reap();
        let entry = catalog()
            .iter()
            .find(|e| e.id == model)
            .ok_or_else(|| VoiceDownloadError::UnknownModel(model.as_str().into()))?;
        let mut state = self.state.lock();
        if state.active.is_some() {
            return Err(VoiceDownloadError::Busy);
        }
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let root = self.root.clone();
        let client = self.client.clone();
        let app = self.app.lock().clone();
        let install_gate = Arc::clone(&self.install_gate);
        #[cfg(test)]
        let test_files = self.test_files.clone();
        let entry = *entry;
        let progress = Arc::new(Mutex::new(SherpaDownloadProgress {
            model: entry.id.as_str(),
            received: 0,
            total: entry_bytes(&entry),
        }));
        let task_progress = Arc::clone(&progress);
        let done_progress = Arc::clone(&progress);
        let task_state = Arc::clone(&self.state);
        let task = tauri::async_runtime::spawn(async move {
            let result = download(
                entry,
                root,
                client,
                task_cancel,
                app.clone(),
                task_progress,
                install_gate,
                #[cfg(test)]
                test_files,
            )
            .await;
            match result {
                // A finished install may resolve a durable speech setup
                // error — re-validate so the stale banner clears now.
                Ok(()) => {
                    // Clear our own `active` first — it still names this
                    // finishing task, and `refresh_speech_setup` →
                    // `ensure_punct` must be free to queue the next
                    // missing add-on in the same wake. Identity-checked:
                    // a cancel+restart could have installed a different
                    // download meanwhile.
                    {
                        let mut state = task_state.lock();
                        if state
                            .active
                            .as_ref()
                            .is_some_and(|a| Arc::ptr_eq(&a.progress, &done_progress))
                        {
                            state.active = None;
                        }
                    }
                    if let Some(app) = &app {
                        crate::refresh_speech_setup(app);
                    }
                }
                Err(error) => {
                    // Same identity-checked release as the Ok arm — a
                    // failed task must not leave its finished self
                    // registered as `active` for a reaper to find later.
                    {
                        let mut state = task_state.lock();
                        if state
                            .active
                            .as_ref()
                            .is_some_and(|a| Arc::ptr_eq(&a.progress, &done_progress))
                        {
                            state.active = None;
                        }
                    }
                    emit_error(&app, entry.id.as_str(), &error);
                }
            }
        });
        state.active = Some(ActiveDownload {
            cancel,
            task: Some(task),
            progress,
        });
        Ok(())
    }
    /// Download each missing punctuation add-on once per run when sherpa
    /// is the active provider and its STT model is installed. One
    /// download at a time: a completed install clears `active` then
    /// re-enters through `refresh_speech_setup` → `ensure_punct`, which
    /// queues the next missing entry. Silent best-effort — `Busy`/errors
    /// surface only as `sherpa:*` events.
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
        // A task that died before reaching its finishing arm leaves a
        // completed handle registered as `active` — reap it like
        // `status`/`start_download`/`remove_model` do or the chain stalls.
        self.reap();
        // Catalog scan + filesystem stats stay outside the state lock.
        let missing: Vec<SherpaModelId> = catalog()
            .iter()
            .filter(|e| {
                e.kind == SherpaModelKind::Punctuation && !entry_installed_at(&self.root, e)
            })
            .map(|e| e.id)
            .collect();
        let next = {
            let mut state = self.state.lock();
            if state.active.is_some() {
                return;
            }
            let Some(&id) = missing
                .iter()
                .find(|id| !state.punct_attempted.contains(id))
            else {
                return;
            };
            state.punct_attempted.push(id);
            id
        };
        // A start that lost the `active` race never spawned anything —
        // the once-per-run mark must roll back so the next refresh can
        // still heal this entry this run.
        if self.start_download(next).is_err() {
            self.state
                .lock()
                .punct_attempted
                .retain(|id| *id != next);
        }
    }
    pub async fn cancel_download(&self) -> Result<(), VoiceDownloadError> {
        let _gate = self.cancel_gate.lock().await;
        let (task, progress) = {
            let mut state = self.state.lock();
            let Some(active) = state.active.as_mut() else {
                return Ok(());
            };
            active.cancel.cancel();
            (active.task.take(), Arc::clone(&active.progress))
        };
        if let Some(task) = task {
            let _ = task.await;
        }
        // Identity-checked like the task-side clear: while the cancelled
        // task wound down its finishing arm may have freed the slot and a
        // new download registered — a blind clear would orphan it.
        let mut state = self.state.lock();
        if state
            .active
            .as_ref()
            .is_some_and(|a| Arc::ptr_eq(&a.progress, &progress))
        {
            state.active = None;
        }
        Ok(())
    }
    pub fn set_selected_model(&self, model: Option<SherpaModelId>) {
        self.state.lock().selected = model;
    }
    pub fn remove_model(&self, model: SherpaModelId) -> Result<(), VoiceDownloadError> {
        self.reap();
        let state = self.state.lock();
        if state.active.is_some() {
            return Err(VoiceDownloadError::Busy);
        }
        if state.selected == Some(model) {
            return Err(VoiceDownloadError::ActiveModel);
        }
        drop(state);
        let entry = catalog()
            .iter()
            .find(|e| e.id == model)
            .ok_or_else(|| VoiceDownloadError::UnknownModel(model.as_str().into()))?;
        match fs::remove_dir_all(entry_dir(&self.root, entry)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(VoiceDownloadError::Download(e.to_string())),
        }
    }
}

/// Download the entry's file set sequentially into `root/<dirname>/`. Each file
/// goes through the whisper `download_inner` flow (tmp, sha256 + ±10% size
/// verify, install gate, rename); `progress.received` aggregates the bytes of
/// completed files plus the in-flight file's count.
#[allow(clippy::too_many_arguments)]
async fn download(
    entry: SherpaCatalogEntry,
    root: PathBuf,
    client: reqwest::Client,
    cancel: CancellationToken,
    app: Option<AppHandle>,
    progress: Arc<Mutex<SherpaDownloadProgress>>,
    install_gate: Arc<tokio::sync::Mutex<()>>,
    #[cfg(test)] test_files: Option<Vec<TestSource>>,
) -> Result<(), VoiceDownloadError> {
    let dir = entry_dir(&root, &entry);
    fs::create_dir_all(&dir).map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    // Per-file test overrides align with `entry.files` by position — the
    // sequential `next()` mirrors indexing without an unused counter in
    // non-test builds.
    #[cfg(test)]
    let mut test_sources = test_files.map(|files| files.into_iter());
    let mut completed = 0u64;
    for file in entry.files.iter() {
        let final_path = dir.join(file.filename);
        let tmp_path = dir.join(format!("{}.tmp", file.filename));
        #[cfg(test)]
        let test_source = test_sources.as_mut().and_then(|sources| sources.next());
        completed += download_inner(
            &entry,
            file,
            &final_path,
            &tmp_path,
            client.clone(),
            cancel.clone(),
            app.clone(),
            Arc::clone(&progress),
            completed,
            Arc::clone(&install_gate),
            #[cfg(test)]
            test_source,
        )
        .await?;
    }
    // Completion is authoritative: catalog bytes are display metadata, so the
    // terminal event reports the bytes actually received across the whole set.
    emit_progress(&app, terminal_progress(entry.id.as_str(), completed));
    Ok(())
}
struct TempFileGuard<'a> {
    path: &'a Path,
    owned: bool,
}
impl Drop for TempFileGuard<'_> {
    fn drop(&mut self) {
        if self.owned {
            let _ = fs::remove_file(self.path);
        }
    }
}

/// Download one file of an entry. `completed` is the byte count of the files
/// already installed this run — progress events report `completed + received`
/// against the catalog's total display bytes. Returns the file's received
/// byte count so the caller can accumulate it.
#[allow(clippy::too_many_arguments)]
async fn download_inner(
    entry: &SherpaCatalogEntry,
    file: &SherpaFileSpec,
    final_path: &Path,
    tmp_path: &Path,
    client: reqwest::Client,
    cancel: CancellationToken,
    app: Option<AppHandle>,
    progress: Arc<Mutex<SherpaDownloadProgress>>,
    completed: u64,
    install_gate: Arc<tokio::sync::Mutex<()>>,
    #[cfg(test)] test_source: Option<TestSource>,
) -> Result<u64, VoiceDownloadError> {
    let (url, expected_size, expected_sha256) = {
        #[cfg(test)]
        if let Some(source) = test_source {
            (source.url, source.bytes, source.sha256)
        } else {
            (file.url.to_string(), file.bytes, file.sha256.to_string())
        }
        #[cfg(not(test))]
        (file.url.to_string(), file.bytes, file.sha256.to_string())
    };
    let total = entry_bytes(entry);
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(VoiceDownloadError::Cancelled),
        r = client.get(url).send() => r.map_err(|e| VoiceDownloadError::Download(e.to_string()))?
    };
    if !response.status().is_success() {
        return Err(VoiceDownloadError::Download(format!(
            "HTTP {}",
            response.status()
        )));
    }
    let mut file_options = OpenOptions::new();
    file_options.create_new(true).write(true);
    #[cfg(unix)]
    file_options.mode(0o600);
    let mut file_handle = file_options
        .open(tmp_path)
        .map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    let mut guard = TempFileGuard {
        path: tmp_path,
        owned: true,
    };
    let mut stream = response.bytes_stream();
    let mut received = 0u64;
    let mut hash = Sha256::new();
    let mut last_progress = Instant::now() - Duration::from_millis(150);
    while let Some(chunk) = tokio::select! {
        _ = cancel.cancelled() => return Err(VoiceDownloadError::Cancelled),
        chunk = stream.next() => chunk
    } {
        let chunk = chunk.map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
        received = received.saturating_add(chunk.len() as u64);
        let max_size = expected_size_bounds(expected_size).1;
        if received > max_size {
            return Err(VoiceDownloadError::Verification);
        }
        file_handle
            .write_all(&chunk)
            .map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
        hash.update(&chunk);
        progress.lock().received = completed + received;
        if last_progress.elapsed() >= Duration::from_millis(150) {
            emit_progress(
                &app,
                SherpaDownloadProgress {
                    model: entry.id.as_str(),
                    received: completed + received,
                    total,
                },
            );
            last_progress = Instant::now();
        }
    }
    file_handle
        .sync_all()
        .map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    let digest = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let (min_size, max_size) = expected_size_bounds(expected_size);
    if received < min_size || received > max_size || digest != expected_sha256 {
        return Err(VoiceDownloadError::Verification);
    }
    // Serialize the cancellation decision with the final rename. If cancellation
    // gets this gate first, no install occurs; if rename gets it first, cancel
    // waits and returns only after the installed file is authoritative.
    let _install = install_gate.lock().await;
    if cancel.is_cancelled() {
        return Err(VoiceDownloadError::Cancelled);
    }
    emit_progress(
        &app,
        SherpaDownloadProgress {
            model: entry.id.as_str(),
            received: completed + received,
            total,
        },
    );
    fs::rename(tmp_path, final_path).map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    guard.owned = false;
    Ok(received)
}

fn terminal_progress(model: &'static str, received: u64) -> SherpaDownloadProgress {
    SherpaDownloadProgress {
        model,
        received,
        total: received,
    }
}

fn emit_progress(app: &Option<AppHandle>, progress: SherpaDownloadProgress) {
    if let Some(app) = app {
        let _ = app.emit(EV_SHERPA_DOWNLOAD_PROGRESS, progress);
    }
}

fn emit_error(app: &Option<AppHandle>, model: &'static str, error: &VoiceDownloadError) {
    if matches!(error, VoiceDownloadError::Cancelled) {
        return;
    }
    let message = match error {
        VoiceDownloadError::Cancelled => "download cancelled",
        VoiceDownloadError::Verification => "download verification failed",
        VoiceDownloadError::Download(_) => "download failed",
        // The remaining arms are unreachable from `download()` (it only
        // produces `Cancelled` — filtered above — `Verification`, and
        // `Download`) and exist for match exhaustiveness.
        VoiceDownloadError::Busy => "download is already active",
        VoiceDownloadError::UnknownModel(_) => "unknown voice model",
        VoiceDownloadError::ActiveModel => "cannot remove the selected voice model",
    };
    if let Some(app) = app {
        let _ = app.emit(
            EV_SHERPA_DOWNLOAD_ERROR,
            DownloadErrorPayload { model, message },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn catalog_is_the_approved_file_set() {
        assert_eq!(catalog().len(), 4);
        let entry = &catalog()[0];
        assert_eq!(entry.id.as_str(), "sense-voice");
        assert_eq!(entry.dirname, "sense-voice");
        assert_eq!(entry.kind, SherpaModelKind::Stt);
        assert_eq!(
            entry.files.iter().map(|f| f.filename).collect::<Vec<_>>(),
            ["model.int8.onnx", "tokens.txt", "silero_vad.onnx"]
        );
        assert!(entry.files.iter().all(|f| f.url.starts_with("https://")
            && f.sha256.len() == 64
            && f.sha256.chars().all(|c| c.is_ascii_hexdigit())));
        let speaker = &catalog()[1];
        assert_eq!(speaker.id.as_str(), "speaker-id");
        assert_eq!(speaker.dirname, "speaker-id");
        assert_eq!(speaker.kind, SherpaModelKind::SpeakerEmbedding);
        assert_eq!(speaker.files.len(), 1);
        assert!(speaker.files.iter().all(|f| f.url.starts_with("https://")
            && f.sha256.len() == 64
            && f.sha256.chars().all(|c| c.is_ascii_hexdigit())));
        let punct = &catalog()[2];
        assert_eq!(punct.id.as_str(), "punct-en");
        assert_eq!(punct.dirname, "punct-en");
        assert_eq!(punct.kind, SherpaModelKind::Punctuation);
        assert_eq!(
            punct.files.iter().map(|f| f.filename).collect::<Vec<_>>(),
            ["model.int8.onnx", "bpe.vocab"]
        );
        assert!(punct.files.iter().all(|f| f.url.starts_with("https://")
            && f.sha256.len() == 64
            && f.sha256.chars().all(|c| c.is_ascii_hexdigit())));
        let punct_zh = &catalog()[3];
        assert_eq!(punct_zh.id.as_str(), "punct-zh");
        assert_eq!(punct_zh.dirname, "punct-zh");
        assert_eq!(punct_zh.kind, SherpaModelKind::Punctuation);
        assert_eq!(
            punct_zh.files.iter().map(|f| f.filename).collect::<Vec<_>>(),
            ["model.int8.onnx"]
        );
        assert!(punct_zh.files.iter().all(|f| f.url.starts_with("https://")
            && f.sha256.len() == 64
            && f.sha256.chars().all(|c| c.is_ascii_hexdigit())));
    }

    /// Non-STT entries are downloadable but must never be selectable as the
    /// transcription model — `stt_entry_for_value` filters them while the
    /// general lookup still resolves them for download/remove.
    #[test]
    fn aux_models_are_downloadable_but_never_an_stt_model() {
        for id in ["speaker-id", "punct-en", "punct-zh"] {
            assert!(entry_for_value(id).is_some());
            assert!(stt_entry_for_value(id).is_none());
            assert!(crate::config::validate_sherpa_model(id).is_err());
        }
        assert!(stt_entry_for_value("sense-voice").is_some());
        assert_eq!(
            crate::config::validate_sherpa_model("sense-voice").unwrap(),
            "sense-voice"
        );
    }

    #[test]
    fn lookup_rejects_arbitrary_ids_urls_and_paths() {
        assert_eq!(
            entry_for_id(" SenseVoice ").unwrap().id,
            SherpaModelId::SenseVoice
        );
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
                kind: "stt",
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

    fn fixture(body: Vec<u8>, delay: Duration) -> TestSource {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let expected = body.clone();
        std::thread::spawn(move || {
            // Loop-accept: a source may be re-fetched — the punctuation
            // chain downloads more than one catalog entry per manager.
            while let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0; 1024];
                let _ = stream.read(&mut request);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    expected.len()
                );
                let _ = stream.write_all(header.as_bytes());
                for chunk in expected.chunks(2) {
                    let _ = stream.write_all(chunk);
                    let _ = stream.flush();
                    std::thread::sleep(delay);
                }
            }
        });
        let mut sha = Sha256::new();
        sha.update(&body);
        let digest = sha
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        TestSource {
            url: format!("http://{address}/model"),
            bytes: body.len() as u64,
            sha256: digest,
        }
    }

    /// One one-shot server per file — files download sequentially, so each
    /// entry file needs its own listener. The returned vec overrides the
    /// catalog specs by index via `with_test_files`, which requires one
    /// source per entry file.
    fn fixture_set(bodies: &[Vec<u8>], delay: Duration) -> Vec<TestSource> {
        bodies
            .iter()
            .map(|body| fixture(body.clone(), delay))
            .collect()
    }

    fn temp_root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir();
        loop {
            let root = base.join(format!(
                "marvis-sherpa-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
            ));
            if fs::create_dir(&root).is_ok() {
                return root;
            }
        }
    }

    #[test]
    fn sync_start_download_uses_the_tauri_runtime() {
        let root = temp_root();
        // File 1 lands quickly; files 2/3 stream slowly so the download is
        // still in flight when the synchronous cancel below arrives.
        let sources = vec![
            fixture(b"sync start fixture".to_vec(), Duration::ZERO),
            fixture(b"sync start tokens".to_vec(), Duration::from_millis(100)),
            fixture(b"sync start vad".to_vec(), Duration::from_millis(100)),
        ];
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, sources);

        manager.start_download(SherpaModelId::SenseVoice).unwrap();
        tauri::async_runtime::block_on(manager.cancel_download()).unwrap();

        let dir = entry_dir(&root, &catalog()[0]);
        for file in catalog()[0].files {
            assert!(!dir.join(format!("{}.tmp", file.filename)).exists());
        }
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn lifecycle_verifies_cleans_preserves_and_serializes_downloads() {
        let bodies = [
            b"sense voice model fixture".to_vec(),
            b"tokens fixture".to_vec(),
            b"vad fixture".to_vec(),
        ];
        let sources = fixture_set(&bodies, Duration::from_millis(10));
        assert!(sources[0].url.starts_with("http://127.0.0.1:"));
        let root = temp_root();
        let dir = entry_dir(&root, &catalog()[0]);
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, sources);
        manager.start_download(SherpaModelId::SenseVoice).unwrap();
        assert!(matches!(
            manager.start_download(SherpaModelId::SenseVoice),
            Err(VoiceDownloadError::Busy)
        ));
        manager.cancel_download().await.unwrap();
        for file in catalog()[0].files {
            assert!(!dir.join(format!("{}.tmp", file.filename)).exists());
        }

        // A failed sha256 must not clobber an already-installed file.
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("model.int8.onnx"), b"keep me").unwrap();
        let mut bad_sources = fixture_set(&bodies, Duration::ZERO);
        bad_sources[0].sha256 = "00".repeat(32);
        let bad = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, bad_sources);
        bad.start_download(SherpaModelId::SenseVoice).unwrap();
        while bad.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert_eq!(fs::read(dir.join("model.int8.onnx")).unwrap(), b"keep me");
        assert!(!dir.join("model.int8.onnx.tmp").exists());

        // A size far outside the ±10% band aborts the install as well.
        let mut bad_size_sources = fixture_set(&bodies, Duration::ZERO);
        bad_size_sources[0].bytes = u64::MAX / 2;
        let bad_size = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, bad_size_sources);
        bad_size.start_download(SherpaModelId::SenseVoice).unwrap();
        while bad_size.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(!dir.join("model.int8.onnx.tmp").exists());
        assert_eq!(fs::read(dir.join("model.int8.onnx")).unwrap(), b"keep me");

        // Success requires every file in the set to land, content intact.
        let success_sources = fixture_set(&bodies, Duration::ZERO);
        let success = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, success_sources);
        success.start_download(SherpaModelId::SenseVoice).unwrap();
        while success.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(entry_installed_at(&root, &catalog()[0]));
        for (i, file) in catalog()[0].files.iter().enumerate() {
            assert_eq!(fs::read(dir.join(file.filename)).unwrap(), bodies[i]);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("model.int8.onnx"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_reclaims_only_catalog_temps() {
        let root = temp_root();
        let dir = entry_dir(&root, &catalog()[0]);
        fs::create_dir_all(&dir).unwrap();
        for file in catalog()[0].files {
            fs::write(dir.join(format!("{}.tmp", file.filename)), b"stale").unwrap();
        }
        fs::write(dir.join("foreign.tmp"), b"keep").unwrap();
        fs::write(root.join("foreign.tmp"), b"keep root").unwrap();
        let _manager = SherpaModelManager::at(root.clone());
        for file in catalog()[0].files {
            assert!(!dir.join(format!("{}.tmp", file.filename)).exists());
        }
        assert!(dir.join("foreign.tmp").exists());
        assert!(root.join("foreign.tmp").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn active_model_removal_is_rejected() {
        let manager = SherpaModelManager::at(temp_root());
        manager.set_selected_model(Some(SherpaModelId::SenseVoice));
        assert!(matches!(
            manager.remove_model(SherpaModelId::SenseVoice),
            Err(VoiceDownloadError::ActiveModel)
        ));
        manager.set_selected_model(None);
        let _ = fs::remove_dir_all(manager.root);
    }

    #[tokio::test]
    async fn removal_is_rejected_while_download_is_active() {
        let root = temp_root();
        let bodies = [
            b"fixture a".to_vec(),
            b"fixture b".to_vec(),
            b"fixture c".to_vec(),
        ];
        let sources = fixture_set(&bodies, Duration::from_millis(100));
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, sources);
        manager.start_download(SherpaModelId::SenseVoice).unwrap();
        assert!(matches!(
            manager.remove_model(SherpaModelId::SenseVoice),
            Err(VoiceDownloadError::Busy)
        ));
        manager.cancel_download().await.unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancellation_racing_final_install_has_one_authoritative_outcome() {
        let root = temp_root();
        let bodies = [b"race a".to_vec(), b"race b".to_vec(), b"race c".to_vec()];
        let sources = fixture_set(&bodies, Duration::ZERO);
        let manager = Arc::new(SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, sources));
        manager.start_download(SherpaModelId::SenseVoice).unwrap();
        let canceller = Arc::clone(&manager);
        canceller.cancel_download().await.unwrap();
        let dir = entry_dir(&root, &catalog()[0]);
        for file in catalog()[0].files {
            assert!(!dir.join(format!("{}.tmp", file.filename)).exists());
        }
        if entry_installed_at(&root, &catalog()[0]) {
            for (i, file) in catalog()[0].files.iter().enumerate() {
                assert_eq!(fs::read(dir.join(file.filename)).unwrap(), bodies[i]);
            }
        }
        assert!(manager.status().download.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancellation_does_not_remove_preexisting_temp_file() {
        let root = temp_root();
        let dir = entry_dir(&root, &catalog()[0]);
        fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join("foreign.tmp");
        fs::write(&tmp, b"owned by someone else").unwrap();
        // Slow fixtures keep the download in flight so the cancel below
        // deterministically lands mid-stream.
        let bodies = [
            b"fixture a".to_vec(),
            b"fixture b".to_vec(),
            b"fixture c".to_vec(),
        ];
        let sources = fixture_set(&bodies, Duration::from_millis(100));
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::SenseVoice, sources);
        manager.start_download(SherpaModelId::SenseVoice).unwrap();
        manager.cancel_download().await.unwrap();
        assert_eq!(fs::read(tmp).unwrap(), b"owned by someone else");
        let _ = fs::remove_dir_all(root);
    }

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
        // Each further call queues the next missing add-on — the zh
        // punctuator here (it consumes the same positional fixtures).
        manager.ensure_punct("sherpa", "sense-voice");
        while manager.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(entry_installed_at(&root, entry_for_id("punct-zh").unwrap()));
        // Once everything is installed the call is a no-op — no Busy
        // error, nothing re-queued.
        manager.ensure_punct("sherpa", "sense-voice");
        assert!(manager.status().download.is_none());
        let _ = fs::remove_dir_all(root);
    }

    /// A non-sherpa provider, an stt value outside the sherpa STT catalog,
    /// and a not-yet-installed STT model all skip the fetch — and none of
    /// those skips burns the once-per-run flag, so the first refresh after
    /// SenseVoice lands still heals.
    #[tokio::test]
    async fn ensure_punct_only_fires_for_sherpa_with_stt_installed() {
        let root = temp_root();
        let stt_dir = entry_dir(&root, &catalog()[0]);
        fs::create_dir_all(&stt_dir).unwrap();
        for f in catalog()[0].files {
            fs::write(stt_dir.join(f.filename), b"x").unwrap();
        }
        let sources = fixture_set(&[b"punct model".to_vec(), b"bpe vocab".to_vec()], Duration::ZERO);
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::PunctEn, sources);
        manager.ensure_punct("whisper", "sense-voice");
        manager.ensure_punct("sherpa", "tiny");
        assert!(manager.status().download.is_none());
        manager.ensure_punct("sherpa", "sense-voice");
        while manager.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(entry_installed_at(&root, entry_for_id("punct-en").unwrap()));
        let _ = fs::remove_dir_all(root);

        let root = temp_root();
        let sources = fixture_set(&[b"punct model".to_vec(), b"bpe vocab".to_vec()], Duration::ZERO);
        let missing = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::PunctEn, sources);
        missing.ensure_punct("sherpa", "sense-voice");
        assert!(missing.status().download.is_none());
        let stt_dir = entry_dir(&root, &catalog()[0]);
        fs::create_dir_all(&stt_dir).unwrap();
        for f in catalog()[0].files {
            fs::write(stt_dir.join(f.filename), b"x").unwrap();
        }
        missing.ensure_punct("sherpa", "sense-voice");
        while missing.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(entry_installed_at(&root, entry_for_id("punct-en").unwrap()));
        let _ = fs::remove_dir_all(root);
    }

    /// A task that died before reaching its finishing arm leaves a
    /// completed handle registered as `active`. `ensure_punct` must reap
    /// it like `status`/`start_download` do — otherwise the chain stalls
    /// until something else happens to reap.
    #[tokio::test]
    async fn ensure_punct_reaps_a_finished_active_before_deciding() {
        let root = temp_root();
        let stt_dir = entry_dir(&root, &catalog()[0]);
        fs::create_dir_all(&stt_dir).unwrap();
        for f in catalog()[0].files {
            fs::write(stt_dir.join(f.filename), b"x").unwrap();
        }
        let sources = fixture_set(&[b"punct model".to_vec(), b"bpe vocab".to_vec()], Duration::ZERO);
        let manager = SherpaModelManager::with_test_files(root.clone(), SherpaModelId::PunctEn, sources);
        // The state a dead task leaves behind: a finished handle still
        // named in `active`.
        let task = tauri::async_runtime::spawn(async {});
        manager.state.lock().active = Some(ActiveDownload {
            cancel: CancellationToken::new(),
            task: Some(task),
            progress: Arc::new(Mutex::new(SherpaDownloadProgress {
                model: "punct-en",
                received: 0,
                total: 1,
            })),
        });
        // Let the spawned task finish — `status()` would reap, so wait on
        // the raw handle instead.
        loop {
            let done = manager
                .state
                .lock()
                .active
                .as_ref()
                .and_then(|a| a.task.as_ref())
                .is_some_and(|t| t.inner().is_finished());
            if done {
                break;
            }
            tokio::task::yield_now().await;
        }
        manager.ensure_punct("sherpa", "sense-voice");
        while manager.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(entry_installed_at(&root, entry_for_id("punct-en").unwrap()));
        let _ = fs::remove_dir_all(root);
    }

    /// The cancelled task's finishing arm can free `active` and a new
    /// download register before the stale cancel resumes — its final
    /// clear must be identity-checked or it orphans the newer download.
    #[tokio::test]
    async fn cancel_does_not_orphan_a_download_registered_after_the_old_one() {
        let root = temp_root();
        let manager = Arc::new(SherpaModelManager::at(root.clone()));
        // A registered download whose completion the test controls — the
        // shape `active` takes while a download is in flight.
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel::<()>();
        let a_progress = Arc::new(Mutex::new(SherpaDownloadProgress {
            model: "sense-voice",
            received: 0,
            total: 1,
        }));
        manager.state.lock().active = Some(ActiveDownload {
            cancel: CancellationToken::new(),
            task: Some(tauri::async_runtime::spawn(async move {
                let _ = finish_rx.await;
            })),
            progress: a_progress,
        });
        let canceller = Arc::clone(&manager);
        let cancel = tokio::spawn(async move { canceller.cancel_download().await });
        // Wait until the cancel has lifted the task handle and parked on
        // its completion.
        loop {
            let parked = manager
                .state
                .lock()
                .active
                .as_ref()
                .is_some_and(|a| a.task.is_none());
            if parked {
                break;
            }
            tokio::task::yield_now().await;
        }
        // A "finished on its own" mid-cancel: the slot freed and a new
        // download registered before the stale cancel resumed.
        let b_progress = Arc::new(Mutex::new(SherpaDownloadProgress {
            model: "punct-en",
            received: 0,
            total: 1,
        }));
        manager.state.lock().active = Some(ActiveDownload {
            cancel: CancellationToken::new(),
            task: None,
            progress: Arc::clone(&b_progress),
        });
        let _ = finish_tx.send(());
        cancel.await.unwrap().unwrap();
        // B's registration survives — the stale cancel only cleared its own.
        assert!(manager
            .state
            .lock()
            .active
            .as_ref()
            .is_some_and(|a| Arc::ptr_eq(&a.progress, &b_progress)));
        let _ = fs::remove_dir_all(root);
    }
}
