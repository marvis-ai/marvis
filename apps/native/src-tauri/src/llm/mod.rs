//! LLM providers — the [`Provider`] trait every backend implements, the
//! message types they consume, [`ProviderKind`] for picking one, and
//! [`make_provider`], the factory that builds it.
//!
//! `Provider` returns boxed futures instead of using `async fn` so that
//! `Box<dyn Provider>` is object-safe — callers hold whichever provider the
//! config names. Implementations wrap an inner `async fn`:
//!
//! ```ignore
//! fn stream_chat<'a>(&'a self, msgs: &'a [ChatMessage],
//!                    on_token: &'a mut (dyn FnMut(&str) + Send))
//!     -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send + 'a>>
//! {
//!     Box::pin(async move { self.stream_chat_inner(msgs, on_token).await })
//! }
//! ```
//!
//! The four adapters share the private [`stream_sse`] / [`stream_ndjson`]
//! helpers below: they send one request, verify the status, then drain the
//! byte stream into complete events — UTF-8 decoding only ever runs on a
//! complete event, so multibyte chars split across TCP chunks survive.

mod anthropic;
mod gemini;
mod ollama;
mod openai;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Connect timeout shared by every adapter's client. No total-request
/// timeout is set: a streamed generation may legitimately run long (slow
/// local Ollama inference), so stream lifetime is governed by caller abort.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Per-request timeout applied only to `validate()` calls, which must
/// return promptly for UI key-checks.
pub(crate) const VALIDATE_TIMEOUT: Duration = Duration::from_secs(30);

/// The four supported LLM backends; string form matches `config.toml`'s
/// `models.llm_provider`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Gemini,
    Ollama,
}

impl ProviderKind {
    /// Parse a provider id — exactly `"openai"`, `"anthropic"`, `"gemini"`
    /// or `"ollama"` (case-insensitive). Dropped providers
    /// (`"openai-glass"`, `"portkey"`) and anything else are rejected.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "openai" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            "gemini" => Some(Self::Gemini),
            "ollama" => Some(Self::Ollama),
            _ => None,
        }
    }

    /// The canonical lowercase id; `from_str(k.as_str()) == Some(k)`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Ollama => "ollama",
        }
    }
}

/// Message author; serializes to the lowercase wire name all four APIs use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// One piece of message content. Images stay as raw JPEG bytes — base64
/// encoding happens at each adapter's wire boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentPart {
    Text(String),
    ImageJpeg(Vec<u8>),
}

/// A chat message: a role plus ordered text/image parts.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: Vec<ContentPart>,
}

impl ChatMessage {
    /// Single-part text message.
    pub fn text(role: Role, s: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![ContentPart::Text(s.into())],
        }
    }

    /// User message pairing a caption with one JPEG frame.
    pub fn user_with_image(text: impl Into<String>, jpeg_bytes: Vec<u8>) -> Self {
        Self {
            role: Role::User,
            content: vec![
                ContentPart::Text(text.into()),
                ContentPart::ImageJpeg(jpeg_bytes),
            ],
        }
    }
}

/// Everything that can go wrong on a provider call.
#[derive(Debug, Error)]
pub enum LlmError {
    /// HTTP failure: status code plus the provider's error message body.
    #[error("http {status}: {message}")]
    Http { status: u16, message: String },
    /// 401/403 or a missing key (adapters map HTTP auth failures to this).
    #[error("authentication failed")]
    Auth,
    /// No model selected/configured.
    #[error("no model configured")]
    #[allow(dead_code)] // reserved variant — no adapter produces this yet
    NoModel,
    /// The provider explicitly refused image input.
    #[error("model does not support image input")]
    MultimodalUnsupported,
    /// Transport-level failure (connect, timeout, body stream).
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
}

