use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use sha1::{Digest, Sha1};
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{async_runtime::JoinHandle, AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use crate::{paths, stt};

pub const HUGGING_FACE_PREFIX: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";
const CATALOG_SOURCE: &str = "Hugging Face · ggerganov/whisper.cpp";
const SIZE_TOLERANCE_PERCENT: u64 = 10;

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
    pub source: &'static str,
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
            source: CATALOG_SOURCE,
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
    pub source: &'static str,
}
impl From<&ModelCatalogEntry> for VoiceModelCatalogPayload {
    fn from(e: &ModelCatalogEntry) -> Self {
        Self {
            id: e.id.as_str(),
            filename: e.filename,
            label: e.label,
            description: e.description,
            bytes: e.bytes,
            source: CATALOG_SOURCE,
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
pub struct WhisperInstalledModel {
    pub id: &'static str,
    pub filename: &'static str,
    pub installed: bool,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WhisperDownloadStatus {
    pub binary: Option<String>,
    pub models: Vec<WhisperInstalledModel>,
    pub download: Option<WhisperDownloadProgress>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WhisperDownloadError {
    pub model: &'static str,
    pub message: &'static str,
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
    task: Option<JoinHandle<()>>,
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
    cancel_gate: tokio::sync::Mutex<()>,
    install_gate: Arc<tokio::sync::Mutex<()>>,
    #[cfg(test)]
    test_source: Option<TestSource>,
}

#[cfg(test)]
#[derive(Clone)]
struct TestSource {
    url: String,
    bytes: u64,
    sha1: String,
}
impl Default for VoiceModelManager {
    fn default() -> Self {
        Self::new()
    }
}

fn reclaim_catalog_temps(root: &Path) {
    for entry in catalog() {
        let tmp = root.join(format!("{}.tmp", entry.filename));
        // Only exact catalog-owned names are ever reclaimed; unrelated temp files
        // remain untouched. A stale catalog temp cannot be resumed safely because
        // its download identity is not persisted.
        if tmp.is_file() {
            let _ = fs::remove_file(tmp);
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

impl VoiceModelManager {
    pub fn new() -> Self {
        Self::at(paths::whisper_models_dir())
    }
    pub fn at(root: PathBuf) -> Self {
        reclaim_catalog_temps(&root);
        Self {
            root,
            client: reqwest::Client::new(),
            state: Mutex::new(ManagerState {
                active: None,
                selected: None,
            }),
            app: Mutex::new(None),
            cancel_gate: tokio::sync::Mutex::new(()),
            install_gate: Arc::new(tokio::sync::Mutex::new(())),
            #[cfg(test)]
            test_source: None,
        }
    }
    #[cfg(test)]
    fn with_test_source(root: PathBuf, source: TestSource) -> Self {
        let mut manager = Self::at(root);
        manager.test_source = Some(source);
        manager
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
            .and_then(|a| a.task.as_ref())
            .is_some_and(|task| task.inner().is_finished())
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
            binary: stt::WhisperProvider::discover().map(|path| path.display().to_string()),
            models: catalog()
                .iter()
                .map(|e| WhisperInstalledModel {
                    id: e.id.as_str(),
                    filename: e.filename,
                    installed: self.root.join(e.filename).is_file(),
                    bytes: e.bytes,
                })
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
        let install_gate = Arc::clone(&self.install_gate);
        #[cfg(test)]
        let test_source = self.test_source.clone();
        let entry = *entry;
        let progress = Arc::new(Mutex::new(WhisperDownloadProgress {
            model: entry.id.as_str(),
            received: 0,
            total: entry.bytes,
        }));
        let task_progress = Arc::clone(&progress);
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
                test_source,
            )
            .await;
            if let Err(error) = result {
                emit_error(&app, entry.id.as_str(), &error);
            }
        });
        state.active = Some(ActiveDownload {
            cancel,
            task: Some(task),
            progress,
        });
        Ok(())
    }
    pub async fn cancel_download(&self) -> Result<(), VoiceDownloadError> {
        let _gate = self.cancel_gate.lock().await;
        let task = {
            let mut state = self.state.lock();
            let Some(active) = state.active.as_mut() else {
                return Ok(());
            };
            active.cancel.cancel();
            active.task.take()
        };
        if let Some(task) = task {
            let _ = task.await;
        }
        self.state.lock().active = None;
        Ok(())
    }
    pub fn set_selected_model(&self, model: Option<ModelId>) {
        self.state.lock().selected = model;
    }
    pub fn remove_model(&self, model: ModelId) -> Result<(), VoiceDownloadError> {
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
        match fs::remove_file(self.root.join(entry.filename)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(VoiceDownloadError::Download(e.to_string())),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn download(
    entry: ModelCatalogEntry,
    root: PathBuf,
    client: reqwest::Client,
    cancel: CancellationToken,
    app: Option<AppHandle>,
    progress: Arc<Mutex<WhisperDownloadProgress>>,
    install_gate: Arc<tokio::sync::Mutex<()>>,
    #[cfg(test)] test_source: Option<TestSource>,
) -> Result<(), VoiceDownloadError> {
    fs::create_dir_all(&root).map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    let final_path = root.join(entry.filename);
    let tmp_path = root.join(format!("{}.tmp", entry.filename));
    download_inner(
        entry,
        &final_path,
        &tmp_path,
        client,
        cancel,
        app,
        progress,
        install_gate,
        #[cfg(test)]
        test_source,
    )
    .await
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

#[allow(clippy::too_many_arguments)]
async fn download_inner(
    entry: ModelCatalogEntry,
    final_path: &Path,
    tmp_path: &Path,
    client: reqwest::Client,
    cancel: CancellationToken,
    app: Option<AppHandle>,
    progress: Arc<Mutex<WhisperDownloadProgress>>,
    install_gate: Arc<tokio::sync::Mutex<()>>,
    #[cfg(test)] test_source: Option<TestSource>,
) -> Result<(), VoiceDownloadError> {
    let (url, expected_size, expected_sha1) = {
        #[cfg(test)]
        if let Some(source) = test_source {
            (source.url, source.bytes, source.sha1)
        } else {
            (entry.url.to_string(), entry.bytes, entry.sha1.to_string())
        }
        #[cfg(not(test))]
        (entry.url.to_string(), entry.bytes, entry.sha1.to_string())
    };
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
    let mut file = file_options
        .open(tmp_path)
        .map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    let mut guard = TempFileGuard {
        path: tmp_path,
        owned: true,
    };
    let mut stream = response.bytes_stream();
    let mut received = 0u64;
    let mut hash = Sha1::new();
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
        file.write_all(&chunk)
            .map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
        hash.update(&chunk);
        progress.lock().received = received;
        if last_progress.elapsed() >= Duration::from_millis(150) {
            emit_progress(
                &app,
                WhisperDownloadProgress {
                    model: entry.id.as_str(),
                    received,
                    total: entry.bytes,
                },
            );
            last_progress = Instant::now();
        }
    }
    file.sync_all()
        .map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    let digest = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let (min_size, max_size) = expected_size_bounds(expected_size);
    if received < min_size || received > max_size || digest != expected_sha1 {
        return Err(VoiceDownloadError::Verification);
    }
    // Serialize the cancellation decision with the final rename. If cancellation
    // gets this gate first, no install occurs; if rename gets it first, cancel
    // waits and returns only after the installed file is authoritative.
    let _install = install_gate.lock().await;
    if cancel.is_cancelled() {
        return Err(VoiceDownloadError::Cancelled);
    }
    // Completion is authoritative: the catalog size is display metadata and may
    // differ from the bytes actually returned by Hugging Face.
    emit_progress(&app, terminal_progress(entry.id.as_str(), received));
    fs::rename(tmp_path, final_path).map_err(|e| VoiceDownloadError::Download(e.to_string()))?;
    guard.owned = false;
    Ok(())
}

fn terminal_progress(model: &'static str, received: u64) -> WhisperDownloadProgress {
    WhisperDownloadProgress {
        model,
        received,
        total: received,
    }
}

fn emit_progress(app: &Option<AppHandle>, progress: WhisperDownloadProgress) {
    if let Some(app) = app {
        let _ = app.emit("whisper:download-progress", progress);
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
        VoiceDownloadError::Busy => "download is already active",
        VoiceDownloadError::UnknownModel(_) => "unknown voice model",
        VoiceDownloadError::ActiveModel => "cannot remove the selected voice model",
    };
    if let Some(app) = app {
        let _ = app.emit(
            "whisper:download-error",
            WhisperDownloadError { model, message },
        );
    }
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

    #[test]
    fn progress_dto_has_only_the_public_fields() {
        let value = serde_json::to_value(WhisperDownloadProgress {
            model: "tiny",
            received: 1,
            total: 2,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({"model": "tiny", "received": 1, "total": 2})
        );
    }

    #[test]
    fn terminal_progress_uses_authoritative_received_bytes() {
        let progress = terminal_progress("tiny", 123_456_789);
        assert_eq!(progress.received, 123_456_789);
        assert_eq!(progress.total, progress.received);
    }

    #[test]
    fn catalog_and_status_dtos_have_safe_documented_shapes() {
        let catalog = serde_json::to_value(VoiceModelCatalogPayload::from(&catalog()[0])).unwrap();
        assert_eq!(catalog["source"], CATALOG_SOURCE);
        assert!(catalog.get("url").is_none());
        assert!(catalog.get("sha1").is_none());

        let status = serde_json::to_value(WhisperDownloadStatus {
            binary: Some("/usr/local/bin/whisper-cli".into()),
            models: vec![WhisperInstalledModel {
                id: "tiny",
                filename: "ggml-tiny.bin",
                installed: true,
                bytes: 75,
            }],
            download: None,
        })
        .unwrap();
        assert_eq!(
            status,
            serde_json::json!({
                "binary": "/usr/local/bin/whisper-cli",
                "models": [{"id": "tiny", "filename": "ggml-tiny.bin", "installed": true, "bytes": 75}],
                "download": null
            })
        );
    }

    #[test]
    fn download_error_dto_has_only_safe_fields() {
        let value = serde_json::to_value(WhisperDownloadError {
            model: "tiny",
            message: "download failed",
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({"model": "tiny", "message": "download failed"})
        );
    }

    fn fixture(body: Vec<u8>, delay: Duration) -> (String, TestSource) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let expected = body.clone();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
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
        let mut sha = Sha1::new();
        sha.update(&body);
        let digest = sha
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        (
            format!("http://{address}/model"),
            TestSource {
                url: format!("http://{address}/model"),
                bytes: body.len() as u64,
                sha1: digest,
            },
        )
    }

    fn temp_root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir();
        loop {
            let root = base.join(format!(
                "marvis-voice-{}-{}-{}",
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
        let (_, source) = fixture(b"sync start fixture".to_vec(), Duration::ZERO);
        let manager = VoiceModelManager::with_test_source(root.clone(), source);

        manager.start_download(ModelId::Tiny).unwrap();
        tauri::async_runtime::block_on(manager.cancel_download()).unwrap();

        assert!(!root.join("ggml-tiny.bin.tmp").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn lifecycle_verifies_cleans_preserves_and_serializes_downloads() {
        let body = b"voice model fixture".to_vec();
        let (url, source) = fixture(body.clone(), Duration::from_millis(10));
        assert!(url.starts_with("http://127.0.0.1:"));
        let root = temp_root();
        let manager = VoiceModelManager::with_test_source(root.clone(), source.clone());
        manager.start_download(ModelId::Tiny).unwrap();
        assert!(matches!(
            manager.start_download(ModelId::Base),
            Err(VoiceDownloadError::Busy)
        ));
        manager.cancel_download().await.unwrap();
        assert!(!root.join("ggml-tiny.bin.tmp").exists());

        let (_, bad_source) = fixture(body.clone(), Duration::ZERO);
        let bad = VoiceModelManager::with_test_source(
            root.clone(),
            TestSource {
                sha1: "00".repeat(20),
                ..bad_source
            },
        );
        fs::write(root.join("ggml-tiny.bin"), b"keep me").unwrap();
        bad.start_download(ModelId::Tiny).unwrap();
        while bad.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert_eq!(fs::read(root.join("ggml-tiny.bin")).unwrap(), b"keep me");
        assert!(!root.join("ggml-tiny.bin.tmp").exists());

        let (_, bad_size_source) = fixture(body.clone(), Duration::ZERO);
        let bad_size = VoiceModelManager::with_test_source(
            root.clone(),
            TestSource {
                bytes: u64::MAX / 2,
                ..bad_size_source
            },
        );
        bad_size.start_download(ModelId::Tiny).unwrap();
        while bad_size.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(!root.join("ggml-tiny.bin.tmp").exists());

        let (_, success_source) = fixture(body.clone(), Duration::ZERO);
        let success = VoiceModelManager::with_test_source(root.clone(), success_source);
        success.start_download(ModelId::Tiny).unwrap();
        while success.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert_eq!(fs::read(root.join("ggml-tiny.bin")).unwrap(), body);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(root.join("ggml-tiny.bin"))
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
        fs::write(root.join("ggml-tiny.bin.tmp"), b"stale").unwrap();
        fs::write(root.join("foreign.tmp"), b"keep").unwrap();
        let _manager = VoiceModelManager::at(root.clone());
        assert!(!root.join("ggml-tiny.bin.tmp").exists());
        assert!(root.join("foreign.tmp").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn active_model_removal_is_rejected() {
        let manager = VoiceModelManager::at(temp_root());
        manager.set_selected_model(Some(ModelId::Tiny));
        assert!(matches!(
            manager.remove_model(ModelId::Tiny),
            Err(VoiceDownloadError::ActiveModel)
        ));
        manager.set_selected_model(None);
        let _ = fs::remove_dir_all(manager.root);
    }

    #[tokio::test]
    async fn removal_is_rejected_while_download_is_active() {
        let root = temp_root();
        let (_, source) = fixture(b"fixture".to_vec(), Duration::from_millis(100));
        let manager = VoiceModelManager::with_test_source(root.clone(), source);
        manager.start_download(ModelId::Tiny).unwrap();
        assert!(matches!(
            manager.remove_model(ModelId::Tiny),
            Err(VoiceDownloadError::Busy)
        ));
        manager.cancel_download().await.unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancellation_racing_final_install_has_one_authoritative_outcome() {
        let root = temp_root();
        let body = b"race fixture".to_vec();
        let (_, source) = fixture(body.clone(), Duration::ZERO);
        let manager = Arc::new(VoiceModelManager::with_test_source(root.clone(), source));
        manager.start_download(ModelId::Tiny).unwrap();
        let canceller = Arc::clone(&manager);
        canceller.cancel_download().await.unwrap();
        assert!(!root.join("ggml-tiny.bin.tmp").exists());
        if root.join("ggml-tiny.bin").exists() {
            assert_eq!(fs::read(root.join("ggml-tiny.bin")).unwrap(), body);
        }
        assert!(manager.status().download.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancellation_does_not_remove_preexisting_temp_file() {
        let root = temp_root();
        let tmp = root.join("foreign.tmp");
        fs::write(&tmp, b"owned by someone else").unwrap();
        let (_, source) = fixture(b"fixture".to_vec(), Duration::ZERO);
        let manager = VoiceModelManager::with_test_source(root.clone(), source);
        manager.start_download(ModelId::Tiny).unwrap();
        manager.cancel_download().await.unwrap();
        assert_eq!(fs::read(tmp).unwrap(), b"owned by someone else");
        let _ = fs::remove_dir_all(root);
    }
}
