use super::*;

/// The send's meeting context. A linked send (`Some`) is about THAT
/// listen doc — live or ended — so its persisted summary leads and the
/// WHOLE transcript follows: the summary is already generated over all
/// turns, and the verbatim rows answer "who said what" anywhere in the
/// session. An unlinked ask keeps the ambient behavior: the live
/// listen session's tail, transcript only.
pub(super) fn load_listen_context(db: &Db, listen_id: Option<i64>) -> String {
    let (sid, linked) = match listen_id {
        Some(id) => (id, true),
        None => match db.session_active_id("listen").ok().flatten() {
            Some(id) => (id, false),
            None => return String::new(),
        },
    };
    let rows = if linked {
        db.transcripts_for(sid, None)
    } else {
        db.transcripts_tail(sid, HISTORY_TAIL)
    };
    let transcript = match rows {
        Ok(rows) => rows
            .iter()
            .map(|row: &Transcript| format!("{}: {}", row.speaker, row.content))
            .collect::<Vec<_>>()
            .join("\n"),
        Err(error) => {
            log::warn!("ask: listen history load failed: {error}");
            String::new()
        }
    };
    if !linked {
        return transcript;
    }
    let summary = match db.summary_latest(sid) {
        Ok(summary) => summary.map(|s| {
            let mut text = match s.topic.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                Some(topic) => format!("Topic: {topic}\n"),
                None => String::new(),
            };
            text.push_str(&format!("TLDR: {}", s.tldr));
            for bullet in &s.bullets {
                text.push_str(&format!("\n- {bullet}"));
            }
            text
        }),
        Err(error) => {
            log::warn!("ask: listen summary load failed: {error}");
            None
        }
    };
    match (summary, transcript.is_empty()) {
        (Some(s), false) => format!("Summary:\n{s}\n\nTranscript:\n{transcript}"),
        (Some(s), true) => format!("Summary:\n{s}"),
        (None, _) => transcript,
    }
}

/// Where this send's rows land — resolved by `kick` BEFORE the run
/// registers (the busy gate keys on it), with `send_chain` keeping it
/// as a fallback for direct tests. A `fresh` send (the card was
/// closed) ends the still-open ask session so get_or_create mints a
/// new one. `listen_id` binds to the listen doc's own ask session —
/// one chat per doc, reopened or minted. A `regenerate` keeps the
/// active session — its question IS that session's last user row.
/// `None` means the lookup itself failed — the run still streams, it
/// just has nowhere to persist.
pub(super) fn resolve_session(
    db: &Db,
    fresh_session: bool,
    regenerate: bool,
    listen_id: Option<i64>,
) -> Option<i64> {
    if fresh_session && listen_id.is_none() {
        if let Ok(Some(id)) = db.session_active_id("ask") {
            if let Err(error) = db.session_end(id) {
                log::warn!("ask: session_end before fresh send failed: {error}");
            }
        }
    }
    match (regenerate, listen_id) {
        (false, Some(lid)) => match db.ask_session_for_listen(lid) {
            Ok(id) => Some(id),
            Err(error) => {
                log::warn!("ask: linked session resolve failed: {error}");
                None
            }
        },
        _ => open_ask_session(db),
    }
}

/// The active `ask` session id, or `None` when the lookup itself fails —
/// history and the assistant row then have nowhere to go.
pub(super) fn open_ask_session(db: &Db) -> Option<i64> {
    match db.session_get_or_create_active("ask") {
        Ok(sid) => Some(sid),
        Err(e) => {
            log::warn!("ask: session_get_or_create_active failed: {e}");
            None
        }
    }
}

/// The session's persisted `messages` rows (attachments already joined)
/// — the send's history source. `None` session or a failed lookup
/// yields an empty tail: a storage hiccup must not block the stream.
pub(super) fn message_rows(db: &Db, session_id: Option<i64>) -> Vec<crate::storage::Message> {
    let Some(sid) = session_id else {
        return Vec::new();
    };
    match db.messages_for(sid) {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!("ask: history load failed: {e}");
            Vec::new()
        }
    }
}

/// The managed JPEG bytes for one stored attachment. The stored path
/// must resolve under `root` — a row pointing elsewhere is corrupt
/// metadata, not a file to open. A missing file is an error for the
/// caller to surface, never a silent skip.
pub(super) fn read_attachment(root: &Path, att: &MessageAttachment) -> Result<Vec<u8>, String> {
    let path = PathBuf::from(&att.path);
    // `starts_with` is component-wise — a `..` segment would still
    // prefix-match, so reject parent traversal explicitly too.
    let escapes = !path.starts_with(root)
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
    if escapes {
        return Err(format!("Attachment \"{}\" has an invalid path", att.name));
    }
    std::fs::read(&path).map_err(|_| {
        format!(
            "Attachment \"{}\" is missing — the image can't be sent",
            att.name
        )
    })
}

