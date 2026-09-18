//! Gemini adapter — stub holding credentials and a shared client; real
//! `streamGenerateContent` SSE streaming lands in Task 6.

use std::future::Future;
use std::pin::Pin;

use super::{ChatMessage, LlmError, Provider, REQUEST_TIMEOUT};

/// `api_key` is `None` when the keystore has no entry for this provider;
/// `client` carries the shared 120s request timeout.
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
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("reqwest client builder failed"),
        }
    }

    /// Implemented in Task 6 (`POST :streamGenerateContent?alt=sse`, SSE).
    async fn stream_chat_inner(
        &self,
        _msgs: &[ChatMessage],
        _on_token: &mut (dyn FnMut(&str) + Send),
    ) -> Result<String, LlmError> {
        Err(LlmError::NoModel)
    }

    /// Implemented in Task 6 (`GET /v1beta/models`).
    async fn validate_inner(&self) -> Result<(), LlmError> {
        Err(LlmError::NoModel)
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
