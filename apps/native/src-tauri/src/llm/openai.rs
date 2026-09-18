//! OpenAI adapter — `POST /v1/chat/completions` SSE streaming.

use std::future::Future;
use std::pin::Pin;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::{json, Value};

use super::{
    check_status, stream_sse, ChatMessage, ContentPart, LlmError, Provider, REQUEST_TIMEOUT,
};

const CHAT_URL: &str = "https://api.openai.com/v1/chat/completions";
const MODELS_URL: &str = "https://api.openai.com/v1/models";

/// `api_key` is `None` when the keystore has no entry for this provider;
/// `client` carries the shared 120s request timeout.
pub struct OpenAiProvider {
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(api_key: Option<String>, model: String) -> Self {
        Self {
            api_key,
            model,
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("reqwest client builder failed"),
        }
    }

    /// One SSE line → a token. `data:` payloads carrying an `error` field
    /// (and malformed `data:` JSON) abort via `Err`; `[DONE]`, keepalives
    /// and non-`data:` lines are `Ok(None)`. `pub(crate)` so the shared
    /// stream tests can drive it.
    pub(crate) fn parse_event(line: &str) -> Result<Option<String>, LlmError> {
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(None);
        };
        let data = data.trim_start();
        if data.is_empty() || data == "[DONE]" {
            return Ok(None);
        }
        let v: Value =
            serde_json::from_str(data).map_err(|e| super::malformed_stream("openai", e))?;
        if let Some(err) = v.get("error") {
            return Err(super::stream_error(err));
        }
        Ok(v["choices"][0]["delta"]["content"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from))
    }

    /// Test seam per the task brief — same decode as [`Self::parse_event`]
    /// but errors collapse to `None`.
    pub fn parse_stream_line(line: &str) -> Option<String> {
        Self::parse_event(line).ok().flatten()
    }

    /// Wire body: `{model, stream, temperature, max_tokens, messages}` with
    /// system kept as a message and images as `data:` URLs.
    fn request_body(&self, msgs: &[ChatMessage]) -> Value {
        let messages: Vec<Value> = msgs
            .iter()
            .map(|m| {
                let content: Vec<Value> = m
                    .content
                    .iter()
                    .map(|p| match p {
                        ContentPart::Text(t) => json!({"type": "text", "text": t}),
                        ContentPart::ImageJpeg(bytes) => json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:image/jpeg;base64,{}", B64.encode(bytes))
                            },
                        }),
                    })
                    .collect();
                json!({"role": m.role.as_str(), "content": content})
            })
            .collect();
        json!({
            "model": self.model,
            "stream": true,
            "temperature": 0.7,
            "max_tokens": 2048,
            "messages": messages,
        })
    }

    async fn stream_chat_inner(
        &self,
        msgs: &[ChatMessage],
        on_token: &mut (dyn FnMut(&str) + Send),
    ) -> Result<String, LlmError> {
        let key = self.api_key.as_deref().ok_or(LlmError::Auth)?;
        let req = self
            .client
            .post(CHAT_URL)
            .bearer_auth(key)
            .json(&self.request_body(msgs));
        stream_sse(req, Self::parse_event, on_token).await
    }

    async fn validate_inner(&self) -> Result<(), LlmError> {
        let key = self.api_key.as_deref().ok_or(LlmError::Auth)?;
        let req = self.client.get(MODELS_URL).bearer_auth(key);
        check_status(req.send().await?).await?;
        Ok(())
    }
}

impl std::fmt::Debug for OpenAiProvider {
    /// `api_key` is redacted — secrets never reach logs (same rule as
    /// `keystore::Keyring`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiProvider")
            .field("api_key", &self.api_key.as_ref().map(|_| "…"))
            .field("model", &self.model)
            .field("client", &self.client)
            .finish()
    }
}

impl Provider for OpenAiProvider {
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
    use crate::llm::Role;

    #[test]
    fn openai_parses_delta() {
        assert_eq!(
            OpenAiProvider::parse_stream_line(
                r#"data: {"choices":[{"delta":{"content":"Hel"}}]}"#
            ),
            Some("Hel".into())
        );
        assert_eq!(OpenAiProvider::parse_stream_line("data: [DONE]"), None);
    }

    #[test]
    fn openai_skips_non_token_deltas() {
        // First delta carries only the role; usage/keepalive lines carry no
        // `data:` payload at all — all map to None.
        assert_eq!(
            OpenAiProvider::parse_stream_line(
                r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#
            ),
            None
        );
        assert_eq!(OpenAiProvider::parse_stream_line(": keepalive"), None);
        assert_eq!(OpenAiProvider::parse_stream_line("event: ping"), None);
        assert_eq!(OpenAiProvider::parse_stream_line("data:"), None);
    }

    #[test]
    fn openai_error_event_aborts_via_parse_event() {
        // A `data:` payload with an `error` field must abort the stream —
        // `parse_event` surfaces Err; the Option wrapper hides it as None.
        let err = OpenAiProvider::parse_event(
            r#"data: {"error":{"message":"boom","code":500}}"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, LlmError::Http { status: 500, .. }),
            "{err:?}"
        );
        assert_eq!(
            OpenAiProvider::parse_stream_line(r#"data: {"error":{"message":"boom"}}"#),
            None
        );
    }

    #[test]
    fn openai_malformed_data_json_aborts_via_parse_event() {
        assert!(OpenAiProvider::parse_event("data: {not json").is_err());
    }

    #[test]
    fn openai_body_keeps_system_as_a_message() {
        // OpenAI accepts system inline — it is NOT hoisted like Anthropic's.
        let p = OpenAiProvider::new(None, "gpt-4o".into());
        let body = p.request_body(&[
            ChatMessage::text(Role::System, "be brief"),
            ChatMessage::text(Role::User, "hi"),
        ]);
        let roles: Vec<&str> = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["system", "user"]);
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn openai_body_encodes_images_as_data_urls() {
        let p = OpenAiProvider::new(None, "gpt-4o".into());
        let body = p.request_body(&[ChatMessage::user_with_image("see", vec![1, 2, 3])]);
        assert_eq!(
            body["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/jpeg;base64,AQID"
        );
    }

    #[test]
    fn openai_debug_never_leaks_api_key() {
        let p = OpenAiProvider::new(Some("sk-SENTINEL".into()), "gpt-4o".into());
        assert!(!format!("{p:?}").contains("sk-SENTINEL"));
    }
}
