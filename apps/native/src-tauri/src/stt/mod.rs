//! Speech-to-text provider interfaces.

use crate::audio::PcmChunk;

mod deepgram;
mod whisper;

pub use deepgram::DeepgramProvider;
pub use whisper::{WhisperProvider, WhisperStatus};

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
) -> anyhow::Result<Box<dyn SttProvider>> {
    match provider {
        "deepgram" => Ok(Box::new(DeepgramProvider::new(
            key.ok_or_else(|| anyhow::anyhow!("Deepgram API key is not configured"))?,
            model,
            channel,
        ))),
        "whisper" => Ok(Box::new(WhisperProvider::new(model, channel))),
        _ => anyhow::bail!("unsupported STT provider: {provider}"),
    }
}

pub trait SttProvider: Send {
    /// Start processing chunks and call `callback` from the provider worker.
    fn start(&mut self, callback: Box<dyn Fn(TranscriptEvent) + Send + Sync>)
        -> anyhow::Result<()>;

    /// Enqueue a chunk without waiting for transcription.
    fn enqueue(&self, chunk: PcmChunk) -> bool;

    /// Stop processing and release the provider worker.
    fn stop(&mut self);
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
}