/// Lowercase substrings providers put in 4xx bodies when a text-only model
/// receives image input — adapters surface these so callers can retry
/// without images instead of hard-failing.
const MULTIMODAL_ERROR_PATTERNS: &[&str] = &[
    "image_url",
    "does not support image",
    "vision",
    "invalid image",
    "unexpected image",
];

impl LlmError {
    /// True when the failure means "this model can't take images" — either
    /// the explicit variant or an `Http` body matching a known provider
    /// rejection phrasing (case-insensitive substring match).
    pub fn is_multimodal(&self) -> bool {
        match self {
            Self::MultimodalUnsupported => true,
            Self::Http { message, .. } => {
                let msg = message.to_lowercase();
                MULTIMODAL_ERROR_PATTERNS
                    .iter()
                    .any(|pat| msg.contains(pat))
            }
            _ => false,
        }
    }

    /// True for the `Auth` variant — 401/403s are mapped to it, so `Http`
    /// never needs checking here.
    #[allow(dead_code)] // auth-flow consumers (re-unlock UX) land later
    pub fn is_auth(&self) -> bool {
        matches!(self, Self::Auth)
    }
}

/// A streaming LLM backend.
///
/// `stream_chat` invokes `on_token` with each decoded text delta as it
/// arrives and resolves to the full accumulated reply; `validate` performs
/// a cheap authenticated call so settings can verify key + model.
pub trait Provider: Send + Sync {
    fn stream_chat<'a>(
        &'a self,
        msgs: &'a [ChatMessage],
        on_token: &'a mut (dyn FnMut(&str) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send + 'a>>;

    fn validate<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), LlmError>> + Send + 'a>>;
}

/// Build the adapter for `kind`. Pure dispatch — no validation beyond
/// construction; a missing or bad `api_key` surfaces as `LlmError::Auth`
/// when the provider is actually called.
pub fn make_provider(
    kind: ProviderKind,
    api_key: Option<String>,
    model: String,
) -> Box<dyn Provider> {
    match kind {
        ProviderKind::OpenAi => Box::new(openai::OpenAiProvider::new(api_key, model)),
        ProviderKind::Anthropic => Box::new(anthropic::AnthropicProvider::new(api_key, model)),
        ProviderKind::Gemini => Box::new(gemini::GeminiProvider::new(api_key, model)),
        ProviderKind::Ollama => Box::new(ollama::OllamaProvider::new(api_key, model)),
    }
}

// ---------------------------------------------------------------------------
// Shared streaming machinery
//
// All four adapters stream tokens the same way: POST → status check → read
// `bytes_stream()` into a byte buffer → split at protocol boundaries →
// decode each COMPLETE piece as UTF-8 → run the adapter's line parser.
// Byte-level buffering is the correctness-critical part: a multibyte char
// split across two chunks must never see a per-chunk lossy decode.
// ---------------------------------------------------------------------------

/// Bytes of an error response body kept for `LlmError::Http::message`.
const ERROR_BODY_CAP: usize = 4096;

/// A line parser: one drained line → a token (`Ok(Some)`), a non-token line
/// (`Ok(None)`), or a stream-aborting failure (`Err`) — e.g. a `data:`
/// payload carrying an `error` field, or malformed JSON.
type LineParser = fn(&str) -> Result<Option<String>, LlmError>;

/// Send `req` as an SSE stream (`\n\n` / `\r\n\r\n` event boundaries) and
/// fold parsed tokens into `on_token` + the returned full reply.
pub(crate) async fn stream_sse(
    req: reqwest::RequestBuilder,
    parse: LineParser,
    on_token: &mut (dyn FnMut(&str) + Send),
) -> Result<String, LlmError> {
    stream_lines(req, drain_sse_lines, parse, on_token).await
}

/// Same as [`stream_sse`] but for NDJSON (Ollama): `\n`-delimited objects.
pub(crate) async fn stream_ndjson(
    req: reqwest::RequestBuilder,
    parse: LineParser,
    on_token: &mut (dyn FnMut(&str) + Send),
) -> Result<String, LlmError> {
    stream_lines(req, drain_ndjson_lines, parse, on_token).await
}

