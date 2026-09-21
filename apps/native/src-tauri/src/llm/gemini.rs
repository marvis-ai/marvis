//! Gemini adapter — `POST :streamGenerateContent?alt=sse` SSE streaming.
//!
//! The API key travels in the `x-goog-api-key` header — never as a `?key=`
//! query param, because `reqwest::Error`'s Display embeds the request URL
//! and would leak the key into `LlmError::Network` (and from there, logs).

use std::future::Future;
use std::pin::Pin;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::{json, Value};

use super::{
    check_status, stream_sse, ChatMessage, CONNECT_TIMEOUT, ContentPart, LlmError, Provider,
    Role, VALIDATE_TIMEOUT,
};

const MODELS_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";

/// `api_key` is `None` when the keystore has no entry for this provider;
/// `client` carries only a connect timeout — streamed bodies run unbounded.
pub struct GeminiProvider {
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
}

impl GeminiProvider {
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

    /// One SSE line → a token. `data:` payloads carrying an `error` field
    /// (and malformed `data:` JSON) abort via `Err`; metadata-only chunks
    /// and non-`data:` lines are `Ok(None)`.
    pub(crate) fn parse_event(line: &str) -> Result<Option<String>, LlmError> {
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(None);
        };
        let data = data.trim_start();
        if data.is_empty() {
            return Ok(None);
        }
        let v: Value =
            serde_json::from_str(data).map_err(|e| super::malformed_stream("gemini", e))?;
        if let Some(err) = v.get("error") {
            return Err(super::stream_error(err));
        }
        // A chunk may carry several parts — concatenate their text.
        let text: String = v["candidates"][0]["content"]["parts"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<String>()
            })
            .unwrap_or_default();
        Ok(if text.is_empty() { None } else { Some(text) })
    }

    /// Test seam per the task brief — same decode as [`Self::parse_event`]
    /// but errors collapse to `None`.
    #[allow(dead_code)] // test-only seam
    pub fn parse_stream_line(line: &str) -> Option<String> {
        Self::parse_event(line).ok().flatten()
    }

    /// The model lives in the URL path; the key NEVER does — see module
    /// docs. `?alt=sse` is the only query param.
    fn stream_url(&self) -> String {
        format!("{MODELS_URL}/{}:streamGenerateContent?alt=sse", self.model)
    }

    /// Wire body: `{system_instruction?, contents}` — system text goes to
    /// `system_instruction.parts`, and the assistant role is `"model"`.
    fn request_body(&self, msgs: &[ChatMessage]) -> Value {
        let system: Vec<&str> = msgs
            .iter()
            .filter(|m| m.role == Role::System)
            .flat_map(|m| {
                m.content.iter().filter_map(|p| match p {
                    ContentPart::Text(t) => Some(t.as_str()),
                    _ => None,
                })
            })
            .collect();
        let contents: Vec<Value> = msgs
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| {
                let role = match m.role {
                    Role::Assistant => "model",
                    r => r.as_str(),
                };
                let parts: Vec<Value> = m
                    .content
                    .iter()
                    .map(|p| match p {
                        ContentPart::Text(t) => json!({"text": t}),
                        ContentPart::ImageJpeg(bytes) => json!({
                            "inline_data": {
                                "mime_type": "image/jpeg",
                                "data": B64.encode(bytes),
                            },
                        }),
                    })
                    .collect();
                json!({"role": role, "parts": parts})
            })
            .collect();
        let mut body = json!({"contents": contents});
        if !system.is_empty() {
            body["system_instruction"] = json!({
                "parts": system.iter().map(|t| json!({"text": t})).collect::<Vec<_>>()
            });
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
            .post(self.stream_url())
            .header("x-goog-api-key", key)
            .json(&self.request_body(msgs));
        stream_sse(req, Self::parse_event, on_token).await
    }

    async fn validate_inner(&self) -> Result<(), LlmError> {
        let key = self.api_key.as_deref().ok_or(LlmError::Auth)?;
        let req = self
            .client
            .get(MODELS_URL)
            .header("x-goog-api-key", key)
            .timeout(VALIDATE_TIMEOUT);
        check_status(req.send().await?).await?;
        Ok(())
    }
}

impl std::fmt::Debug for GeminiProvider {
    /// `api_key` is redacted — secrets never reach logs (same rule as
    /// `keystore::Keyring`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeminiProvider")
            .field("api_key", &self.api_key.as_ref().map(|_| "…"))
            .field("model", &self.model)
            .field("client", &self.client)
            .finish()
    }
}

impl Provider for GeminiProvider {
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
    fn gemini_parses_candidate_parts() {
        assert_eq!(
            GeminiProvider::parse_stream_line(
                r#"data: {"candidates":[{"content":{"parts":[{"text":"Yo"}]}}]}"#
            ),
            Some("Yo".into())
        );
    }

    #[test]
    fn gemini_skips_metadata_and_empty_chunks() {
        // usageMetadata-only trailing chunks and promptFeedback carry no
        // candidate text.
        assert_eq!(
            GeminiProvider::parse_stream_line(
                r#"data: {"usageMetadata":{"totalTokenCount":10}}"#
            ),
            None
        );
        assert_eq!(GeminiProvider::parse_stream_line("data:"), None);
        assert_eq!(GeminiProvider::parse_stream_line(": keepalive"), None);
    }

    #[test]
    fn gemini_error_event_aborts_via_parse_event() {
        let err = GeminiProvider::parse_event(
            r#"data: {"error":{"code":400,"message":"bad request","status":"INVALID_ARGUMENT"}}"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, LlmError::Http { status: 400, .. }),
            "{err:?}"
        );
    }

    #[test]
    fn gemini_system_maps_to_system_instruction_and_model_role() {
        // Gemini takes system text in `system_instruction.parts`, and its
        // assistant role is "model" — not "assistant".
        let p = GeminiProvider::new(None, "gemini-2.0-flash".into());
        let body = p.request_body(&[
            ChatMessage::text(Role::System, "be brief"),
            ChatMessage::text(Role::User, "hi"),
            ChatMessage::text(Role::Assistant, "hello"),
        ]);
        assert_eq!(body["system_instruction"]["parts"][0]["text"], "be brief");
        let roles: Vec<&str> = body["contents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["user", "model"]);
    }

    #[test]
    fn gemini_body_encodes_images_as_inline_data() {
        let p = GeminiProvider::new(None, "gemini-2.0-flash".into());
        let body = p.request_body(&[ChatMessage::user_with_image("see", vec![1, 2, 3])]);
        assert_eq!(
            body["contents"][0]["parts"][1]["inline_data"],
            serde_json::json!({"mime_type": "image/jpeg", "data": "AQID"})
        );
    }

    #[test]
    fn gemini_debug_never_leaks_api_key() {
        let p = GeminiProvider::new(Some("sk-SENTINEL".into()), "gemini".into());
        assert!(!format!("{p:?}").contains("sk-SENTINEL"));
    }
}
