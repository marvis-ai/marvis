//! Speech-to-text provider interfaces.

use crate::audio::PcmChunk;

mod deepgram;
mod whisper;

pub use deepgram::DeepgramProvider;
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptEvent {
    pub channel: SpeakerChannel,
    pub text: String,
    pub finality: Finality,
}

/// Platform-independent speech-to-text provider.
/// Construct the configured provider. Callers retrieve the Deepgram key from
/// `Keystore` and pass it here; this module never accesses the keystore.
pub fn make_stt_provider(
    provider: &str,
    key: Option<String>,
    model: String,
    channel: SpeakerChannel,
    bundled_whisper: Option<&std::path::Path>,
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
        ))),
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

/// Provider-error text safe for `*:error` webview events: flattened to one
/// line and length-capped so process output or transport details cannot
/// reach the frontend raw.
pub(crate) fn sanitize_provider_error(message: &str) -> String {
    let mut sanitized = message.replace(['\n', '\r'], " ");
    if sanitized.len() > 240 {
        sanitized.truncate(240);
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_events_have_explicit_finality() {
        let event = TranscriptEvent {
            channel: SpeakerChannel::Me,
            text: "hello".to_string(),
            finality: Finality::Final,
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
    fn sanitize_provider_error_flattens_lines_and_caps_length() {
        assert_eq!(sanitize_provider_error("one\ntwo\rthree"), "one two three");
        assert!(!sanitize_provider_error("a\r\nb").contains(['\n', '\r']));
        assert_eq!(sanitize_provider_error(&"x".repeat(500)).len(), 240);
    }
}
