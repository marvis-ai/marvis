//! Speech-to-text provider interfaces.

use crate::audio::PcmChunk;

mod deepgram;
mod sherpa;
pub mod speaker;
mod whisper;

pub use deepgram::DeepgramProvider;
pub use sherpa::SherpaProvider;
pub use speaker::{tracker_if_installed, SpeakerTracker};
pub use whisper::{WhisperBinarySource, WhisperBinaryStatus, WhisperProvider, WhisperStatus};

/// The source channel represented by a transcript event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeakerChannel {
    Me,
    Them,
}

/// Whether a transcript may be displayed as a provisional result or is complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finality {
    Interim,
    Final,
}

/// A normalized transcript emitted by an STT provider.
/// `speaker_idx` is the diarized voice cluster within `channel` —
/// `None` when speaker diarization is off or the segment was unlabelable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptEvent {
    /// Start in the provider's PCM stream, before decoding/transport latency.
    pub audio_start_ms: Option<u64>,
    pub channel: SpeakerChannel,
    pub text: String,
    pub finality: Finality,
    pub speaker_idx: Option<u32>,
}

/// Platform-independent speech-to-text provider.
/// Construct the configured provider. Callers retrieve the Deepgram key from
/// `Keystore` and pass it here; this module never accesses the keystore.
/// `diarize` enables per-segment speaker clustering for the local
/// providers — it degrades silently when the embedding model is absent.
pub fn make_stt_provider(
    provider: &str,
    key: Option<String>,
    model: String,
    channel: SpeakerChannel,
    bundled_whisper: Option<&std::path::Path>,
    diarize: bool,
) -> anyhow::Result<Box<dyn SttProvider>> {
    match provider {
        "deepgram" => Ok(Box::new(DeepgramProvider::new(
            key.ok_or_else(|| anyhow::anyhow!("Deepgram API key is not configured"))?,
            model,
            channel,
        ))),
        "whisper" => Ok(Box::new(WhisperProvider::new(
            model,
            channel,
            bundled_whisper,
            diarize,
        ))),
        "sherpa" => Ok(Box::new(SherpaProvider::new(&model, channel, diarize))),
        _ => anyhow::bail!("unsupported STT provider: {provider}"),
    }
}

pub trait SttProvider: Send {
    /// Start processing chunks, reporting transcripts and terminal provider errors.
    fn start(
        &mut self,
        callback: Box<dyn Fn(TranscriptEvent) + Send + Sync>,
        error_callback: Box<dyn Fn(String) + Send + Sync>,
    ) -> anyhow::Result<()>;

    /// Enqueue a chunk without waiting for transcription.
    fn enqueue(&self, chunk: PcmChunk) -> bool;

    /// Stop processing and release the provider worker.
    fn stop(&mut self);
}

/// Bundled-aware Whisper setup validation shared by Listen and Dictation:
/// a missing executable or configured model is a user-fixable setup error.
/// Messages are curated — they never include paths or install details.
pub(crate) fn whisper_setup_error(status: &WhisperStatus, model: &str) -> Option<&'static str> {
    if status.binary.is_none() {
        return Some("whisper-cli was not found; install it and try again");
    }
    let model_available = WhisperProvider::model_filename(model)
        .ok()
        .is_some_and(|filename| status.models.iter().any(|name| name == filename));
    (!model_available)
        .then_some("configured Whisper model was not found; choose an installed model in Settings")
}

/// The full provider setup check shared by Listen and Dictation — both at
/// start pre-flight and when a settings change re-validates a durable
/// setup error. Deepgram needs its keystore entry, whisper its binary +
/// model, sherpa its downloaded file set under `sherpa_root`. Unknown
/// providers pass here and fail later in `make_stt_provider`.
pub(crate) fn stt_setup_error(
    provider: &str,
    deepgram_key: Option<&str>,
    model: &str,
    bundled_whisper: Option<&std::path::Path>,
    sherpa_root: &std::path::Path,
) -> Option<&'static str> {
    if provider == "deepgram" && deepgram_key.is_none() {
        return Some("Speech-to-text provider is not configured");
    }
    if provider == "whisper" {
        return whisper_setup_error(
            &WhisperProvider::status_with_bundled(bundled_whisper),
            model,
        );
    }
    if provider == "sherpa" {
        return sherpa_setup_error_at(sherpa_root, model);
    }
    None
}

/// Sherpa setup validation shared by Listen and Dictation — a missing or
/// partially downloaded model is a user-fixable setup error. Messages are
/// curated; they never include paths or engine details.
fn sherpa_setup_error_at(root: &std::path::Path, model: &str) -> Option<&'static str> {
    let Some(entry) = crate::sherpa_models::stt_entry_for_value(model) else {
        return Some(
            "configured Sherpa model was not found; choose an installed model in Settings",
        );
    };
    (!crate::sherpa_models::entry_installed_at(root, entry))
        .then_some("the SenseVoice model is not downloaded; download it in Settings")
}

