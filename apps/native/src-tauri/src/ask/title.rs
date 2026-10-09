use super::*;

/// A still-untitled session gets named by the provider that just
/// answered — a small `stream_chat` for a short title over the raw
/// question, then `session_set_title`'s `title IS NULL` write guard
/// makes it first-wins (a failed call retries on the next send, a
/// landed one is never redone). Failure is silent to the run: the
/// history row keeps the first-question fallback. `sessions:changed`
/// goes out through the run's gen-guarded `emit` — a stale run's ping
/// drops, but the next run's `idle` refresh picks the title up anyway.
/// Only the first 1,000 Unicode scalar values of `question` are sent.
/// The provider call has a 30-second deadline; cancellation, timeout,
/// provider/storage errors, or an empty cleaned title leave it unwritten.
pub(super) async fn maybe_title_session(
    db: &Db,
    provider: &dyn Provider,
    session_id: Option<i64>,
    question: &str,
    emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync),
    cancel: &CancellationToken,
) {
    let Some(sid) = session_id else { return };
    match db.session_title(sid) {
        Ok(None) => {}
        Ok(Some(_)) => return, // already named — never retitle
        Err(e) => {
            log::warn!("ask: session title read failed: {e}");
            return;
        }
    }
    let question: String = question.chars().take(TITLE_QUESTION_CAP).collect();
    let msgs = [
        ChatMessage::text(Role::System, TITLE_PROMPT),
        ChatMessage::text(Role::User, question),
    ];
    let mut sink = |_: &str| {};
    let reply = tokio::select! {
        _ = cancel.cancelled() => return,
        r = tokio::time::timeout(TITLE_TIMEOUT, provider.stream_chat(&msgs, &mut sink)) => r,
    };
    let title = match reply {
        Ok(Ok(r)) => clean_title(&r.full),
        Ok(Err(e)) => {
            log::warn!("ask: title generation failed: {e}");
            return;
        }
        Err(_) => {
            log::warn!("ask: title generation timed out");
            return;
        }
    };
    if title.is_empty() {
        return;
    }
    match db.session_set_title(sid, &title) {
        Ok(true) => emit(EV_SESSIONS_CHANGED, json!({"id": sid})),
        Ok(false) => {}
        Err(e) => log::warn!("ask: session title write failed: {e}"),
    }
}

/// Turn the trimmed first reply line into a label: strip `Title:` or
/// `title:`, then edge double quotes/backticks, then trailing `. … 。 ! ！ ? ？`
/// and whitespace. Cap at 60 Unicode scalar values. Empty means "don't write".
pub(super) fn clean_title(raw: &str) -> String {
    let mut line = raw.lines().next().unwrap_or_default().trim();
    if let Some(rest) = line
        .strip_prefix("Title:")
        .or_else(|| line.strip_prefix("title:"))
    {
        line = rest.trim();
    }
    line = line
        .trim_matches(|c: char| c == '"' || c == '`')
        .trim_end_matches(['.', '…', '。', '!', '！', '?', '？'])
        .trim_end();
    line.chars().take(60).collect()
}

