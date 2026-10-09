use super::*;

/// Speaker text stays referenceable long enough to outlast the mic's
/// acoustic path plus the STT window and decode lag behind it.
pub(super) const ECHO_REFERENCE_WINDOW: Duration = Duration::from_secs(15);
/// A mic segment is echo when this fraction of its character-bigram
/// multiset is covered by recent speaker text — the acoustic decode
/// garbles a few characters but keeps most of the utterance.
pub(super) const ECHO_CONTAINMENT: f64 = 0.65;
/// Segments this short carry too few bigrams to judge; suppressing them
/// would drop genuine acknowledgments ("好的", "OK"), so they never match.
pub(super) const ECHO_MIN_CHARS: usize = 6;

/// Mic re-capture gate. The capture path has no acoustic echo
/// cancellation, so speech played through the speakers is transcribed
/// twice: digitally on `them`, then acoustically on `me` seconds later —
/// one utterance becomes both a "Speaker N" and a "You" line. The gate
/// retains a short window of normalized `them` text and drops `me`
/// events whose text is contained in it. It only engages while speaker
/// audio was actually captured, so mic-only sessions are unaffected.
#[derive(Debug, Default)]
pub(super) struct EchoGate {
    /// `(seen_at, normalized_text)` of recent `them` segments.
    them_recent: VecDeque<(Instant, String)>,
}

impl EchoGate {
    /// Retain a `them` segment's text as echo reference.
    pub(super) fn record(&mut self, text: &str) {
        self.record_at(text, Instant::now());
    }

    pub(super) fn record_at(&mut self, text: &str, now: Instant) {
        let normalized = normalize_echo_text(text);
        if normalized.is_empty() {
            return;
        }
        self.them_recent.push_back((now, normalized));
        self.evict(now);
    }

    /// `text` heard on the mic re-transcribes speaker output when its
    /// bigrams are mostly covered by the retained `them` window. The
    /// digital channel decodes first — the acoustic path adds the lag —
    /// so the reference already exists when the echo arrives.
    pub(super) fn is_echo(&mut self, text: &str) -> bool {
        self.is_echo_at(text, Instant::now())
    }

    pub(super) fn is_echo_at(&mut self, text: &str, now: Instant) -> bool {
        self.evict(now);
        let probe = normalize_echo_text(text);
        if probe.chars().count() < ECHO_MIN_CHARS || self.them_recent.is_empty() {
            return false;
        }
        // One concatenated window covers an echo that re-decoded to a
        // slice of a speaker segment or spanned a segment boundary.
        let reference: String = self
            .them_recent
            .iter()
            .map(|(_, segment)| segment.as_str())
            .collect();
        echo_containment(&probe, &reference) >= ECHO_CONTAINMENT
    }

    fn evict(&mut self, now: Instant) {
        while self
            .them_recent
            .front()
            .is_some_and(|(seen, _)| now.duration_since(*seen) > ECHO_REFERENCE_WINDOW)
        {
            self.them_recent.pop_front();
        }
    }
}

/// Case-folded alphanumerics only — the digital and acoustic decodes of
/// one utterance disagree on punctuation and spacing more than on the
/// spoken characters themselves.
pub(super) fn normalize_echo_text(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// The fraction of `probe`'s character-bigram multiset covered by
/// `reference`'s. Containment — not symmetric similarity — because an
/// echo may re-decode to just a slice of the speaker text.
pub(super) fn echo_containment(probe: &str, reference: &str) -> f64 {
    let mut available: HashMap<(char, char), usize> = HashMap::new();
    let mut previous = None;
    for c in reference.chars() {
        if let Some(p) = previous.replace(c) {
            *available.entry((p, c)).or_default() += 1;
        }
    }
    let (mut matched, mut total) = (0usize, 0usize);
    let mut previous = None;
    for c in probe.chars() {
        if let Some(p) = previous.replace(c) {
            total += 1;
            let slot = available.entry((p, c)).or_insert(0);
            if *slot > 0 {
                *slot -= 1;
                matched += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        matched as f64 / total as f64
    }
}

