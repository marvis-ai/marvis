use super::*;
use super::catalog::*;

pub(super) struct TempFileGuard<'a> {
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
pub(super) async fn download_inner(
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

pub(super) fn terminal_progress(model: &'static str, received: u64) -> SherpaDownloadProgress {
    SherpaDownloadProgress {
        model,
        received,
        total: received,
    }
}

pub(super) fn emit_progress(app: &Option<AppHandle>, progress: SherpaDownloadProgress) {
    if let Some(app) = app {
        let _ = app.emit(EV_SHERPA_DOWNLOAD_PROGRESS, progress);
    }
}

pub(super) fn emit_error(app: &Option<AppHandle>, model: &'static str, error: &VoiceDownloadError) {
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

