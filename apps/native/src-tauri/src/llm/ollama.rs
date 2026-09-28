//! Ollama adapter — `POST /api/chat` NDJSON streaming (no SSE framing, no
//! auth — it talks to a local daemon on localhost:11434).

use std::future::Future;
use std::pin::Pin;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::{json, Value};

use super::{
    check_status, stream_ndjson, ChatMessage, CONNECT_TIMEOUT, ContentPart, LlmError, ParsedLine,
    Provider, StreamReply, TokenUsage, VALIDATE_TIMEOUT,
};

const CHAT_URL: &str = "http://localhost:11434/api/chat";
const TAGS_URL: &str = "http://localhost:11434/api/tags";

/// Local Ollama needs no key, but the field stays for a uniform
/// `make_provider` signature; `client` carries only a connect timeout —
/// streamed bodies run unbounded.
pub struct OllamaProvider {
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(api_key: Option<String>, model: String) -> Self {
        Self {
            api_key,
            model,
            client: reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .build()
                .expect("reqwest client builder failed"),
        }
    }

    /// One NDJSON line → a token; the terminating `done:true` line also
    /// reports `prompt_eval_count`/`eval_count`. `{"error":"…"}` lines
    /// (and malformed JSON) abort via `Err` — Ollama closes the body
    /// after the done line.
    pub(crate) fn parse_event(line: &str) -> Result<ParsedLine, LlmError> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(ParsedLine::NONE);
        }
        let v: Value =
            serde_json::from_str(line).map_err(|e| super::malformed_stream("ollama", e))?;
        if let Some(err) = v.get("error") {
            return Err(super::stream_error(err));
        }
        Ok(ParsedLine {
            token: v["message"]["content"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(String::from),
            usage: (v.get("prompt_eval_count").is_some() || v.get("eval_count").is_some())
                .then(|| TokenUsage {
                    input: v["prompt_eval_count"].as_u64(),
                    output: v["eval_count"].as_u64(),
                }),
        })
    }

    /// Test seam per the task brief — same decode as [`Self::parse_event`]
    /// but errors collapse to `None`.
    #[allow(dead_code)] // test-only seam
    pub fn parse_stream_line(line: &str) -> Option<String> {
        Self::parse_event(line).ok().and_then(|p| p.token)
    }

    /// Wire body: `{model, stream, messages}` — plain string content,
    /// system stays a message, images go in a separate `images` array
    /// (included only when present).
    fn request_body(&self, msgs: &[ChatMessage]) -> Value {
        let messages: Vec<Value> = msgs
            .iter()
            .map(|m| {
                let text: String = m
                    .content
                    .iter()
                    .filter_map(|p| match p {
                        ContentPart::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect();
                let images: Vec<String> = m
                    .content
                    .iter()
                    .filter_map(|p| match p {
                        ContentPart::ImageJpeg(b) => Some(B64.encode(b)),
                        _ => None,
                    })
                    .collect();
                let mut msg = json!({"role": m.role.as_str(), "content": text});
                if !images.is_empty() {
                    msg["images"] = json!(images);
                }
                msg
            })
            .collect();
        json!({"model": self.model, "stream": true, "messages": messages})
    }

    async fn stream_chat_inner(
        &self,
        msgs: &[ChatMessage],
        on_token: &mut (dyn FnMut(&str) + Send),
    ) -> Result<StreamReply, LlmError> {
        // No api_key — Ollama is a local daemon with no auth header.
        let req = self.client.post(CHAT_URL).json(&self.request_body(msgs));
        stream_ndjson(req, Self::parse_event, on_token).await
    }

    async fn validate_inner(&self) -> Result<(), LlmError> {
        let req = self.client.get(TAGS_URL).timeout(VALIDATE_TIMEOUT);
        check_status(req.send().await?).await?;
        Ok(())
    }
}

impl std::fmt::Debug for OllamaProvider {
    /// `api_key` is redacted — secrets never reach logs (same rule as
    /// `keystore::Keyring`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OllamaProvider")
            .field("api_key", &self.api_key.as_ref().map(|_| "…"))
            .field("model", &self.model)
            .field("client", &self.client)
            .finish()
    }
}

impl Provider for OllamaProvider {
    fn stream_chat<'a>(
        &'a self,
        msgs: &'a [ChatMessage],
        on_token: &'a mut (dyn FnMut(&str) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<StreamReply, LlmError>> + Send + 'a>> {
        Box::pin(async move { self.stream_chat_inner(msgs, on_token).await })
    }

    fn validate<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), LlmError>> + Send + 'a>> {
        Box::pin(async move { self.validate_inner().await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::Role;

    #[test]
    fn ollama_parses_message_content() {
        assert_eq!(
            OllamaProvider::parse_stream_line(
                r#"{"message":{"role":"assistant","content":"Hey"},"done":false}"#
            ),
            Some("Hey".into())
        );
    }

    #[test]
    fn ollama_done_line_reports_eval_counts() {
        let parsed = OllamaProvider::parse_event(
            r#"{"message":{"role":"assistant","content":""},"done":true,"prompt_eval_count":26,"eval_count":8}"#,
        )
        .unwrap();
        assert_eq!(parsed.token, None);
        assert_eq!(parsed.usage.unwrap().input, Some(26));
        assert_eq!(parsed.usage.unwrap().output, Some(8));
    }

    #[test]
    fn ollama_done_line_yields_no_token() {
        // The terminating line may carry an empty content string.
        assert_eq!(
            OllamaProvider::parse_stream_line(
                r#"{"message":{"role":"assistant","content":""},"done":true}"#
            ),
            None
        );
        assert_eq!(OllamaProvider::parse_stream_line(""), None);
    }

    #[test]
    fn ollama_error_line_aborts_via_parse_event() {
        // Ollama reports errors as a bare string field.
        let err = OllamaProvider::parse_event(r#"{"error":"model not found"}"#).unwrap_err();
        assert!(
            matches!(err, LlmError::Http { ref message, .. } if message.contains("model not found")),
            "{err:?}"
        );
    }

    #[test]
    fn ollama_body_collects_text_and_images() {
        // Ollama takes plain string content (system stays a message) plus a
        // separate `images` array — included only when images exist.
        let p = OllamaProvider::new(None, "llama3.2".into());
        let body = p.request_body(&[
            ChatMessage::text(Role::System, "be brief"),
            ChatMessage::user_with_image("see", vec![1, 2, 3]),
        ]);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "be brief");
        assert!(body["messages"][0].get("images").is_none());
        assert_eq!(body["messages"][1]["content"], "see");
        assert_eq!(body["messages"][1]["images"][0], "AQID");
        assert_eq!(body["model"], "llama3.2");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn ollama_debug_never_leaks_api_key() {
        let p = OllamaProvider::new(Some("sk-SENTINEL".into()), "llama".into());
        assert!(!format!("{p:?}").contains("sk-SENTINEL"));
    }
}
