use std::sync::Arc;

use crate::llm::{ChatMessage, Provider, Role};
use crate::session_lifecycle::SessionLifecycle;
use crate::storage::{Db, Message};

use super::{
    COMPACT_BATCH, COMPACT_TIMEOUT, HISTORY_TAIL, MAX_COMPACT_CHARS, MAX_COMPACT_INPUT_BYTES,
    MAX_COMPACT_ROW_CHARS,
};

const COMPACTION_SYSTEM_PROMPT: &str = include_str!("../../prompts/marvis-compaction.md");

/// The owned uncovered prefix of one session incarnation that should be folded
/// into its continuity digest. `session_token` prevents the detached job from
/// being applied to a later session that reuses the same integer id. The
/// eventual watermark is chosen by the renderer from the last source row
/// actually represented in `<new_messages>`. The source rows are copied so a
/// detached task never borrows the send's history or stack frame.
pub(crate) struct CompactionPlan {
    pub(crate) session_id: i64,
    pub(crate) session_token: String,
    pub(crate) expected_through: Option<i64>,
    pub(crate) previous: Option<String>,
    pub(crate) source_rows: Vec<Message>,
}

/// Select the dropped rows that have not already been represented by the
/// stored digest. The live provider tail remains outside the plan.
pub(crate) fn compaction_plan(
    session_id: i64,
    session_token: String,
    history_rows: &[Message],
    previous: Option<String>,
    expected_through: Option<i64>,
) -> Option<CompactionPlan> {
    let dropped_len = history_rows.len().saturating_sub(HISTORY_TAIL);
    let dropped = &history_rows[..dropped_len];
    let source_rows: Vec<Message> = dropped
        .iter()
        .filter(|row| expected_through.map_or(true, |through| row.id > through))
        .cloned()
        .collect();
    if source_rows.len() < COMPACT_BATCH {
        return None;
    }
    Some(CompactionPlan {
        session_id,
        session_token,
        expected_through,
        previous,
        source_rows,
    })
}

const MAX_COMPACT_ATTACHMENT_NAME_CHARS: usize = 256;

/// Render one source row without allowing attachment metadata to add
/// unbounded or structural prompt content. The complete line, including its
/// role, content, and any complete attachment markers, stays within the row
/// scalar bound.
fn render_compaction_row(row: &Message) -> String {
    let prefix = format!("{}: ", row.role);
    let prefix_len = prefix.chars().count().min(MAX_COMPACT_ROW_CHARS);
    let mut line: String = prefix.chars().take(MAX_COMPACT_ROW_CHARS).collect();
    let content_budget = MAX_COMPACT_ROW_CHARS.saturating_sub(prefix_len);
    line.extend(row.content.chars().take(content_budget));

    for attachment in &row.attachments {
        let name: String = attachment
            .name
            .chars()
            .filter(|character| {
                !character.is_control() && !matches!(character, '[' | ']' | '<' | '>')
            })
            .take(MAX_COMPACT_ATTACHMENT_NAME_CHARS)
            .collect();
        let marker = format!(" [attached image: {name}]");
        if line.chars().count() + marker.chars().count() > MAX_COMPACT_ROW_CHARS {
            break;
        }
        line.push_str(&marker);
    }
    line
}

/// Render the selected rows into the bounded `<new_messages>` payload. Rows
/// stay oldest-first; the returned watermark is the id of the last source row
/// actually represented in the bounded block. Later rows remain uncovered.
pub(super) fn render_compaction_rows(rows: &[Message]) -> (String, Option<i64>) {
    let mut rendered = String::new();
    let mut last_rendered = None;
    for row in rows {
        let line = render_compaction_row(row);
        let separator_len = usize::from(!rendered.is_empty());
        let remaining = MAX_COMPACT_INPUT_BYTES.saturating_sub(rendered.len() + separator_len);
        if line.len() <= remaining {
            if separator_len != 0 {
                rendered.push('\n');
            }
            rendered.push_str(&line);
            last_rendered = Some(row.id);
            continue;
        }

        // A very large first row still gives the provider some context. Later
        // rows are all-or-nothing so the block never exceeds its byte bound.
        if rendered.is_empty() {
            let first = truncate_to_bytes(&line, MAX_COMPACT_INPUT_BYTES);
            if !first.is_empty() {
                rendered.push_str(&first);
                last_rendered = Some(row.id);
            }
        }
        break;
    }
    (rendered, last_rendered)
}

fn bounded_compaction_digest(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.chars().take(MAX_COMPACT_CHARS).collect())
    }
}

/// Construct the isolated compaction exchange and return the source watermark
/// represented in its bounded `<new_messages>` block. Unlike the live Ask
/// request, the newly dropped rows are the user payload and the continuity
/// digest is explicitly marked as context rather than instructions.
fn compaction_messages_with_watermark(
    previous: Option<&str>,
    source_rows: &[Message],
) -> (Vec<ChatMessage>, Option<i64>) {
    let (rendered_rows, last_rendered) = render_compaction_rows(source_rows);
    let mut prompt = String::new();
    if let Some(previous) = previous.and_then(bounded_compaction_digest) {
        prompt.push_str("<previous_summary>\n");
        prompt.push_str(&previous);
        prompt.push_str("\n</previous_summary>\n\n");
    }
    prompt.push_str("<new_messages>\n");
    prompt.push_str(&rendered_rows);
    prompt.push_str("\n</new_messages>");

    (
        vec![
            ChatMessage::text(Role::System, COMPACTION_SYSTEM_PROMPT),
            ChatMessage::text(Role::User, prompt),
        ],
        last_rendered,
    )
}

