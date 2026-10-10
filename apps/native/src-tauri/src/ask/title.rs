use super::*;

/// The detached title job handed to `send_chain` — mirrors
/// `MemoryHook`: it owns the `Db` share and the `sessions:changed`
/// emit because the spawned task outlives the run (the answering
/// provider is only known at schedule time, so it's a `schedule`
/// argument). Unlike the old awaited call, `stop` can't recall it —
/// a "New chat" mid-title still names the ended session.
pub(super) struct TitleSidecar {
    db: Arc<Db>,
    titled: Arc<dyn Fn(i64) + Send + Sync>,
}

impl TitleSidecar {
    pub(super) fn new(db: Arc<Db>, titled: Arc<dyn Fn(i64) + Send + Sync>) -> Self {
        Self { db, titled }
    }

    /// Fire-and-forget: the ask task never waits on it. The
    /// still-untitled check runs here so an already-named session —
    /// every send after the first — never spawns; the write-side
    /// `title IS NULL` guard keeps the write first-wins anyway.
    /// `None` session or an unreadable row never tasks.
    pub(super) fn schedule(
        self,
        provider: Arc<dyn Provider>,
        session_id: Option<i64>,
        question: String,
    ) {
        let Some(sid) = session_id else { return };
        match self.db.session_title(sid) {
            Ok(None) => {}
            Ok(Some(_)) => return, // already named — never retitle
            Err(e) => {
                log::warn!("ask: session title read failed: {e}");
                return;
            }
        }
        let Self { db, titled } = self;
        tauri::async_runtime::spawn(async move {
            title_session(db.as_ref(), provider.as_ref(), sid, &question, titled.as_ref()).await;
        });
    }
}

/// One title attempt: a small `stream_chat` over the raw question on
/// the provider that answered — [title prompt, question] — under a
/// total deadline (`stream_chat` bounds only the connect; a hung call
/// would leak the task). No cancel arm — the task deliberately
/// survives `stop`. Failure is silent: the row keeps its
/// first-question fallback and the next send retries. Only the first
/// `TITLE_QUESTION_CAP` scalars of `question` are sent. On a landed
/// write, `titled` pings `sessions:changed` — an open history list
/// re-reads to swap the fallback for the title.
async fn title_session(
    db: &Db,
    provider: &dyn Provider,
    sid: i64,
    question: &str,
    titled: &(dyn Fn(i64) + Send + Sync),
) {
    let question: String = question.chars().take(TITLE_QUESTION_CAP).collect();
    let msgs = [
        ChatMessage::text(Role::System, TITLE_PROMPT),
        ChatMessage::text(Role::User, question),
    ];
    let mut sink = |_: &str| {};
    let reply = match tokio::time::timeout(TITLE_TIMEOUT, provider.stream_chat(&msgs, &mut sink))
        .await
    {
        Ok(r) => r,
        Err(_) => {
            log::warn!("ask: title generation timed out");
            return;
        }
    };
    let title = match reply {
        Ok(r) => clean_title(&r.full),
        Err(e) => {
            log::warn!("ask: title generation failed: {e}");
            return;
        }
    };
    if title.is_empty() {
        return;
    }
    match db.session_set_title(sid, &title) {
        Ok(true) => titled(sid),
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