/// Shared streaming core behind [`stream_sse`] / [`stream_ndjson`].
async fn stream_lines(
    req: reqwest::RequestBuilder,
    drain: fn(&mut Vec<u8>) -> Vec<String>,
    parse: LineParser,
    on_token: &mut (dyn FnMut(&str) + Send),
) -> Result<String, LlmError> {
    let resp = check_status(req.send().await?).await?;
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut full = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.extend_from_slice(&chunk);
        for line in drain(&mut buf) {
            if let Some(token) = parse(&line)? {
                on_token(&token);
                full.push_str(&token);
            }
        }
    }
    // Flush an unterminated tail — a final event/line without a trailing
    // delimiter still gets parsed.
    for line in take_tail_lines(&mut buf) {
        if let Some(token) = parse(&line)? {
            on_token(&token);
            full.push_str(&token);
        }
    }
    Ok(full)
}

/// Drain every complete SSE event in `buf` into its constituent lines.
/// Events end at `\n\n` or `\r\n\r\n`; an incomplete tail stays buffered.
pub(crate) fn drain_sse_lines(buf: &mut Vec<u8>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some((end, sep_len)) = find_event_boundary(buf) {
        let event: Vec<u8> = buf.drain(..end + sep_len).collect();
        for line in decode_complete(&event[..end]).lines() {
            lines.push(line.strip_suffix('\r').unwrap_or(line).to_string());
        }
    }
    lines
}

/// Drain every complete `\n`-terminated line in `buf`; tail stays buffered.
pub(crate) fn drain_ndjson_lines(buf: &mut Vec<u8>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(end) = buf.iter().position(|b| *b == b'\n') {
        let line: Vec<u8> = buf.drain(..=end).collect();
        let s = decode_complete(&line[..line.len() - 1]);
        lines.push(s.strip_suffix('\r').unwrap_or(&s).to_string());
    }
    lines
}

/// Earliest `\n\n` or `\r\n\r\n` in `buf` → (event_end, separator_len).
fn find_event_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = find_subslice(buf, b"\n\n").map(|i| (i, 2));
    let crlf = find_subslice(buf, b"\r\n\r\n").map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Decode bytes that form ONE complete event/line. Boundaries are found at
/// byte level, so a multibyte char can only straddle two events — never sit
/// half-decoded inside this slice.
fn decode_complete(bytes: &[u8]) -> String {
    match String::from_utf8(bytes.to_vec()) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

/// Decode whatever `buf` still holds at end-of-stream into lines.
fn take_tail_lines(buf: &mut Vec<u8>) -> Vec<String> {
    if buf.is_empty() {
        return Vec::new();
    }
    let s = decode_complete(buf);
    buf.clear();
    s.lines()
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
        .collect()
}

/// Pass a 2xx response through; otherwise read the body (capped) and map
/// the status to an [`LlmError`].
pub(crate) async fn check_status(
    resp: reqwest::Response,
) -> Result<reqwest::Response, LlmError> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let body: String = body.chars().take(ERROR_BODY_CAP).collect();
    Err(map_http_error(status, &body))
}

/// 401/403 → [`LlmError::Auth`]; bodies matching a provider's "no image
/// support" phrasing → [`LlmError::MultimodalUnsupported`] (via
/// [`LlmError::is_multimodal`]); anything else → [`LlmError::Http`] with
/// the body text, or the status's reason phrase when the body is empty.
pub(crate) fn map_http_error(status: u16, body: &str) -> LlmError {
    if status == 401 || status == 403 {
        return LlmError::Auth;
    }
    let probe = LlmError::Http {
        status,
        message: body.to_string(),
    };
    if probe.is_multimodal() {
        return LlmError::MultimodalUnsupported;
    }
    let message = if body.trim().is_empty() {
        reqwest::StatusCode::from_u16(status)
            .ok()
            .and_then(|s| s.canonical_reason())
            .unwrap_or("unknown error")
            .to_string()
    } else {
        body.trim().to_string()
    };
    LlmError::Http { status, message }
}

