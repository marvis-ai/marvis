//! Prompt text for the memory pipeline: the bounded profile block Ask
//! injects, and the extractor's contract + payload shape.
//!
//! `profile_prompt` renders `memories` rows as `- {category}/
//! {attribute}: {value} [{basis}, {confidence}%]` inside
//! `<user_profile>` tags — hard-capped at `MAX_PROFILE_ROWS` rows and
//! `MAX_PROFILE_BYTES` UTF-8 bytes so a large profile can't blow up the
//! system prompt. Deterministic ordering (`category`, `attribute`,
//! `id`) keeps the prompt stable across identical databases — the same
//! facts produce byte-identical context.
//!
//! `extraction_messages` builds the two-message extractor call: a
//! system contract plus ONE user message wrapping the existing
//! profile, the new ask text, and the send's observation date in
//! separate data blocks — the raw ask text is the only new source
//! content (no assistant reply, no screen text, no history).

use super::{MAX_PROFILE_BYTES, MAX_PROFILE_ROWS};
use crate::llm::{ChatMessage, Role};
use crate::storage::Memory;

/// `- {category}/{attribute}: {value} [{basis}, {confidence}%]` lines.
fn profile_lines(rows: &[Memory]) -> Vec<String> {
    let mut sorted: Vec<&Memory> = rows.iter().collect();
    sorted.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then(a.attribute.cmp(&b.attribute))
            .then(a.id.cmp(&b.id))
    });
    sorted
        .into_iter()
        .take(MAX_PROFILE_ROWS)
        .map(|row| {
            format!(
                "- {}/{}: {} [{}, {}%]",
                row.category,
                row.attribute,
                row.value,
                row.basis,
                (row.confidence * 100.0).round() as u32
            )
        })
        .collect()
}

/// The stored profile as a `<user_profile>` block for the Ask system
/// prompt — `None` when there are no facts (the caller then uses the
/// plain prompt, byte-identical to today). Whole lines drop when the
/// byte budget is hit — a line is never split mid-UTF-8. No provenance
/// (source ids) ever renders.
pub(crate) fn profile_prompt(rows: &[Memory]) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    const OPEN: &str = "<user_profile>\n";
    const CLOSE: &str = "</user_profile>";
    let budget = MAX_PROFILE_BYTES.saturating_sub(OPEN.len() + CLOSE.len() + 1);
    let mut out = String::from(OPEN);
    let mut used = 0;
    for line in profile_lines(rows) {
        let line = format!("{line}\n");
        if used + line.len() > budget {
            break;
        }
        out.push_str(&line);
        used += line.len();
    }
    if used == 0 {
        return None;
    }
    out.push_str(CLOSE);
    Some(out)
}

/// The extractor's contract — strict JSON, durable user facts only —
/// lives in `prompts/marvis-extraction.md` beside the other mode
/// prompts (`src/prompts.rs`).
fn extraction_system_prompt() -> &'static str {
    include_str!("../../prompts/marvis-extraction.md").trim()
}

/// `YYYY-MM-DD` in UTC — the extractor's temporal anchor. Day
/// precision is all it needs; a local-tz date would move the answer
/// only around midnight.
pub(crate) fn today_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// The extractor call: system contract + one user message carrying
/// `<existing_profile>` (current rows — context, dedup signal),
/// `<new_user_message>` (the raw ask text — the only extraction
/// source), and `<observation_date>` (the send's date — the anchor for
/// resolving the message's relative time references).
pub(crate) fn extraction_messages(existing: &[Memory], source_text: &str) -> Vec<ChatMessage> {
    let lines = profile_lines(existing);
    let profile = if lines.is_empty() {
        "(none)".to_string()
    } else {
        lines.join("\n")
    };
    vec![
        ChatMessage::text(Role::System, extraction_system_prompt()),
        ChatMessage::text(
            Role::User,
            format!(
                "<existing_profile>\n{profile}\n</existing_profile>\n\n\
                 <new_user_message>\n{source_text}\n</new_user_message>\n\n\
                 <observation_date>\n{}\n</observation_date>",
                today_utc()
            ),
        ),
    ]
}
