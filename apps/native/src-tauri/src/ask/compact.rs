use std::sync::Arc;

use crate::llm::{ChatMessage, Provider, Role};
use crate::storage::{Db, Message};

use super::{
    COMPACT_BATCH, COMPACT_TIMEOUT, HISTORY_TAIL, MAX_COMPACT_CHARS, MAX_COMPACT_INPUT_BYTES,
    MAX_COMPACT_ROW_CHARS,
};

const COMPACTION_SYSTEM_PROMPT: &str = include_str!("../../prompts/marvis-compaction.md");

/// The owned prefix of a session that should be folded into its continuity
/// digest. The source rows are copied so a detached task never borrows the
/// send's history or stack frame.
pub(crate) struct CompactionPlan {
    pub(crate) session_id: i64,
    pub(crate) expected_through: Option<i64>,
    pub(crate) previous: Option<String>,
    pub(crate) source_rows: Vec<Message>,
    pub(crate) new_through: i64,
}

/// Select the dropped rows that have not already been represented by the
/// stored digest. The live provider tail remains outside the plan.
pub(crate) fn compaction_plan(
    session_id: i64,
    history_rows: &[Message],
    previous: Option<String>,
    expected_through: Option<i64>,
) -> Option<CompactionPlan> {
    let dropped_len = history_rows.len().saturating_sub(HISTORY_TAIL);
    let dropped = &history_rows[..dropped_len];
    let new_through = dropped.last()?.id;
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
        expected_through,
        previous,
        source_rows,
        new_through,
    })
}

/// Render the selected rows into the bounded `<new_messages>` payload. Rows
/// stay oldest-first; a row's content is capped by Unicode scalar values and
/// attachment paths never enter the prompt.
pub(super) fn render_compaction_rows(rows: &[Message]) -> String {
    let mut rendered = String::new();
    for (index, row) in rows.iter().enumerate() {
        let mut line = format!(
            "{}: {}",
            row.role,
            row.content
                .chars()
                .take(MAX_COMPACT_ROW_CHARS)
                .collect::<String>()
        );
        for attachment in &row.attachments {
            line.push_str(" [attached image: ");
            line.push_str(&attachment.name);
            line.push(']');
        }

        let separator_len = usize::from(!rendered.is_empty());
        let remaining = MAX_COMPACT_INPUT_BYTES.saturating_sub(rendered.len() + separator_len);
        if line.len() <= remaining {
            if separator_len != 0 {
                rendered.push('\n');
            }
            rendered.push_str(&line);
            continue;
        }

        // A very large first row still gives the provider some context. Later
        // rows are all-or-nothing so the block never exceeds its byte bound.
        if index == 0 && rendered.is_empty() {
            rendered.push_str(&truncate_to_bytes(&line, MAX_COMPACT_INPUT_BYTES));
        }
        break;
    }
    rendered
}

/// Construct the isolated compaction exchange. Unlike the live Ask request,
/// the newly dropped rows are the user payload and the continuity digest is
/// explicitly marked as context rather than instructions.
pub(super) fn compaction_messages(
    previous: Option<&str>,
    source_rows: &[Message],
) -> Vec<ChatMessage> {
    let mut prompt = String::new();
    if let Some(previous) = previous.filter(|digest| !digest.trim().is_empty()) {
        prompt.push_str("<previous_summary>\n");
        prompt.push_str(previous);
        prompt.push_str("\n</previous_summary>\n\n");
    }
    prompt.push_str("<new_messages>\n");
    prompt.push_str(&render_compaction_rows(source_rows));
    prompt.push_str("\n</new_messages>");

    vec![
        ChatMessage::text(Role::System, COMPACTION_SYSTEM_PROMPT),
        ChatMessage::text(Role::User, prompt),
    ]
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
        plan: CompactionPlan,
    ) -> anyhow::Result<()> {
        let _permit = self.gate.lock().await;
        let (stored_digest, stored_through) = db.session_compaction(plan.session_id)?;
        if stored_through != plan.expected_through
            || stored_digest.as_deref() != plan.previous.as_deref()
        {
            return Ok(());
        }

        let messages = compaction_messages(stored_digest.as_deref(), &plan.source_rows);
        let mut sink = |_token: &str| {};
        let reply =
            tokio::time::timeout(COMPACT_TIMEOUT, provider.stream_chat(&messages, &mut sink))
                .await
                .map_err(|_| anyhow::anyhow!("compaction provider timed out"))??;
        let digest = normalize_compaction_reply(&reply.full)?;
        if !db.session_compact_write(
            plan.session_id,
            plan.expected_through,
            &digest,
            plan.new_through,
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
}

impl CompactHook {
    pub(crate) fn new(db: Arc<Db>, service: Arc<CompactService>) -> Self {
        Self { db, service }
    }

    /// Fire-and-forget compaction. Any provider, timeout, validation, or
    /// storage error is warning-only and never changes the completed Ask.
    pub(crate) fn maybe_schedule(self, provider: Arc<dyn Provider>, plan: CompactionPlan) {
        let Self { db, service } = self;
        tauri::async_runtime::spawn(async move {
            if let Err(error) = service
                .compact_once(db.as_ref(), provider.as_ref(), plan)
                .await
            {
                log::warn!("ask: session compaction skipped: {error}");
            }
        });
    }
}
