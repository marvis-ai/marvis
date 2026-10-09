use super::*;

/// Single-speaker draft state: committed finals plus the latest interim.
/// Unlike `TurnAssembler` there is no turn closing, silence cadence, or
/// channel switching — `finish()` yields the whole draft once and resets.
/// Live snapshots are `final: false`; only `finish()` marks `final: true`.
#[derive(Debug, Clone, Default)]
pub struct DraftAssembler {
    committed: String,
    provisional: Option<String>,
}

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

pub(super) fn append_segment(committed: &mut String, segment: &str) {
    if !committed.is_empty() {
        committed.push(' ');
    }
    committed.push_str(segment);
}