/// Rows → the trailing `ChatMessage` tail — at most `HISTORY_TAIL`
/// rows, user/assistant only. A user row's attachments re-read as
/// `ImageJpeg` parts (same wire shape as the live send); their file
/// reads are why this runs inside `spawn_blocking`.
pub(super) fn rows_to_history_at(
    rows: &[crate::storage::Message],
    root: &Path,
) -> Result<Vec<ChatMessage>, String> {
    rows.iter()
        .skip(rows.len().saturating_sub(HISTORY_TAIL))
        .filter_map(|m| match m.role.as_str() {
            "user" => {
                let mut text = m.content.clone();
                let mut images = Vec::new();
                for attachment in &m.attachments {
                    match read_attachment(root, attachment) {
                        Ok(image) => images.push(image),
                        Err(error) => {
                            log::warn!("ask: history attachment unavailable: {error}");
                            text.push_str(&format!(
                                "\n[Attachment unavailable: {}]",
                                attachment.name
                            ));
                        }
                    }
                }
                Some(Ok(ChatMessage::user_with_images(text, images)))
            }
            "assistant" => Some(Ok(ChatMessage::text(Role::Assistant, m.content.clone()))),
            _ => None,
        })
        .collect()
}

/// `regenerate` inputs: history ROWS before the session's last user
/// turn — the row being re-asked — plus that row's id and attachments
/// (their files re-read by the caller as the turn's images; the id is
/// where a retry's fresh screenshot attaches). Every later row (the
/// rejected reply) is deleted so a reload never replays it. `None`
/// when the session has no user turn to re-ask or the lookup failed.
pub(super) fn regenerate_tail_rows(
    db: &Db,
    session_id: Option<i64>,
) -> Option<(Vec<crate::storage::Message>, i64, Vec<MessageAttachment>)> {
    let sid = session_id?;
    let rows = db.messages_for(sid).ok()?;
    let cut = rows.iter().rposition(|r| r.role == "user")?;
    let re_asked_id = rows[cut].id;
    let re_asked_attachments = rows[cut].attachments.clone();
    let rejected_is_compacted = match db.session_compaction(sid) {
        Ok(Some((_, _, Some(compact_through)))) => {
            rows[cut + 1..].iter().any(|row| row.id <= compact_through)
        }
        Ok(Some((_, _, None)) | None) => false,
        Err(error) => {
            log::warn!("ask: session compaction watermark load failed: {error}");
            false
        }
    };
    for row in &rows[cut + 1..] {
        if let Err(e) = db.message_delete(row.id) {
            log::warn!("ask: failed to drop rejected reply {}: {e}", row.id);
        }
    }
    if rejected_is_compacted {
        if let Err(error) = db.session_compact_clear(sid) {
            log::warn!("ask: session compaction clear after regenerate failed: {error}");
        }
    }
    Some((rows[..cut].to_vec(), re_asked_id, re_asked_attachments))
}

/// The new user row, next to its session; returns its id for the
/// attachment link-up. `preset` records the armed instruct preset
/// (presets.rs id) so `retry` re-resolves the same steering. `None`
/// session (lookup failed) or a failed insert yields `None` — the
/// stream must not die on a storage hiccup (attachment sends DO die:
/// their images need the row to link to).
pub(super) fn persist_user_message(
    db: &Db,
    session_id: Option<i64>,
    text: &str,
    preset: Option<&str>,
) -> Option<i64> {
    let sid = session_id?;
    let meta = MessageMeta {
        preset: preset.map(str::to_string),
        ..MessageMeta::default()
    };
    match db.message_add_meta(sid, "user", text, &meta) {
        Ok(id) => Some(id),
        Err(e) => {
            log::warn!("ask: failed to persist user message: {e}");
            None
        }
    }
}

/// The completed assistant reply, next to its user row — provenance and
/// token spend ride along for the card's per-message ⋯ menu.
pub(super) fn persist_assistant_message(
    db: &Db,
    session_id: Option<i64>,
    full: &str,
    provider: &str,
    model: &str,
    usage: Option<TokenUsage>,
) {
    let Some(sid) = session_id else {
        return;
    };
    let meta = MessageMeta {
        provider: Some(provider.to_string()),
        model: Some(model.to_string()),
        tokens_in: usage.and_then(|u| u.input.map(|n| n as i64)),
        tokens_out: usage.and_then(|u| u.output.map(|n| n as i64)),
        // Armed presets ride user rows, not assistant replies.
        preset: None,
    };
    if let Err(e) = db.message_add_meta(sid, "assistant", full, &meta) {
        log::warn!("ask: failed to persist assistant message: {e}");
    }
}
