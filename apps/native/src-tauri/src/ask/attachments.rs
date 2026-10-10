use super::*;

/// A decoded composer image — `name` is display metadata only; `jpeg`
/// is the validated payload written to a managed file.
pub(super) struct PendingImage {
    pub(super) name: String,
    pub(super) jpeg: Vec<u8>,
}

/// Decode and sniff one `jpegBase64` input. The webview already
/// normalized it to JPEG — anything that isn't valid base64 or JPEG
/// magic is a malformed (or crafted) invoke and must fail the send
/// rather than reach a provider half-specified.
pub(super) fn decode_attachment(input: &AskAttachmentInput) -> Result<PendingImage, String> {
    use base64::Engine;
    if input.jpeg_base64.len() > MAX_ATTACHMENT_BYTES * 4 / 3 + 8 {
        return Err(format!("Attachment \"{}\" is too large", input.name));
    }
    let jpeg = base64::engine::general_purpose::STANDARD
        .decode(input.jpeg_base64.trim())
        .map_err(|_| format!("Attachment \"{}\" isn't valid base64", input.name))?;
    if jpeg.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!("Attachment \"{}\" is too large", input.name));
    }
    if !jpeg.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Err(format!("Attachment \"{}\" isn't a JPEG image", input.name));
    }
    Ok(PendingImage {
        name: input.name.clone(),
        jpeg,
    })
}

/// Generated storage name — never the user's filename (no traversal,
/// no collisions): nanos plus a process counter, `att-*.jpg`.
pub(super) fn attachment_filename() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("att-{nanos}-{}.jpg", N.fetch_add(1, Ordering::Relaxed))
}

/// Write every pending image under `root` with generated names and
/// owner-only Unix permissions (same 0600 as keys.json/marvis.db — the
/// 0700 root is the real gate, this is the standing convention). A
/// partial failure unlinks what it wrote — callers see all-or-nothing.
pub(super) fn write_attachment_files(root: &Path, jpegs: &[Vec<u8>]) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(root)?;
    let mut paths = Vec::with_capacity(jpegs.len());
    for jpeg in jpegs {
        let path = root.join(attachment_filename());
        if let Err(e) = std::fs::write(&path, jpeg) {
            for written in &paths {
                let _ = std::fs::remove_file(written);
            }
            return Err(e);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        paths.push(path);
    }
    Ok(paths)
}

/// Persist `pending` under `root`, link its rows to `message_id`, and
/// hand back (request bytes, metadata) — the provider turn reads the
/// same JPEGs the message row carries. Any failure unlinks the files
/// it wrote; the caller drops the user row on `Err`.
pub(super) async fn persist_attachments(
    db: &Db,
    root: &Path,
    message_id: i64,
    pending: Vec<PendingImage>,
) -> Result<(Vec<Vec<u8>>, Vec<MessageAttachment>), String> {
    let names: Vec<String> = pending.iter().map(|p| p.name.clone()).collect();
    let jpegs: Vec<Vec<u8>> = pending.into_iter().map(|p| p.jpeg).collect();
    let user_images = jpegs.clone();
    let root_owned = root.to_path_buf();
    // Blocking fs off the async executor (same rule as the one-shot shot).
    let paths = tokio::task::spawn_blocking(move || {
        write_attachment_files(&root_owned, &jpegs)
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r.map_err(|e| format!("Couldn't save an attachment: {e}")))?;
    // Positions continue past the row's existing attachments — a
    // retry's fresh screenshot appends rather than colliding at 0.
    let offset = db
        .attachments_for(message_id)
        .map(|a| a.len() as i64)
        .unwrap_or(0);
    let rows: Vec<NewAttachment> = names
        .iter()
        .zip(&paths)
        .zip(&user_images)
        .enumerate()
        .map(|(i, ((name, path), jpeg))| NewAttachment {
            name: name.clone(),
            path: path.to_string_lossy().into_owned(),
            mime: "image/jpeg".to_string(),
            bytes: jpeg.len() as i64,
            position: offset + i as i64,
        })
        .collect();
    match db.attachments_add(message_id, &rows) {
        Ok(meta) => Ok((user_images, meta)),
        Err(e) => {
            for path in &paths {
                let _ = std::fs::remove_file(path);
            }
            Err(format!("Couldn't record an attachment: {e}"))
        }
    }
}

/// The `loading` payload — `attachments` rides along only when the
/// turn carries persisted images (composer picks or a screenshot).
/// `attempt` is the 0-based failover index (0 = the run's start emit)
/// and `regenerate` marks `ask_retry`'s re-ask (`re_asked` — a regen
/// that found no user tail sends normally, so it reports false): the
/// card's fold tells retry/re-ask apart from a fresh turn by these
/// fields, not by matching the question text.
pub(super) fn make_loading(
    text: &str,
    preset_id: Option<&str>,
    run_attachments: &[MessageAttachment],
    attempt: usize,
    regenerate: bool,
) -> serde_json::Value {
    let mut loading = json!({
        "state": "loading",
        "question": text,
        "preset": preset_id,
        "attempt": attempt,
        "regenerate": regenerate,
    });
    if !run_attachments.is_empty() {
        loading["attachments"] = json!(run_attachments);
    }
    loading
}

/// The `resolve_screen` Err arm's shape for pre-chain attachment
/// failures: `ask:error` + `idle`, then the status-0 sentinel.
pub(super) fn attachment_error(
    emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync),
    message: String,
) -> LlmError {
    emit(EV_ERROR, json!({ "message": message }));
    emit(EV_STATE, json!({"state": "idle"}));
    LlmError::Http {
        status: 0,
        message,
    }
}

