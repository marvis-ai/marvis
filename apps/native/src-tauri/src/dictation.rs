//! Ask-input dictation: transient draft assembly for the `me` channel.

use serde::Serialize;

use crate::stt::{Finality, SpeakerChannel, TranscriptEvent};

#[allow(dead_code)] // wired into AppState/commands in Task 2
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DictationStatus {
    pub state: String, // "idle" | "listening" | "error"
    pub provider: Option<String>,
    pub error: Option<DictationError>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DictationError {
    pub message: String,
    pub needs_setup: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DictationDraft {
    pub text: String,
    #[serde(rename = "final")]
    pub finality: bool,
}

/// Single-speaker draft state: committed finals plus the latest interim.
/// Unlike `TurnAssembler` there is no turn closing, silence cadence, or
/// channel switching — `finish()` yields the whole draft once and resets.
/// Live snapshots are `final: false`; only `finish()` marks `final: true`.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct DraftAssembler {
    committed: String,
    provisional: Option<String>,
}

#[allow(dead_code)]
impl DraftAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one STT event into the draft and return the live snapshot;
    /// `None` for events that cannot change it (other channels, blank text).
    pub fn push(&mut self, event: TranscriptEvent) -> Option<DictationDraft> {
        if event.channel != SpeakerChannel::Me {
            return None;
        }
        let text = event.text.trim();
        if text.is_empty() {
            return None;
        }
        match event.finality {
            Finality::Interim => self.provisional = Some(text.to_string()),
            Finality::Final => {
                self.provisional = None;
                append_segment(&mut self.committed, text);
            }
        }
        Some(self.snapshot(false))
    }

    /// The complete draft — committed + provisional — marked `final`,
    /// then resets so a repeated `finish()` returns an empty draft.
    pub fn finish(&mut self) -> DictationDraft {
        let draft = self.snapshot(true);
        self.committed.clear();
        self.provisional = None;
        draft
    }

    fn snapshot(&self, finality: bool) -> DictationDraft {
        let mut text = self.committed.clone();
        if let Some(provisional) = &self.provisional {
            append_segment(&mut text, provisional);
        }
        DictationDraft { text, finality }
    }
}

#[allow(dead_code)]
fn append_segment(committed: &mut String, segment: &str) {
    if !committed.is_empty() {
        committed.push(' ');
    }
    committed.push_str(segment);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stt::{Finality, SpeakerChannel, TranscriptEvent};

    fn event(channel: SpeakerChannel, text: &str, finality: Finality) -> TranscriptEvent {
        TranscriptEvent {
            channel,
            text: text.into(),
            finality,
        }
    }

    #[test]
    fn interim_replaces_previous_provisional() {
        let mut a = DraftAssembler::new();
        let first = a
            .push(event(SpeakerChannel::Me, "hel", Finality::Interim))
            .unwrap();
        assert_eq!(first.text, "hel");
        assert!(!first.finality);
        let next = a
            .push(event(SpeakerChannel::Me, "hello", Finality::Interim))
            .unwrap();
        assert_eq!(next.text, "hello");
    }

    #[test]
    fn final_appends_to_committed_and_clears_provisional() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "hel", Finality::Interim));
        let draft = a
            .push(event(SpeakerChannel::Me, "hello", Finality::Final))
            .unwrap();
        assert_eq!(draft.text, "hello");
        assert!(!draft.finality);
        let draft = a
            .push(event(SpeakerChannel::Me, "wor", Finality::Interim))
            .unwrap();
        assert_eq!(draft.text, "hello wor");
    }

    #[test]
    fn whisper_style_final_chunks_accumulate_in_order() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "chunk one", Finality::Final));
        let draft = a
            .push(event(SpeakerChannel::Me, "chunk two", Finality::Final))
            .unwrap();
        assert_eq!(draft.text, "chunk one chunk two");
    }

    #[test]
    fn repeated_identical_words_are_kept() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "yes", Finality::Final));
        let draft = a
            .push(event(SpeakerChannel::Me, "yes", Finality::Final))
            .unwrap();
        assert_eq!(draft.text, "yes yes");
    }

    #[test]
    fn empty_and_whitespace_text_is_ignored() {
        let mut a = DraftAssembler::new();
        assert!(a
            .push(event(SpeakerChannel::Me, "  ", Finality::Interim))
            .is_none());
        assert!(a
            .push(event(SpeakerChannel::Me, "", Finality::Final))
            .is_none());
        assert_eq!(a.finish().text, "");
    }

    #[test]
    fn draft_text_is_trimmed() {
        let mut a = DraftAssembler::new();
        let draft = a
            .push(event(SpeakerChannel::Me, "  hello  ", Finality::Interim))
            .unwrap();
        assert_eq!(draft.text, "hello");
    }

    #[test]
    fn other_channels_do_not_change_the_draft() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "mine", Finality::Final));
        assert!(a
            .push(event(SpeakerChannel::Them, "theirs", Finality::Final))
            .is_none());
        assert!(a
            .push(event(SpeakerChannel::Them, "noise", Finality::Interim))
            .is_none());
        assert_eq!(a.finish().text, "mine");
    }

    #[test]
    fn finish_returns_complete_draft_once_and_resets() {
        let mut a = DraftAssembler::new();
        a.push(event(SpeakerChannel::Me, "one", Finality::Final));
        a.push(event(SpeakerChannel::Me, "two", Finality::Interim));
        let draft = a.finish();
        assert_eq!(draft.text, "one two");
        assert!(draft.finality);
        let again = a.finish();
        assert_eq!(again.text, "");
        assert!(again.finality);
        let next = a
            .push(event(SpeakerChannel::Me, "three", Finality::Interim))
            .unwrap();
        assert_eq!(next.text, "three");
    }

    #[test]
    fn draft_serializes_final_field_name() {
        let payload = serde_json::to_value(DictationDraft {
            text: "hi".into(),
            finality: true,
        })
        .unwrap();
        assert_eq!(payload, serde_json::json!({ "text": "hi", "final": true }));
    }

    #[test]
    fn status_serializes_documented_wire_fields() {
        let status = DictationStatus {
            state: "error".into(),
            provider: Some("whisper".into()),
            error: Some(DictationError {
                message: "setup".into(),
                needs_setup: true,
            }),
        };
        let payload = serde_json::to_value(status).unwrap();
        assert_eq!(
            payload,
            serde_json::json!({
                "state": "error",
                "provider": "whisper",
                "error": { "message": "setup", "needs_setup": true },
            })
        );
    }
}
