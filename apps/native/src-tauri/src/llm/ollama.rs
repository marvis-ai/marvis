//! Ollama adapter — stub holding credentials and a shared client; real
//! `/api/chat` NDJSON streaming lands in Task 6.

use std::future::Future;
use std::pin::Pin;

use super::{ChatMessage, LlmError, Provider, REQUEST_TIMEOUT};

/// Local Ollama needs no key, but the field stays for a uniform
/// `make_provider` signature; `client` carries the shared 120s timeout.
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
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("reqwest client builder failed"),
        }
    }

    /// Implemented in Task 6 (`POST /api/chat`, NDJSON lines).
    async fn stream_chat_inner(
        &self,
        _msgs: &[ChatMessage],
        _on_token: &mut (dyn FnMut(&str) + Send),
    ) -> Result<String, LlmError> {
        Err(LlmError::NoModel)
    }

    /// Implemented in Task 6 (`GET /api/tags`).
    async fn validate_inner(&self) -> Result<(), LlmError> {
        Err(LlmError::NoModel)
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
    ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send + 'a>> {
        Box::pin(async move { self.stream_chat_inner(msgs, on_token).await })
    }

    fn validate<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), LlmError>> + Send + 'a>> {
        Box::pin(async move { self.validate_inner().await })
    }
}
