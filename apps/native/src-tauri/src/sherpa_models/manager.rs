use super::*;
use super::catalog::*;
use super::download::*;

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

pub(super) struct ActiveDownload {
    pub(super) cancel: CancellationToken,
    pub(super) task: Option<JoinHandle<()>>,
    pub(super) progress: Arc<Mutex<SherpaDownloadProgress>>,
}
pub(super) struct ManagerState {
    pub(super) active: Option<ActiveDownload>,
    pub(super) selected: Option<SherpaModelId>,
    /// Punctuation entries already auto-attempted this run — one shot
    /// each so a failing add-on doesn't retry on every refresh.
    pub(super) punct_attempted: Vec<SherpaModelId>,
}
pub struct SherpaModelManager {
    pub(super) root: PathBuf,
    pub(super) client: reqwest::Client,
    pub(super) state: Arc<Mutex<ManagerState>>,
    pub(super) app: Mutex<Option<AppHandle>>,
    pub(super) cancel_gate: tokio::sync::Mutex<()>,
    pub(super) install_gate: Arc<tokio::sync::Mutex<()>>,
    #[cfg(test)]
    pub(super) test_files: Option<Vec<TestSource>>,
}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct TestSource {
    pub(super) url: String,
    pub(super) bytes: u64,
    pub(super) sha256: String,
}
impl Default for SherpaModelManager {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn reclaim_catalog_temps(root: &Path) {
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

pub(super) fn expected_size_bounds(display_bytes: u64) -> (u64, u64) {
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
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .read_timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("download HTTP client"),
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
    pub(super) fn with_test_files(
        root: PathBuf,
        model: SherpaModelId,
        files: Vec<TestSource>,
    ) -> Self {
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
pub(super) async fn download(
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
