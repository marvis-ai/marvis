//! Anthropic adapter — `POST /v1/messages` SSE streaming.

use std::future::Future;
use std::pin::Pin;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::{json, Value};

use super::{
    check_status, stream_sse, ChatMessage, CONNECT_TIMEOUT, ContentPart, LlmError, Provider,
    Role, VALIDATE_TIMEOUT,
};

const MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const MODELS_URL: &str = "https://api.anthropic.com/v1/models";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// `api_key` is `None` when the keystore has no entry for this provider;
/// `client` carries only a connect timeout — streamed bodies run unbounded.
pub struct AnthropicProvider {
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
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

    /// One SSE line → a token. Only `content_block_delta` events carry
    /// text; `{"type":"error",…}` events (and malformed `data:` JSON)
    /// abort via `Err`; every other event type is `Ok(None)`.
    pub(crate) fn parse_event(line: &str) -> Result<Option<String>, LlmError> {
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(None);
        };
        let data = data.trim_start();
        if data.is_empty() {
            return Ok(None);
        }
        let v: Value =
            serde_json::from_str(data).map_err(|e| super::malformed_stream("anthropic", e))?;
        match v["type"].as_str() {
            Some("content_block_delta") => Ok(v["delta"]["text"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(String::from)),
            Some("error") => Err(super::stream_error(&v["error"])),
            _ => Ok(None),
        }
    }

    /// Test seam per the task brief — same decode as [`Self::parse_event`]
    /// but errors collapse to `None`.
    pub fn parse_stream_line(line: &str) -> Option<String> {
        Self::parse_event(line).ok().flatten()
    }

    /// Wire body: `{model, max_tokens, stream, system?, messages}` — system
    /// messages are hoisted into the top-level `system` string (Anthropic
    /// rejects `role:"system"` inside `messages`).
    fn request_body(&self, msgs: &[ChatMessage]) -> Value {
        let system: Vec<String> = msgs
            .iter()
            .filter(|m| m.role == Role::System)
            .flat_map(|m| {
                m.content.iter().filter_map(|p| match p {
                    ContentPart::Text(t) => Some(t.clone()),
                    _ => None,
                })
            })
            .collect();
        let messages: Vec<Value> = msgs
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| {
                let content: Vec<Value> = m
                    .content
                    .iter()
                    .map(|p| match p {
                        ContentPart::Text(t) => json!({"type": "text", "text": t}),
                        ContentPart::ImageJpeg(bytes) => json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/jpeg",
                                "data": B64.encode(bytes),
                            },
                        }),
                    })
                    .collect();
                json!({"role": m.role.as_str(), "content": content})
            })
            .collect();
        let mut body = json!({
            "model": self.model,
            "max_tokens": 2048,
            "stream": true,
            "messages": messages,
        });
        if !system.is_empty() {
            body["system"] = json!(system.join("\n"));
        }
        body
    }

    async fn stream_chat_inner(
        &self,
        msgs: &[ChatMessage],
        on_token: &mut (dyn FnMut(&str) + Send),
    ) -> Result<String, LlmError> {
        let key = self.api_key.as_deref().ok_or(LlmError::Auth)?;
        let req = self
            .client
            .post(MESSAGES_URL)
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&self.request_body(msgs));
        stream_sse(req, Self::parse_event, on_token).await
    }

    async fn validate_inner(&self) -> Result<(), LlmError> {
        let key = self.api_key.as_deref().ok_or(LlmError::Auth)?;
        let req = self
            .client
            .get(MODELS_URL)
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .timeout(VALIDATE_TIMEOUT);
        check_status(req.send().await?).await?;
        Ok(())
    }
}

impl std::fmt::Debug for AnthropicProvider {
    /// `api_key` is redacted — secrets never reach logs (same rule as
    /// `keystore::Keyring`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("api_key", &self.api_key.as_ref().map(|_| "…"))
            .field("model", &self.model)
            .field("client", &self.client)
            .finish()
    }
}

impl Provider for AnthropicProvider {
    fn stream_chat<'a>(
        &'a self,
        msgs: &'a [ChatMessage],
        on_token: &'a mut (dyn FnMut(&str) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send + 'a>> {
        Box::pin(async move { self.stream_chat_inner(msgs, on_token).await })
    }

    fn validate<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), LlmError>> + Send + 'a>> {
        Box::pin(async move { self.validate_inner().await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_parses_content_block_delta() {
        assert_eq!(
            AnthropicProvider::parse_stream_line(
                r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi"}}"#
            ),
            Some("Hi".into())
        );
    }

    #[test]
    fn anthropic_skips_non_delta_events() {
        for line in [
            r#"data: {"type":"message_start","message":{"role":"assistant"}}"#,
            r#"data: {"type":"content_block_start","content_block":{"type":"text"}}"#,
            r#"data: {"type":"ping"}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            r#"data: {"type":"message_stop"}"#,
            "event: message_start",
        ] {
            assert_eq!(AnthropicProvider::parse_stream_line(line), None, "{line}");
        }
    }

    #[test]
    fn anthropic_error_event_aborts_via_parse_event() {
        let err = AnthropicProvider::parse_event(
            r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, LlmError::Http { ref message, .. } if message.contains("Overloaded")),
            "{err:?}"
        );
        assert_eq!(
            AnthropicProvider::parse_stream_line(
                r#"data: {"type":"error","error":{"message":"x"}}"#
            ),
            None
        );
    }

    #[test]
    fn anthropic_system_messages_move_to_top_level_field() {
        // Anthropic rejects role:"system" inside `messages` — the system
        // prompt travels in the top-level `system` string instead.
        let p = AnthropicProvider::new(None, "claude-sonnet-4-5".into());
        let body = p.request_body(&[
            ChatMessage::text(Role::System, "be brief"),
            ChatMessage::text(Role::User, "hi"),
            ChatMessage::text(Role::Assistant, "hello"),
        ]);
        assert_eq!(body["system"], "be brief");
        let roles: Vec<&str> = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["user", "assistant"]);
    }

    #[test]
    fn anthropic_body_encodes_images_as_base64_source() {
        let p = AnthropicProvider::new(None, "claude-sonnet-4-5".into());
        let body = p.request_body(&[ChatMessage::user_with_image("see", vec![1, 2, 3])]);
        assert_eq!(
            body["messages"][0]["content"][1],
            serde_json::json!({
                "type": "image",
                "source": {"type": "base64", "media_type": "image/jpeg", "data": "AQID"}
            })
        );
    }

    #[test]
    fn anthropic_debug_never_leaks_api_key() {
        let p = AnthropicProvider::new(Some("sk-SENTINEL".into()), "claude".into());
        assert!(!format!("{p:?}").contains("sk-SENTINEL"));
    }
}
