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
use crate::voice_models::{VoiceDownloadError, WhisperDownloadError};

pub const CATALOG_SOURCE: &str = "Hugging Face · csukuangfj + k2-fsa/sherpa-onnx";
const SIZE_TOLERANCE_PERCENT: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SherpaModelId {
    SenseVoice,
}
impl SherpaModelId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SenseVoice => "sense-voice",
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
    pub source: &'static str,
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

const CATALOG: [SherpaCatalogEntry; 1] = [SherpaCatalogEntry {
    id: SherpaModelId::SenseVoice,
    dirname: "sense-voice",
    label: "SenseVoice",
    description: "Multilingual SenseVoice recognition (zh/en/ja/ko/yue) segmented by Silero VAD.",
    files: SENSE_VOICE_FILES,
    source: CATALOG_SOURCE,
}];

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
/// `root/<dirname>/` — the directory that owns every file of an entry.
pub fn entry_dir(root: &Path, entry: &SherpaCatalogEntry) -> PathBuf {
    root.join(entry.dirname)
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
    pub source: &'static str,
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
}
pub struct SherpaModelManager {
    root: PathBuf,
    client: reqwest::Client,
    state: Mutex<ManagerState>,
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
            state: Mutex::new(ManagerState {
                active: None,
                selected: None,
            }),
            app: Mutex::new(None),
            cancel_gate: tokio::sync::Mutex::new(()),
            install_gate: Arc::new(tokio::sync::Mutex::new(())),
            #[cfg(test)]
            test_files: None,
        }
    }
    #[cfg(test)]
    fn with_test_files(root: PathBuf, files: Vec<TestSource>) -> Self {
        // `download()` consumes the override positionally, one `TestSource`
        // per entry file — a short vec would silently fall back to the pinned
        // production URLs, so require a complete set at construction.
        assert!(
            catalog()
                .iter()
                .all(|entry| files.len() == entry.files.len()),
            "test override must supply one TestSource per file of the catalog entry"
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
                    source: e.source,
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
        let _ = app.emit("sherpa:download-progress", progress);
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
            "sherpa:download-error",
            WhisperDownloadError { model, message },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

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
        assert!(entry.files.iter().all(|f| f.url.starts_with("https://")
            && f.sha256.len() == 64
            && f.sha256.chars().all(|c| c.is_ascii_hexdigit())));
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

    fn fixture(body: Vec<u8>, delay: Duration) -> TestSource {
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
        let manager = SherpaModelManager::with_test_files(root.clone(), sources);

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
        let manager = SherpaModelManager::with_test_files(root.clone(), sources);
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
        let bad = SherpaModelManager::with_test_files(root.clone(), bad_sources);
        bad.start_download(SherpaModelId::SenseVoice).unwrap();
        while bad.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert_eq!(fs::read(dir.join("model.int8.onnx")).unwrap(), b"keep me");
        assert!(!dir.join("model.int8.onnx.tmp").exists());

        // A size far outside the ±10% band aborts the install as well.
        let mut bad_size_sources = fixture_set(&bodies, Duration::ZERO);
        bad_size_sources[0].bytes = u64::MAX / 2;
        let bad_size = SherpaModelManager::with_test_files(root.clone(), bad_size_sources);
        bad_size.start_download(SherpaModelId::SenseVoice).unwrap();
        while bad_size.status().download.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(!dir.join("model.int8.onnx.tmp").exists());
        assert_eq!(fs::read(dir.join("model.int8.onnx")).unwrap(), b"keep me");

        // Success requires every file in the set to land, content intact.
        let success_sources = fixture_set(&bodies, Duration::ZERO);
        let success = SherpaModelManager::with_test_files(root.clone(), success_sources);
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
        let manager = SherpaModelManager::with_test_files(root.clone(), sources);
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
        let manager = Arc::new(SherpaModelManager::with_test_files(root.clone(), sources));
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
        let manager = SherpaModelManager::with_test_files(root.clone(), sources);
        manager.start_download(SherpaModelId::SenseVoice).unwrap();
        manager.cancel_download().await.unwrap();
        assert_eq!(fs::read(tmp).unwrap(), b"owned by someone else");
        let _ = fs::remove_dir_all(root);
    }
}