/// Build the abort error for an `error` value inside a `data:` payload.
/// Handles both object shapes (`{"error":{"code":n,"message":…}}` — OpenAI,
/// Anthropic, Gemini) and Ollama's bare `{"error":"…"}` string. A missing
/// numeric code maps to status 0 — the error came from the stream body,
/// not an HTTP status line.
pub(crate) fn stream_error(err: &serde_json::Value) -> LlmError {
    if let Some(msg) = err.as_str() {
        return map_http_error(0, msg);
    }
    let message = err
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("provider stream error")
        .to_string();
    let status = err
        .get("code")
        .and_then(|c| c.as_u64())
        .or_else(|| err.get("status").and_then(|s| s.as_u64()))
        .unwrap_or(0)
        .min(u16::MAX as u64) as u16;
    map_http_error(status, &message)
}

/// A `data:` line that isn't JSON means a corrupt stream — abort loudly
/// rather than silently dropping tokens. Status 0: no HTTP status applies.
pub(crate) fn malformed_stream(provider: &str, e: serde_json::Error) -> LlmError {
    LlmError::Http {
        status: 0,
        message: format!("{provider}: malformed stream json: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_from_str_accepts_exactly_the_four_ids() {
        assert_eq!(ProviderKind::from_str("openai"), Some(ProviderKind::OpenAi));
        assert_eq!(
            ProviderKind::from_str("anthropic"),
            Some(ProviderKind::Anthropic)
        );
        assert_eq!(ProviderKind::from_str("gemini"), Some(ProviderKind::Gemini));
        assert_eq!(ProviderKind::from_str("ollama"), Some(ProviderKind::Ollama));
        // Case-insensitive is fine.
        assert_eq!(ProviderKind::from_str("OpenAI"), Some(ProviderKind::OpenAi));
        assert_eq!(ProviderKind::from_str("GEMINI"), Some(ProviderKind::Gemini));
    }

    #[test]
    fn provider_kind_from_str_rejects_dropped_and_unknown_ids() {
        // "openai-glass"/"portkey" were dropped providers — they must not
        // silently map to OpenAI, and neither may empty/unknown strings.
        for bad in ["openai-glass", "portkey", "", "gpt4", "open_ai", " ollama"] {
            assert_eq!(ProviderKind::from_str(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn provider_kind_as_str_roundtrips() {
        for kind in [
            ProviderKind::OpenAi,
            ProviderKind::Anthropic,
            ProviderKind::Gemini,
            ProviderKind::Ollama,
        ] {
            assert_eq!(ProviderKind::from_str(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn chat_message_convenience_ctors() {
        let msg = ChatMessage::text(Role::System, "be brief");
        assert_eq!(msg.role, Role::System);
        assert_eq!(msg.content, vec![ContentPart::Text("be brief".to_string())]);

        let msg = ChatMessage::user_with_image("what's here?", vec![0xff, 0xd8]);
        assert_eq!(msg.role, Role::User);
        assert_eq!(
            msg.content,
            vec![
                ContentPart::Text("what's here?".to_string()),
                ContentPart::ImageJpeg(vec![0xff, 0xd8]),
            ]
        );
    }

    #[test]
    fn role_as_str_matches_wire_names() {
        assert_eq!(Role::System.as_str(), "system");
        assert_eq!(Role::User.as_str(), "user");
        assert_eq!(Role::Assistant.as_str(), "assistant");
    }

    #[test]
    fn http_errors_with_provider_image_patterns_are_multimodal() {
        // Each string is a rejection phrasing a real provider returns when a
        // text-only model gets image input; adapters rely on the retry signal.
        for msg in [
            "image_url is not supported for this model",
            "this model does not support image input",
            "gpt-4o-mini-text is not a vision model",
            "Invalid image data",
            "unexpected image part in request",
        ] {
            let err = LlmError::Http {
                status: 400,
                message: msg.to_string(),
            };
            assert!(err.is_multimodal(), "{msg:?}");
        }
    }

    #[test]
    fn is_multimodal_flags_the_variant_and_rejects_unrelated_errors() {
        assert!(LlmError::MultimodalUnsupported.is_multimodal());

        let plain_http = LlmError::Http {
            status: 500,
            message: "internal server error".to_string(),
        };
        assert!(!plain_http.is_multimodal());
        assert!(!LlmError::Auth.is_multimodal());
        assert!(!LlmError::NoModel.is_multimodal());
    }

    #[test]
    fn is_auth_flags_only_the_auth_variant() {
        assert!(LlmError::Auth.is_auth());
        // 401/403 are mapped to Auth by the adapters, so Http never carries them.
        let http_401 = LlmError::Http {
            status: 401,
            message: "unauthorized".to_string(),
        };
        assert!(!http_401.is_auth());
        assert!(!LlmError::NoModel.is_auth());
    }

    #[tokio::test]
    async fn providers_fail_fast_without_keys_or_server_through_dyn() {
        // Calls go through `Box<dyn Provider>` — proves the trait is
        // object-safe and the factory dispatches every kind. Missing keys
        // map to `Auth` before any network I/O; Ollama has no key gate but
        // a nonsense model must still error (connection refused or a 404
        // from a real local server — both are `Err`).
        for kind in [ProviderKind::OpenAi, ProviderKind::Anthropic, ProviderKind::Gemini] {
            let provider: Box<dyn Provider> = make_provider(kind, None, "some-model".into());
            let mut on_token = |tok: &str| panic!("emitted token {tok:?} without a key");
            let err = provider
                .stream_chat(&[ChatMessage::text(Role::User, "hi")], &mut on_token)
                .await
                .unwrap_err();
            assert!(matches!(err, LlmError::Auth), "{kind:?}: {err:?}");
            let err = provider.validate().await.unwrap_err();
            assert!(matches!(err, LlmError::Auth), "{kind:?}: {err:?}");
        }

        let provider: Box<dyn Provider> = make_provider(
            ProviderKind::Ollama,
            None,
            "definitely-not-a-real-model-xyz".into(),
        );
        let mut on_token = |_: &str| {};
        assert!(provider
            .stream_chat(&[ChatMessage::text(Role::User, "hi")], &mut on_token)
            .await
            .is_err());
        // Ollama's /api/tags is unauthenticated — whether it answers depends
        // on a local server actually running, so no assertion on the result.
        let _ = provider.validate().await;
    }

    #[test]
    fn drain_sse_lines_holds_incomplete_events_and_split_chars() {
        // 'é' is 0xC3 0xA9. Splitting mid-char across two pushes must not
        // corrupt it: decoding only ever runs on COMPLETE events, never on
        // raw stream chunks (a lossy per-chunk decode would mangle this).
        let event = "data: {\"choices\":[{\"delta\":{\"content\":\"héllo\"}}]}\n\n";
        let bytes = event.as_bytes();
        let split = bytes.iter().position(|b| *b == 0xC3).unwrap() + 1;

        let mut buf = Vec::new();
        buf.extend_from_slice(&bytes[..split]);
        assert!(drain_sse_lines(&mut buf).is_empty());

        buf.extend_from_slice(&bytes[split..]);
        let lines = drain_sse_lines(&mut buf);
        assert_eq!(lines.len(), 1);
        assert_eq!(
            openai::OpenAiProvider::parse_stream_line(&lines[0]),
            Some("héllo".into())
        );
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_sse_lines_tolerates_crlf_boundaries() {
        let mut buf = b"data: {\"a\":1}\r\n\r\ndata: {\"b\":2}\n\n".to_vec();
        let lines = drain_sse_lines(&mut buf);
        assert_eq!(lines, vec!["data: {\"a\":1}", "data: {\"b\":2}"]);
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_ndjson_lines_splits_on_newlines_holding_tail() {
        let mut buf = b"{\"a\":1}\n{\"b\":2}\n{\"part".to_vec();
        let lines = drain_ndjson_lines(&mut buf);
        assert_eq!(lines, vec!["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(buf, b"{\"part");
    }

    /// One-shot HTTP server: reads the request head, writes `writes` in
    /// order, returns the URL. Lets tests control exactly how the response
    /// body is fragmented on the wire.
    async fn serve_once(writes: Vec<Vec<u8>>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if sock.read(&mut byte).await.unwrap_or(0) == 0 {
                    return;
                }
                head.push(byte[0]);
            }
            for w in writes {
                if sock.write_all(&w).await.is_err() {
                    return;
                }
            }
        });
        format!("http://{addr}/")
    }

    fn sse_response(body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn stream_sse_aborts_when_an_event_carries_error() {
        // Tokens already emitted are delivered, then the stream dies on the
        // error payload — a silent `None` would swallow a provider failure.
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            "data: {\"error\":{\"message\":\"boom\",\"code\":500}}\n\n"
        );
        let url = serve_once(vec![sse_response(body)]).await;
        let req = reqwest::Client::new().get(&url);
        let mut tokens: Vec<String> = Vec::new();
        let mut on_token = |t: &str| tokens.push(t.to_string());
        let err = stream_sse(req, openai::OpenAiProvider::parse_event, &mut on_token)
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::Http { status: 500, .. }), "{err:?}");
        assert_eq!(tokens, vec!["Hi"]);
    }

    #[tokio::test]
    async fn stream_sse_decodes_chars_split_across_tcp_writes() {
        // Same guarantee as the drain test, but through the real
        // bytes_stream() path with the body fragmented into two writes.
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"héllo\"}}]}\n\n";
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n",
            body.len()
        );
        let bytes = body.as_bytes();
        let split = bytes.iter().position(|b| *b == 0xC3).unwrap() + 1;
        let url = serve_once(vec![
            head.into_bytes(),
            bytes[..split].to_vec(),
            bytes[split..].to_vec(),
        ])
        .await;
        let req = reqwest::Client::new().get(&url);
        let mut tokens: Vec<String> = Vec::new();
        let mut on_token = |t: &str| tokens.push(t.to_string());
        let full = stream_sse(req, openai::OpenAiProvider::parse_event, &mut on_token)
            .await
            .unwrap();
        assert_eq!(full, "héllo");
        assert_eq!(tokens, vec!["héllo"]);
    }

    #[tokio::test]
    async fn stream_sse_maps_401_to_auth() {
        let resp = b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 2\r\n\r\nno".to_vec();
        let url = serve_once(vec![resp]).await;
        let req = reqwest::Client::new().get(&url);
        let mut on_token = |_: &str| {};
        let err = stream_sse(req, openai::OpenAiProvider::parse_event, &mut on_token)
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::Auth), "{err:?}");
    }

    #[tokio::test]
    async fn stream_ndjson_accumulates_lines_until_done() {
        let body = concat!(
            "{\"message\":{\"content\":\"Hel\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"lo\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"\"},\"done\":true}\n"
        );
        let resp = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/x-ndjson\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        )
        .into_bytes();
        let url = serve_once(vec![resp]).await;
        let req = reqwest::Client::new().get(&url);
        let mut tokens: Vec<String> = Vec::new();
        let mut on_token = |t: &str| tokens.push(t.to_string());
        let full = stream_ndjson(req, ollama::OllamaProvider::parse_event, &mut on_token)
            .await
            .unwrap();
        assert_eq!(full, "Hello");
        assert_eq!(tokens, vec!["Hel", "lo"]);
    }
}
