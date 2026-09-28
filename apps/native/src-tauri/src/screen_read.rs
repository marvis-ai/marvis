//! Screen reading: intent detection, the vision-model describe call,
//! and the cached `screen_context` the ask chain consumes.

use tokio_util::sync::CancellationToken;

use crate::capture::Frame;
use crate::llm::{ChatMessage, LlmError, Provider, StreamReply};
use crate::prompts::screen_prompt;

/// A cached screen description + unix-seconds stamp — asks annotate age
/// so stale context is never silently presented as current.
#[derive(Debug, Clone)]
#[allow(dead_code)] // populated by the ScreenReader loop (Task 5)
pub(crate) struct ScreenContext {
    pub text: String,
    pub ts: i64,
}

/// Lowercase substring table — keep it tight: short generic terms fire
/// on unrelated uses of "screen". ASCII terms match on lowercase;
/// CJK terms are unambiguous enough for `contains`.
const INTENT_KEYWORDS: &[&str] = &[
    "my screen",
    "this screen",
    "on screen",
    "the screen",
    "screenshot",
    "屏幕",
    "截图",
];

/// Deterministic screen-intent heuristic — `Cmd+Enter` bypasses this
/// entirely (explicit flag on `ask_send`).
#[allow(dead_code)] // consumed by the intent gate (Task 5); tests cover it now
pub(crate) fn looks_like_screen_intent(text: &str) -> bool {
    let t = text.to_lowercase();
    INTENT_KEYWORDS.iter().any(|k| t.contains(k))
}

/// One frame → a text description. Silent intermediate read — tokens
/// never reach the card. `Ok(None)` = cancelled; `Err` = provider
/// failure (callers decide cache/attach fallback).
pub(crate) async fn describe_screen(
    provider: &dyn Provider,
    frame: &Frame,
    cancel: &CancellationToken,
) -> Result<Option<StreamReply>, LlmError> {
    let msgs = vec![ChatMessage::user_with_image(
        screen_prompt(),
        frame.jpeg.clone(),
    )];
    let mut sink = |_: &str| {};
    tokio::select! {
        _ = cancel.cancelled() => Ok(None),
        r = provider.stream_chat(&msgs, &mut sink) => match r {
            Ok(reply) => Ok(Some(reply)),
            Err(e) => Err(e),
        },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn intent_matches_screen_keywords() {
        for t in [
            "what's on my screen?",
            "read my screen",
            "can you see this screenshot",
            "看看我的屏幕",
            "截图看看",
            "屏幕上是什么",
        ] {
            assert!(super::looks_like_screen_intent(t), "missed: {t}");
        }
    }

    #[test]
    fn intent_ignores_unrelated_text() {
        for t in [
            "summarize the meeting",
            "what time is it",
            "fix the null check on line 4",
            "",
        ] {
            assert!(!super::looks_like_screen_intent(t), "false hit: {t}");
        }
    }

    #[test]
    fn intent_documents_accepted_false_positive() {
        // "the screen" is a keyword → this unrelated use also fires.
        // Acceptable: the cost is one extra screenshot read, not a
        // wrong answer. Locked in as a test so a matcher rewrite
        // revisits the trade-off deliberately.
        assert!(super::looks_like_screen_intent("clean the screen door"));
    }
}