#[cfg(test)]
pub(super) fn compaction_messages(
    previous: Option<&str>,
    source_rows: &[Message],
) -> Vec<ChatMessage> {
    compaction_messages_with_watermark(previous, source_rows).0
}

/// Normalize provider output before it can reach local persistence.
pub(super) fn normalize_compaction_reply(raw: &str) -> anyhow::Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!(
            "compaction provider returned an empty reply"
        ));
    }
    Ok(trimmed.chars().take(MAX_COMPACT_CHARS).collect())
}

fn truncate_to_bytes(value: &str, max_bytes: usize) -> String {
    let end = value
        .char_indices()
        .take_while(|(index, character)| index + character.len_utf8() <= max_bytes)
        .map(|(index, character)| index + character.len_utf8())
        .last()
        .unwrap_or(0);
    value[..end].to_string()
}

/// Serializes compaction provider calls. The permit intentionally remains
/// held across the provider await so concurrent detached jobs cannot each
/// read the same watermark and race to generate redundant digests.
pub(crate) struct CompactService {
    gate: tokio::sync::Mutex<()>,
}

impl CompactService {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            gate: tokio::sync::Mutex::new(()),
        })
    }

    async fn compact_once(
        &self,
        db: &Db,
        provider: &dyn Provider,
        lifecycle: Option<Arc<SessionLifecycle>>,
        plan: CompactionPlan,
    ) -> anyhow::Result<()> {
        let _permit = self.gate.lock().await;
        let Some((stored_token, stored_digest, stored_through)) =
            db.session_compaction(plan.session_id)?
        else {
            // The owning session may have been deleted while this detached job
            // was waiting. Do not send its source rows to the provider.
            return Ok(());
        };
        if stored_token != plan.session_token
            || stored_through != plan.expected_through
            || stored_digest.as_deref() != plan.previous.as_deref()
        {
            return Ok(());
        }
        // Re-read and lease acquisition are the provider handoff boundary:
        // deletion either marked first and makes this a no-op, or waits for
        // this owned lease through stream_chat and the token-bound CAS write.
        let _session_lease = match lifecycle.as_ref() {
            Some(lifecycle) => match lifecycle.acquire(plan.session_id) {
                Some(lease) => Some(lease),
                None => return Ok(()),
            },
            None => None,
        };

        let (messages, rendered_through) =
            compaction_messages_with_watermark(stored_digest.as_deref(), &plan.source_rows);
        let Some(new_through) = rendered_through else {
            return Ok(());
        };
        let mut sink = |_token: &str| {};
        let reply =
            tokio::time::timeout(COMPACT_TIMEOUT, provider.stream_chat(&messages, &mut sink))
                .await
                .map_err(|_| anyhow::anyhow!("compaction provider timed out"))??;
        let digest = normalize_compaction_reply(&reply.full)?;
        if !db.session_compact_write(
            plan.session_id,
            &plan.session_token,
            plan.expected_through,
            &digest,
            new_through,
        )? {
            // Another completed job won the compare-and-set race. That is a
            // normal stale-job outcome, not an error for the ask run.
            return Ok(());
        }
        Ok(())
    }
}

/// Detached compaction hook owned by the Ask send after its answering
/// provider succeeds. All state crossing the spawn boundary is Arc'd or
/// owned by the plan.
pub(crate) struct CompactHook {
    db: Arc<Db>,
    service: Arc<CompactService>,
    lifecycle: Option<Arc<SessionLifecycle>>,
}

impl CompactHook {
    /// Direct pipeline tests can use the legacy constructor without an
    /// application lifecycle registry; production `AskService::kick` uses
    /// [`Self::with_lifecycle`] below.
    #[cfg(test)]
    pub(crate) fn new(db: Arc<Db>, service: Arc<CompactService>) -> Self {
        Self {
            db,
            service,
            lifecycle: None,
        }
    }

    pub(crate) fn with_lifecycle(
        db: Arc<Db>,
        service: Arc<CompactService>,
        lifecycle: Arc<SessionLifecycle>,
    ) -> Self {
        Self {
            db,
            service,
            lifecycle: Some(lifecycle),
        }
    }

    /// Fire-and-forget compaction. Any provider, timeout, validation, or
    /// storage error is warning-only and never changes the completed Ask.
    pub(crate) fn maybe_schedule(self, provider: Arc<dyn Provider>, plan: CompactionPlan) {
        let Self {
            db,
            service,
            lifecycle,
        } = self;
        tauri::async_runtime::spawn(async move {
            if let Err(error) = service
                .compact_once(db.as_ref(), provider.as_ref(), lifecycle, plan)
                .await
            {
                log::warn!("ask: session compaction skipped: {error}");
            }
        });
    }
}