/// Provider-error text safe for `*:error` webview events: flattened to one
/// line and length-capped so process output or transport details cannot
/// reach the frontend raw.
pub(crate) fn sanitize_provider_error(message: &str) -> String {
    let mut sanitized = message.replace(['\n', '\r'], " ");
    if sanitized.len() > 240 {
        // `truncate` panics on a non-char boundary — walk back to one so
        // multi-byte text can never crash the error path.
        let mut boundary = 240;
        while !sanitized.is_char_boundary(boundary) {
            boundary -= 1;
        }
        sanitized.truncate(boundary);
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_events_have_explicit_finality() {
        let event = TranscriptEvent {
            audio_start_ms: None,
            channel: SpeakerChannel::Me,
            text: "hello".to_string(),
            finality: Finality::Final,
            speaker_idx: None,
        };
        assert_eq!(event.channel, SpeakerChannel::Me);
        assert_eq!(event.finality, Finality::Final);
    }

    #[test]
    fn provider_interface_accepts_normalized_pcm_only() {
        fn assert_provider<T: SttProvider>() {}
        assert_provider::<WhisperProvider>();
    }

    #[test]
    fn sherpa_provider_satisfies_the_stt_contract() {
        fn assert_provider<T: SttProvider>() {}
        assert_provider::<SherpaProvider>();
    }

    /// Setup errors are curated messages: an unknown catalog value reports
    /// "not found", a known-but-absent file set reports "not downloaded",
    /// and a complete set in a temp root clears the check.
    #[test]
    fn sherpa_setup_error_distinguishes_unknown_missing_and_installed() {
        let root = stt_test_dir("setup");
        let unknown = sherpa_setup_error_at(&root, "no-such-model").unwrap();
        assert!(unknown.contains("not found"));

        let missing = sherpa_setup_error_at(&root, "sense-voice").unwrap();
        assert!(missing.contains("not downloaded"));

        let entry = crate::sherpa_models::entry_for_value("sense-voice").unwrap();
        let dir = crate::sherpa_models::entry_dir(&root, entry);
        std::fs::create_dir_all(&dir).unwrap();
        for file in entry.files {
            std::fs::write(dir.join(file.filename), b"").unwrap();
        }
        assert_eq!(sherpa_setup_error_at(&root, "sense-voice"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The umbrella check mirrors the per-provider start gates: deepgram
    /// needs a key, sherpa a downloaded file set, and an unknown provider
    /// passes through to `make_stt_provider`.
    #[test]
    fn stt_setup_error_covers_each_provider_gate() {
        let root = stt_test_dir("umbrella");
        assert_eq!(
            stt_setup_error("deepgram", None, "nova-2", None, &root),
            Some("Speech-to-text provider is not configured")
        );
        assert_eq!(
            stt_setup_error("deepgram", Some("dg-key"), "nova-2", None, &root),
            None
        );
        let missing = stt_setup_error("sherpa", None, "sense-voice", None, &root).unwrap();
        assert!(missing.contains("not downloaded"));
        let entry = crate::sherpa_models::entry_for_value("sense-voice").unwrap();
        let dir = crate::sherpa_models::entry_dir(&root, entry);
        std::fs::create_dir_all(&dir).unwrap();
        for file in entry.files {
            std::fs::write(dir.join(file.filename), b"").unwrap();
        }
        assert_eq!(
            stt_setup_error("sherpa", None, "sense-voice", None, &root),
            None
        );
        assert_eq!(
            stt_setup_error("not-a-provider", None, "x", None, &root),
            None
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    fn stt_test_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let path = std::env::temp_dir().join(format!("marvis-stt-{tag}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn sanitize_provider_error_flattens_lines_and_caps_length() {
        assert_eq!(sanitize_provider_error("one\ntwo\rthree"), "one two three");
        assert!(!sanitize_provider_error("a\r\nb").contains(['\n', '\r']));
        assert_eq!(sanitize_provider_error(&"x".repeat(500)).len(), 240);
    }

    /// Byte 240 lands inside the 2-byte `é` — a naive `truncate(240)`
    /// would panic; the cap must walk back to a char boundary instead.
    #[test]
    fn sanitize_provider_error_truncates_on_char_boundary() {
        let input = format!("{}é", "x".repeat(239));
        let sanitized = sanitize_provider_error(&input);
        assert_eq!(sanitized, "x".repeat(239));
        assert_eq!(sanitized.len(), 239);

        // An exactly-240-byte multi-byte tail still truncates cleanly.
        let input = format!("{}é", "x".repeat(238));
        assert_eq!(sanitize_provider_error(&input).len(), 240);
    }
}
