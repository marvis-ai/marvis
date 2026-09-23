use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use sha1::{Digest, Sha1};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{paths, stt};

pub const HUGGING_FACE_PREFIX: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelId {
    Tiny,
    Base,
    Small,
}
impl ModelId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Base => "base",
            Self::Small => "small",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCatalogEntry {
    pub id: ModelId,
    pub filename: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha1: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceModelInfo {
    pub id: &'static str,
    pub filename: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub bytes: u64,
    pub sha1: &'static str,
}
#[derive(Debug, Clone, Serialize)]
pub struct VoiceModelsCatalog {
    pub models: Vec<VoiceModelInfo>,
}

const CATALOG: [ModelCatalogEntry; 3] = [
    ModelCatalogEntry {
        id: ModelId::Tiny,
        filename: "ggml-tiny.bin",
        label: "Tiny",
        description: "Smallest Whisper model with the fastest transcription.",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        bytes: 75 * 1024 * 1024,
        sha1: "bd577a113a864445d4c299885e0cb97d4ba92b5f",
    },
    ModelCatalogEntry {
        id: ModelId::Base,
        filename: "ggml-base.bin",
        label: "Base",
        description: "Balanced Whisper model for speed and accuracy.",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        bytes: 142 * 1024 * 1024,
        sha1: "465707469ff3a37a2b9b8d8f89f2f99de7299dac",
    },
    ModelCatalogEntry {
        id: ModelId::Small,
        filename: "ggml-small.bin",
        label: "Small",
        description: "More accurate Whisper model with a larger footprint.",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        bytes: 466 * 1024 * 1024,
        sha1: "55356645c2b361a969dfd0ef2c5a50d530afd8d5",
    },
];
pub const fn catalog() -> &'static [ModelCatalogEntry] {
    &CATALOG
}
pub fn entry_for_id(value: &str) -> Option<&'static ModelCatalogEntry> {
    let value = value.trim();
    CATALOG
        .iter()
        .find(|e| e.id.as_str().eq_ignore_ascii_case(value))
}
pub fn entry_for_filename(value: &str) -> Option<&'static ModelCatalogEntry> {
    let value = value.trim();
    CATALOG.iter().find(|e| e.filename == value)
}
pub fn entry_for_value(value: &str) -> Option<&'static ModelCatalogEntry> {
    entry_for_id(value).or_else(|| entry_for_filename(value))
}
impl From<&ModelCatalogEntry> for VoiceModelInfo {
    fn from(e: &ModelCatalogEntry) -> Self {
        Self {
            id: e.id.as_str(),
            filename: e.filename,
            label: e.label,
            description: e.description,
            bytes: e.bytes,
            sha1: e.sha1,
        }
    }
}
impl From<&'static [ModelCatalogEntry]> for VoiceModelsCatalog {
    fn from(entries: &'static [ModelCatalogEntry]) -> Self {
        Self {
            models: entries.iter().map(VoiceModelInfo::from).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VoiceModelCatalogPayload {
    pub id: &'static str,
    pub filename: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub bytes: u64,
    pub sha1: &'static str,
}
impl From<&ModelCatalogEntry> for VoiceModelCatalogPayload {
    fn from(e: &ModelCatalogEntry) -> Self {
        Self {
            id: e.id.as_str(),
            filename: e.filename,
            label: e.label,
            description: e.description,
            bytes: e.bytes,
            sha1: e.sha1,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WhisperDownloadProgress {
    pub model: &'static str,
    pub received: u64,
    pub total: u64,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WhisperDownloadStatus {
    pub binary: bool,
    pub models: Vec<String>,
    pub download: Option<WhisperDownloadProgress>,
}
#[derive(Debug, thiserror::Error)]
pub enum VoiceDownloadError {
    #[error("a voice model download is already active")]
    Busy,
    #[error("unknown voice model {0:?}")]
    UnknownModel(String),
    #[error("cannot remove the selected voice model")]
    ActiveModel,
    #[error("download cancelled")]
    Cancelled,
    #[error("download failed: {0}")]
    Download(String),
    #[error("download checksum or size verification failed")]
    Verification,
}

struct ActiveDownload {
    cancel: CancellationToken,
    task: JoinHandle<Result<(), VoiceDownloadError>>,
    progress: Arc<Mutex<WhisperDownloadProgress>>,
}
struct ManagerState {
    active: Option<ActiveDownload>,
    selected: Option<ModelId>,
}
pub struct VoiceModelManager {
    root: PathBuf,
    client: reqwest::Client,
    state: Mutex<ManagerState>,
    app: Mutex<Option<AppHandle>>,
}
impl Default for VoiceModelManager {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceModelManager {
    pub fn new() -> Self {
        Self::at(paths::whisper_models_dir())
    }
    pub fn at(root: PathBuf) -> Self {
        Self {
            root,
            client: reqwest::Client::new(),
            state: Mutex::new(ManagerState {
                active: None,
                selected: None,
            }),
            app: Mutex::new(None),
        }
    }
    pub fn attach_app(&self, app: AppHandle) {
        *self.app.lock() = Some(app);
    }
    pub fn catalog_payload(&self) -> Vec<VoiceModelCatalogPayload> {
        catalog()
            .iter()
            .map(VoiceModelCatalogPayload::from)
            .collect()
    }
    fn reap(&self) {
        if self
            .state
            .lock()
            .active
            .as_ref()
            .is_some_and(|a| a.task.is_finished())
        {
            self.state.lock().active = None;
        }
    }
    pub fn status(&self) -> WhisperDownloadStatus {
        self.reap();
        let download = self
            .state
            .lock()
            .active
            .as_ref()
            .map(|a| a.progress.lock().clone());
        WhisperDownloadStatus {
            binary: stt::WhisperProvider::discover().is_some(),
            models: catalog()
                .iter()
                .filter(|e| self.root.join(e.filename).is_file())
                .map(|e| e.id.as_str().to_string())
                .collect(),
            download,
        }
    }
    pub fn start_download(&self, model: ModelId) -> Result<(), VoiceDownloadError> {
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
        let entry = *entry;
        let progress = Arc::new(Mutex::new(WhisperDownloadProgress {
            model: entry.id.as_str(),
            received: 0,
            total: entry.bytes,
        }));
        let task_progress = Arc::clone(&progress);
        let task = tokio::spawn(async move {
            download(entry, root, client, task_cancel, app, task_progress).await
        });
        state.active = Some(ActiveDownload {
            cancel,
            task,
            progress,
        });
        Ok(())
    }
    pub async fn cancel_download(&self) -> Result<(), VoiceDownloadError> {
        let active = self.state.lock().active.take();
        if let Some(active) = active {
            active.cancel.cancel();
            let _ = active.task.await;
        }
        Ok(())
    }
    pub fn set_selected_model(&self, model: Option<ModelId>) {
        self.state.lock().selected = model;
    }
    pub fn remove_model(&self, model: ModelId) -> Result<(), VoiceDownloadError> {
        self.reap();
        let state = self.state.lock();
        if state.selected == Some(model) {
            return Err(VoiceDownloadError::ActiveModel);
        }
        drop(state);
        let entry = catalog()
            .iter()
            .find(|e| e.id == model)
            .ok_or_else(|| VoiceDownloadError::UnknownModel(model.as_str().into()))?;
        match fs::remove_file(self.root.join(entry.filename)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(VoiceDownloadError::Download(e.to_string())),
        }
    }
}

async fn download(
    entry: ModelCatalogEntry,
    root: PathBuf,
    client: reqwest::Client,
    cancel: CancellationToken,
    app: Option<AppHandle>,
    progress: Arc<Mutex<WhisperDownloadProgress>>,
) -> Result<(), VoiceDownloadError> {
    fs::create_dir_all(&root).map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    let final_path = root.join(entry.filename);
    let tmp_path = root.join(format!("{}.tmp", entry.filename));
    let result = download_inner(
        entry,
        &final_path,
        &tmp_path,
        client,
        cancel.clone(),
        app,
        progress,
    )
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}
async fn download_inner(
    entry: ModelCatalogEntry,
    final_path: &Path,
    tmp_path: &Path,
    client: reqwest::Client,
    cancel: CancellationToken,
    app: Option<AppHandle>,
    progress: Arc<Mutex<WhisperDownloadProgress>>,
) -> Result<(), VoiceDownloadError> {
    let response = tokio::select! { _ = cancel.cancelled() => return Err(VoiceDownloadError::Cancelled), r = client.get(entry.url).send() => r.map_err(|e| VoiceDownloadError::Download(e.to_string()))? };
    if !response.status().is_success() {
        return Err(VoiceDownloadError::Download(format!(
            "HTTP {}",
            response.status()
        )));
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(tmp_path)
        .map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(tmp_path, fs::Permissions::from_mode(0o600))
            .map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    }
    let mut stream = response.bytes_stream();
    let mut received = 0u64;
    let mut hash = Sha1::new();
    while let Some(chunk) = tokio::select! { _ = cancel.cancelled() => return Err(VoiceDownloadError::Cancelled), chunk = stream.next() => chunk }
    {
        let chunk = chunk.map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
        received = received.saturating_add(chunk.len() as u64);
        if received > entry.bytes {
            return Err(VoiceDownloadError::Verification);
        }
        file.write_all(&chunk)
            .map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
        hash.update(&chunk);
        progress.lock().received = received;
        if let Some(app) = &app {
            let _ = app.emit(
                "whisper:download-progress",
                WhisperDownloadProgress {
                    model: entry.id.as_str(),
                    received,
                    total: entry.bytes,
                },
            );
        }
    }
    file.sync_all()
        .map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    let digest = format!("{:x}", hash.finalize());
    if received != entry.bytes || digest != entry.sha1 {
        return Err(VoiceDownloadError::Verification);
    }
    if cancel.is_cancelled() {
        return Err(VoiceDownloadError::Cancelled);
    }
    fs::rename(tmp_path, final_path).map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_exactly_the_approved_fixed_catalog() {
        assert_eq!(catalog().len(), 3);
        assert_eq!(
            catalog().iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["tiny", "base", "small"]
        );
        assert_eq!(catalog()[0].bytes, 75 * 1024 * 1024);
        assert_eq!(catalog()[1].bytes, 142 * 1024 * 1024);
        assert_eq!(catalog()[2].bytes, 466 * 1024 * 1024);
        assert!(catalog()
            .iter()
            .all(|e| e.url.starts_with(HUGGING_FACE_PREFIX) && e.url.starts_with("https://")));
    }
    #[test]
    fn lookup_rejects_arbitrary_ids_urls_and_paths() {
        assert_eq!(entry_for_id(" BASE ").unwrap().id, ModelId::Base);
        for value in [
            "https://example.com/model.bin",
            "../ggml-base.bin",
            "nested/ggml-base.bin",
            r"nested\ggml-base.bin",
            "unsupported.bin",
        ] {
            assert!(entry_for_value(value).is_none());
        }
    }
}
