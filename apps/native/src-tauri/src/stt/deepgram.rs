//! Deepgram WebSocket speech-to-text provider.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::audio::PcmChunk;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use super::{Finality, SpeakerChannel, SttProvider, TranscriptEvent};

const ENDPOINT: &str = "wss://api.deepgram.com/v1/listen";
const KEEPALIVE: Duration = Duration::from_secs(10);
const RENEW_AFTER: Duration = Duration::from_secs(20 * 60);
const MAX_RECONNECTS: usize = 3;

/// A Deepgram streaming provider. The key is retained only for the lifetime of
/// this provider and is never written to disk or included in diagnostics.
pub struct DeepgramProvider {
    key: String,
    model: String,
    channel: SpeakerChannel,
    input: Option<mpsc::Sender<PcmChunk>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl DeepgramProvider {
    pub fn new(key: String, model: String, channel: SpeakerChannel) -> Self {
        Self {
            key,
            model,
            channel,
            input: None,
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }
}

impl SttProvider for DeepgramProvider {
    fn start(
        &mut self,
        callback: Box<dyn Fn(TranscriptEvent) + Send + Sync>,
    ) -> anyhow::Result<()> {
        if self.worker.is_some() {
            anyhow::bail!("deepgram provider is already running");
        }
        let (sender, receiver) = mpsc::channel(32);
        self.input = Some(sender);
        self.stop.store(false, Ordering::Release);
        let key = self.key.clone();
        let model = self.model.clone();
        let channel = self.channel;
        let stop = Arc::clone(&self.stop);
        self.worker = Some(thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(_) => return,
            };
            runtime.block_on(run_worker(key, model, channel, receiver, callback, stop));
        }));
        Ok(())
    }

    fn enqueue(&self, chunk: PcmChunk) -> bool {
        self.input
            .as_ref()
            .is_some_and(|sender| sender.try_send(chunk).is_ok())
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.input.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for DeepgramProvider {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn run_worker(
    key: String,
    model: String,
    channel: SpeakerChannel,
    mut receiver: mpsc::Receiver<PcmChunk>,
    callback: Box<dyn Fn(TranscriptEvent) + Send + Sync>,
    stop: Arc<AtomicBool>,
) {
    let mut reconnects = 0;
    while !stop.load(Ordering::Acquire) {
        match run_session(&key, &model, channel, &mut receiver, &callback, &stop).await {
            Ok(()) => reconnects = 0,
            Err(_) if stop.load(Ordering::Acquire) => break,
            Err(_) if reconnects < MAX_RECONNECTS => {
                let delay = Duration::from_millis(100 * 2_u64.pow(reconnects as u32));
                reconnects += 1;
                tokio::time::sleep(delay).await;
            }
            Err(_) => break,
        }
    }
}

async fn run_session(
    key: &str,
    model: &str,
    channel: SpeakerChannel,
    receiver: &mut mpsc::Receiver<PcmChunk>,
    callback: &(dyn Fn(TranscriptEvent) + Send + Sync),
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    let url = format!(
        "{ENDPOINT}?model={}&encoding=linear16&sample_rate=16000&channels=1&interim_results=true&punctuate=true&smart_format=true",
        encode_query_component(model)
    );
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Token {key}"))?,
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
    let started = Instant::now();
    let mut idle = tokio::time::interval(KEEPALIVE);
    idle.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        if stop.load(Ordering::Acquire) || started.elapsed() >= RENEW_AFTER {
            let _ = socket.send(Message::Close(None)).await;
            return Ok(());
        }
        tokio::select! {
            chunk = receiver.recv() => match chunk {
                Some(chunk) => socket.send(Message::Binary(pcm_bytes(&chunk).into())).await?,
                None => return Ok(()),
            },
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    if let Some((transcript, finality)) = parse_transcript(text.as_ref())? {
                        callback(TranscriptEvent { channel, text: transcript, finality });
                    }
                }
                Some(Ok(Message::Close(_))) | None => anyhow::bail!("deepgram connection closed"),
                Some(Ok(_)) => {}
                Some(Err(_)) => anyhow::bail!("deepgram connection failed"),
            },
            _ = idle.tick() => {
                socket.send(Message::Text(r#"{"type":"KeepAlive"}"#.to_string().into())).await?;
            }
        }
    }
}

fn pcm_bytes(chunk: &PcmChunk) -> Vec<u8> {
    chunk
        .samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
}

#[derive(Debug, Deserialize)]
struct DeepgramResponse {
    #[serde(default, rename = "type")]
    message_type: Option<String>,
    #[serde(default)]
    is_final: bool,
    #[serde(default)]
    channel: Option<ChannelResult>,
    #[serde(default)]
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ChannelResult {
    #[serde(default)]
    alternatives: Vec<Alternative>,
}

#[derive(Debug, Deserialize)]
struct Alternative {
    #[serde(default)]
    transcript: String,
}

fn encode_query_component(value: &str) -> String {
    value.bytes().fold(String::new(), |mut encoded, byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
        encoded
    })
}

/// Parse one Deepgram result without exposing provider errors or credentials.
fn parse_transcript(payload: &str) -> anyhow::Result<Option<(String, Finality)>> {
    let response: DeepgramResponse = serde_json::from_str(payload)
        .map_err(|_| anyhow::anyhow!("malformed Deepgram response"))?;
    if response.error.is_some()
        || response
            .message_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("error"))
    {
        anyhow::bail!("Deepgram provider error");
    }
    let transcript = response
        .channel
        .as_ref()
        .and_then(|channel| channel.alternatives.first())
        .map(|alternative| alternative.transcript.trim())
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    Ok(transcript.map(|text| {
        let finality = if response.is_final {
            Finality::Final
        } else {
            Finality::Interim
        };
        (text, finality)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "super-secret-key";

    fn payload(transcript: &str, is_final: bool) -> String {
        format!(
            r#"{{"is_final":{is_final},"channel":{{"alternatives":[{{"transcript":"{transcript}"}}]}}}}"#
        )
    }

    #[test]
    fn parses_interim_with_finality_and_without_secret() {
        let event = parse_transcript(&payload(" hello ", false))
            .unwrap()
            .unwrap();
        assert_eq!(event, ("hello".to_string(), Finality::Interim));
        assert!(!format!("{event:?}").contains(KEY));
    }

    #[test]
    fn parses_final_result() {
        assert_eq!(
            parse_transcript(&payload("done", true)).unwrap(),
            Some(("done".into(), Finality::Final))
        );
    }

    #[test]
    fn ignores_empty_transcript() {
        assert_eq!(parse_transcript(&payload(" ", false)).unwrap(), None);
    }

    #[test]
    fn rejects_malformed_json_without_payload_or_secret() {
        let error = parse_transcript(&format!("{KEY} {{"))
            .unwrap_err()
            .to_string();
        assert!(!error.contains(KEY));
        assert!(!error.contains("{"));
    }

    #[test]
    fn rejects_provider_error_without_payload_or_secret() {
        let error = parse_transcript(&format!(r#"{{"type":"Error","error":"{KEY}"}}"#))
            .unwrap_err()
            .to_string();
        assert_eq!(error, "Deepgram provider error");
        assert!(!error.contains(KEY));
    }

    #[test]
    fn provider_implements_shared_interface() {
        fn assert_provider<T: SttProvider>() {}
        assert_provider::<DeepgramProvider>();
    }
}
