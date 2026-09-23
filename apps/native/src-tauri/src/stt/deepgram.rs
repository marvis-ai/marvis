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

#[derive(Debug)]
enum SessionFailure {
    Terminal(String),
    Transport,
}

/// A Deepgram streaming provider. The key is retained only for the lifetime of
/// this provider and is never written to disk or included in diagnostics.
pub struct DeepgramProvider {
    key: String,
    model: String,
    channel: SpeakerChannel,
    endpoint: String,
    input: Option<mpsc::Sender<PcmChunk>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl DeepgramProvider {
    pub fn new(key: String, model: String, channel: SpeakerChannel) -> Self {
        Self::with_endpoint(key, model, channel, ENDPOINT.to_string())
    }

    fn with_endpoint(
        key: String,
        model: String,
        channel: SpeakerChannel,
        endpoint: String,
    ) -> Self {
        Self {
            key,
            model,
            channel,
            endpoint,
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
        error_callback: Box<dyn Fn(String) + Send + Sync>,
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
        let endpoint = self.endpoint.clone();
        let stop = Arc::clone(&self.stop);
        self.worker = Some(thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(_) => {
                    error_callback("Deepgram runtime initialization failed".to_string());
                    return;
                }
            };
            runtime.block_on(run_worker(
                key,
                model,
                channel,
                endpoint,
                receiver,
                callback,
                error_callback,
                stop,
            ));
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

#[allow(clippy::too_many_arguments)]
async fn run_worker(
    key: String,
    model: String,
    channel: SpeakerChannel,
    endpoint: String,
    mut receiver: mpsc::Receiver<PcmChunk>,
    callback: Box<dyn Fn(TranscriptEvent) + Send + Sync>,
    error_callback: Box<dyn Fn(String) + Send + Sync>,
    stop: Arc<AtomicBool>,
) {
    let mut reconnects = 0;
    while !stop.load(Ordering::Acquire) {
        match run_session(
            &key,
            &model,
            channel,
            &endpoint,
            &mut receiver,
            &callback,
            &stop,
        )
        .await
        {
            Ok(()) => reconnects = 0,
            Err(SessionFailure::Terminal(message)) => {
                if !stop.load(Ordering::Acquire) {
                    error_callback(message);
                }
                break;
            }
            Err(SessionFailure::Transport) if stop.load(Ordering::Acquire) => break,
            Err(SessionFailure::Transport) if reconnects < MAX_RECONNECTS => {
                let delay = Duration::from_millis(100 * 2_u64.pow(reconnects as u32));
                reconnects += 1;
                tokio::time::sleep(delay).await;
            }
            Err(SessionFailure::Transport) => {
                error_callback("Deepgram connection failed after reconnect retries".to_string());
                break;
            }
        }
    }
}

fn classify_connect_error(error: tokio_tungstenite::tungstenite::Error) -> SessionFailure {
    if let tokio_tungstenite::tungstenite::Error::Http(response) = &error {
        let status = response.status();
        if status.is_client_error() {
            return SessionFailure::Terminal(format!(
                "Deepgram authentication or protocol error (HTTP {})",
                status.as_u16()
            ));
        }
    }
    SessionFailure::Transport
}

async fn run_session(
    key: &str,
    model: &str,
    channel: SpeakerChannel,
    endpoint: &str,
    receiver: &mut mpsc::Receiver<PcmChunk>,
    callback: &(dyn Fn(TranscriptEvent) + Send + Sync),
    stop: &AtomicBool,
) -> Result<(), SessionFailure> {
    let url = format!(
        "{endpoint}?model={}&encoding=linear16&sample_rate=16000&channels=1&interim_results=true&punctuate=true&smart_format=true",
        encode_query_component(model)
    );
    let mut request = url.into_client_request().map_err(|_| {
        SessionFailure::Terminal("Deepgram request configuration is invalid".to_string())
    })?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Token {key}")).map_err(|_| {
            SessionFailure::Terminal("Deepgram authentication configuration is invalid".to_string())
        })?,
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(classify_connect_error)?;
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
                Some(chunk) => socket
                    .send(Message::Binary(pcm_bytes(&chunk).into()))
                    .await
                    .map_err(|_| SessionFailure::Transport)?,
                None => return Ok(()),
            },
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    if let Some((transcript, finality)) = parse_transcript(text.as_ref())
                        .map_err(|error| SessionFailure::Terminal(error.to_string()))?
                    {
                        callback(TranscriptEvent { channel, text: transcript, finality });
                    }
                }
                Some(Ok(Message::Close(_))) | None => {
                    return Err(SessionFailure::Transport)
                }
                Some(Ok(_)) => {}
                Some(Err(_)) => return Err(SessionFailure::Transport),
            },
            _ = idle.tick() => {
                socket
                    .send(Message::Text(r#"{"type":"KeepAlive"}"#.to_string().into()))
                    .await
                    .map_err(|_| SessionFailure::Transport)?;
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
    use std::sync::Mutex;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

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

    #[tokio::test]
    async fn mocked_session_emits_channel_tagged_transcript_and_reports_close() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}/v1/listen", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(payload("hello", true).into()))
                .await
                .unwrap();
            socket.send(Message::Close(None)).await.unwrap();
        });

        let (_sender, mut receiver) = mpsc::channel(1);
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_callback = Arc::clone(&events);
        let stop = AtomicBool::new(false);
        let result = run_session(
            KEY,
            "nova-2",
            SpeakerChannel::Them,
            &endpoint,
            &mut receiver,
            &|event| events_for_callback.lock().unwrap().push(event),
            &stop,
        )
        .await;

        assert!(result.is_err());
        {
            let events = events.lock().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].channel, SpeakerChannel::Them);
            assert_eq!(events[0].text, "hello");
            assert_eq!(events[0].finality, Finality::Final);
            assert!(!format!("{events:?}").contains(KEY));
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn reports_provider_protocol_error_without_reconnecting() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}/v1/listen", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(
                    format!(r#"{{"type":"Error","error":"{KEY}"}}"#).into(),
                ))
                .await
                .unwrap();
        });
        let (_sender, receiver) = mpsc::channel(1);
        let errors = Arc::new(Mutex::new(Vec::new()));
        let errors_for_callback = Arc::clone(&errors);
        run_worker(
            KEY.to_string(),
            "nova-2".to_string(),
            SpeakerChannel::Me,
            endpoint,
            receiver,
            Box::new(|_| {}),
            Box::new(move |error| errors_for_callback.lock().unwrap().push(error)),
            Arc::new(AtomicBool::new(false)),
        )
        .await;
        assert_eq!(
            errors.lock().unwrap().as_slice(),
            &["Deepgram provider error"]
        );
        assert!(!errors.lock().unwrap()[0].contains(KEY));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn reports_terminal_error_after_reconnect_exhaustion() {
        let (_sender, receiver) = mpsc::channel(1);
        let errors = Arc::new(Mutex::new(Vec::new()));
        let errors_for_callback = Arc::clone(&errors);
        let stop = Arc::new(AtomicBool::new(false));
        run_worker(
            KEY.to_string(),
            "nova-2".to_string(),
            SpeakerChannel::Me,
            "ws://127.0.0.1:1/v1/listen".to_string(),
            receiver,
            Box::new(|_| {}),
            Box::new(move |error| errors_for_callback.lock().unwrap().push(error)),
            stop,
        )
        .await;
        assert_eq!(
            errors.lock().unwrap().as_slice(),
            &["Deepgram connection failed after reconnect retries"]
        );
    }

    #[test]
    fn provider_implements_shared_interface() {
        fn assert_provider<T: SttProvider>() {}
        assert_provider::<DeepgramProvider>();
    }
}
