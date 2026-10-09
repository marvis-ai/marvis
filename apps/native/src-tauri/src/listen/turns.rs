use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedTurn {
    pub audio_start_ms: Option<u64>,
    pub speaker: SpeakerChannel,
    /// Diarized voice cluster within `speaker`'s channel — `None` when
    /// diarization is off or the segment was unlabelable.
    pub speaker_idx: Option<u32>,
    pub text: String,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListenTurn {
    pub audio_start_ms: Option<u64>,
    #[serde(serialize_with = "serialize_speaker")]
    pub speaker: SpeakerChannel,
    pub speaker_idx: Option<u32>,
    pub text: String,
    pub ts: i64,
    pub session_id: i64,
    #[serde(rename = "final")]
    pub finality: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Pending {
    audio_start_ms: Option<u64>,
    committed: String,
    provisional: Option<String>,
    last_final: Option<Instant>,
    speaker_idx: Option<u32>,
}

/// Deterministic state machine for provisional and final STT results.
#[derive(Debug, Clone, Default)]
pub struct TurnAssembler {
    me: Pending,
    them: Pending,
    active: Option<SpeakerChannel>,
    closed: usize,
}

impl TurnAssembler {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, event: TranscriptEvent) -> Vec<ClosedTurn> {
        self.push_at(event, Instant::now())
    }

    pub fn push_at(&mut self, event: TranscriptEvent, now: Instant) -> Vec<ClosedTurn> {
        let text = event.text.trim().to_string();
        if text.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if self.active != Some(event.channel) {
            if let Some(previous) = self.active {
                out.extend(self.close(previous));
            }
            self.active = Some(event.channel);
        }
        // A different confirmed voice closes the current turn even inside
        // one channel's silence window — a document transcript shows each
        // speaker's run as its own block.
        let speaker_changed = event.finality == Finality::Final
            && !self.pending(event.channel).committed.is_empty()
            && self.pending(event.channel).speaker_idx.is_some()
            && event.speaker_idx.is_some()
            && self.pending(event.channel).speaker_idx != event.speaker_idx;
        if speaker_changed {
            out.extend(self.close(event.channel));
        }
        let pending = self.pending_mut(event.channel);
        pending.audio_start_ms = pending.audio_start_ms.or(event.audio_start_ms);
        match event.finality {
            Finality::Interim => pending.provisional = Some(text),
            Finality::Final => {
                pending.provisional = None;
                append_segment(&mut pending.committed, &text);
                pending.last_final = Some(now);
                // An unlabeled segment keeps the turn's current speaker;
                // a labeled one claims it (the turn already split above
                // when two labels disagreed).
                pending.speaker_idx = event.speaker_idx.or(pending.speaker_idx);
            }
        }
        out.extend(self.flush_due(now));
        out
    }

    pub fn flush(&mut self) -> Vec<ClosedTurn> {
        let mut out = Vec::new();
        for channel in [SpeakerChannel::Me, SpeakerChannel::Them] {
            out.extend(self.close(channel));
        }
        self.active = None;
        out
    }
    pub fn flush_at(&mut self, now: Instant) -> Vec<ClosedTurn> {
        self.flush_due(now)
    }

    pub fn interim(&self, channel: SpeakerChannel) -> Option<String> {
        let pending = self.pending(channel);
        let mut text = pending.committed.clone();
        if let Some(provisional) = &pending.provisional {
            append_segment(&mut text, provisional);
        }
        (!text.trim().is_empty()).then(|| text.trim().to_string())
    }

    /// Preserve the open turn's capture origin while its text evolves.
    pub fn interim_audio_start_ms(&self, channel: SpeakerChannel) -> Option<u64> {
        self.pending(channel).audio_start_ms
    }

    /// The open turn's established speaker label.
    pub fn interim_label(&self, channel: SpeakerChannel) -> Option<u32> {
        self.pending(channel).speaker_idx
    }

    fn flush_due(&mut self, now: Instant) -> Vec<ClosedTurn> {
        let mut out = Vec::new();
        for channel in [SpeakerChannel::Me, SpeakerChannel::Them] {
            let due = self
                .pending(channel)
                .last_final
                .map(|at| now.duration_since(at) >= SILENCE)
                .unwrap_or(false);
            if due {
                out.extend(self.close(channel));
                if self.active == Some(channel) {
                    self.active = None;
                }
            }
        }
        out
    }
    fn close(&mut self, channel: SpeakerChannel) -> Vec<ClosedTurn> {
        let pending = self.pending_mut(channel);
        let mut text = std::mem::take(&mut pending.committed);
        if let Some(provisional) = pending.provisional.take() {
            append_segment(&mut text, &provisional);
        }
        pending.last_final = None;
        let speaker_idx = pending.speaker_idx.take();
        let audio_start_ms = pending.audio_start_ms.take();
        if text.trim().is_empty() {
            return Vec::new();
        }
        self.closed += 1;
        vec![ClosedTurn {
            audio_start_ms,
            speaker: channel,
            speaker_idx,
            text: text.trim().to_string(),
            ts: now_unix(),
        }]
    }
    /// Drop a channel's open provisional — the echo gate removed the
    /// matching final upstream, and a stale provisional would still
    /// land in the next `close()`.
    pub fn drop_provisional(&mut self, channel: SpeakerChannel) {
        let pending = self.pending_mut(channel);
        pending.provisional = None;
        if pending.committed.is_empty() {
            pending.audio_start_ms = None;
        }
    }
    fn pending(&self, channel: SpeakerChannel) -> &Pending {
        match channel {
            SpeakerChannel::Me => &self.me,
            SpeakerChannel::Them => &self.them,
        }
    }
    fn pending_mut(&mut self, channel: SpeakerChannel) -> &mut Pending {
        match channel {
            SpeakerChannel::Me => &mut self.me,
            SpeakerChannel::Them => &mut self.them,
        }
    }
    #[allow(dead_code)]
    pub fn summary_boundary(&self) -> bool {
        self.closed > 0 && self.closed.is_multiple_of(SUMMARY_EVERY)
    }
    #[allow(dead_code)]
    pub fn closed_count(&self) -> usize {
        self.closed
    }
}

pub(super) fn append_segment(committed: &mut String, segment: &str) {
    if committed.is_empty() {
        committed.push_str(segment);
    } else if committed != segment {
        committed.push(' ');
        committed.push_str(segment);
    }
}

