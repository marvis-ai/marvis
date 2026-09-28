//! OpenAI-compatible adapter — any endpoint that speaks the OpenAI chat
//! API (Groq, Together, vLLM, LM Studio, …). OpenRouter was once in this
//! list; it is now a first-class provider built on this same client —
//! [`CompatProvider::openrouter`] pins the base URL, makes the key
//! required, and sends the `X-Title` attribution header.
//!
//! It differs from the hosted OpenAI adapter in exactly three ways, which
//! is why it is a separate provider rather than a flag on that one:
//!
//! 1. **URLs come from config.** `{base}/chat/completions` and
//!    `{base}/models`, where `base` is `config.compat.base_url`.
//! 2. **The key is optional.** A local endpoint (vLLM, LM Studio, Ollama's
//!    OpenAI shim) usually has no auth, so a missing key sends an
//!    unauthenticated request instead of failing with `Auth`.
//! 3. **`/models` is advisory.** Plenty of compatible servers don't
//!    implement it; a 404/405 there proves the endpoint is reachable and
//!    counts as a successful `validate`, so a working server can't be
//!    rejected over a listing route it never claimed to serve.
//!
//! The request body and the SSE token parser are reused verbatim from
//! `llm::openai` — the wire format is the whole point of "compatible".

use std::future::Future;
use std::pin::Pin;

use super::openai::{request_body, OpenAiProvider};
use super::{ChatMessage, LlmError, Provider, StreamReply, CONNECT_TIMEOUT, VALIDATE_TIMEOUT};

/// Statuses on `{base}/models` that mean "reachable, but no listing here".
const MODELS_OPTIONAL_STATUSES: [u16; 2] = [404, 405];

/// OpenRouter's OpenAI-compatible API root — fixed for the first-class
/// `ProviderKind::OpenRouter`, never user-configurable.
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

pub struct CompatProvider {
    api_key: Option<String>,
    model: String,
    /// `config.compat.base_url`, trailing slashes trimmed. Empty means the
    /// user hasn't configured an endpoint yet → [`LlmError::NoEndpoint`].
    base_url: String,
    /// When set, a missing/empty key fails preflight with
    /// [`LlmError::Auth`] — hosted compatible endpoints (OpenRouter) want
    /// this instead of the generic adapter's open-endpoint leniency.
    require_key: bool,
    /// Sent as `X-Title` on every request — OpenRouter's app-attribution
    /// header. `None` for generic endpoints.
    app_title: Option<&'static str>,
    client: reqwest::Client,
}

impl CompatProvider {
    pub fn new(api_key: Option<String>, model: String, base_url: Option<String>) -> Self {
        Self {
            api_key,
            model,
            base_url: normalize_base_url(base_url.as_deref().unwrap_or_default()),
            require_key: false,
            app_title: None,
            client: reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .build()
                .expect("reqwest client builder failed"),
        }
    }

    /// OpenRouter — the hosted compatible provider. Same wire format, but
    /// the endpoint is fixed, `sk-or-…` is required, and requests identify
    /// as Marvis via `X-Title` (OpenRouter's app-attribution header).
    pub fn openrouter(api_key: Option<String>, model: String) -> Self {
        Self {
            require_key: true,
            app_title: Some("Marvis"),
            ..Self::new(api_key, model, Some(OPENROUTER_BASE_URL.to_string()))
        }
    }

    /// `{base}/path`, or [`LlmError::NoEndpoint`] when no base is set.
    fn url(&self, path: &str) -> Result<String, LlmError> {
        if self.base_url.is_empty() {
            return Err(LlmError::NoEndpoint);
        }
        Ok(format!("{}/{path}", self.base_url))
    }

    /// Fail-fast gate before any request is built: a missing endpoint is
    /// a config error, a missing key on a `require_key` provider is auth.
    fn preflight(&self) -> Result<(), LlmError> {
        if self.base_url.is_empty() {
            return Err(LlmError::NoEndpoint);
        }
        if self.require_key && self.api_key.as_deref().is_none_or(str::is_empty) {
            return Err(LlmError::Auth);
        }
        Ok(())
    }

    /// Bearer auth only when a key exists — an open endpoint is valid —
    /// plus `X-Title` when this is a named hosted provider.
    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = match self.app_title {
            Some(title) => req.header("X-Title", title),
            None => req,
        };
        match self.api_key.as_deref() {
            Some(key) if !key.is_empty() => req.bearer_auth(key),
            _ => req,
        }
    }

    async fn stream_chat_inner(
        &self,
        msgs: &[ChatMessage],
        on_token: &mut (dyn FnMut(&str) + Send),
    ) -> Result<StreamReply, LlmError> {
        self.preflight()?;
        let req = self
            .authed(self.client.post(self.url("chat/completions")?))
            .json(&request_body(&self.model, msgs));
        super::stream_sse(req, OpenAiProvider::parse_event, on_token).await
    }

    async fn validate_inner(&self) -> Result<(), LlmError> {
        self.preflight()?;
        let req = self
            .authed(self.client.get(self.url("models")?))
            .timeout(VALIDATE_TIMEOUT);
        match super::check_status(req.send().await?).await {
            Ok(_) => Ok(()),
            // Reachable but no model listing — see the module note.
            Err(LlmError::Http { status, .. }) if MODELS_OPTIONAL_STATUSES.contains(&status) => {
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

impl std::fmt::Debug for CompatProvider {
    /// `api_key` is redacted — secrets never reach logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompatProvider")
            .field("api_key", &self.api_key.as_ref().map(|_| "…"))
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("app_title", &self.app_title)
            .finish()
    }
}

impl Provider for CompatProvider {
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

/// Trim whitespace and trailing slashes so `url()` can always join with a
/// single separator. Validation of the scheme happens in `config_set`.
pub fn normalize_base_url(raw: &str) -> String {
    raw.trim().trim_end_matches('/').to_string()
}

/// `true` when `raw` is an absolute `http(s)://` URL with a host — the
/// only shape `config.compat.base_url` accepts (DESIGN.md §6).
pub fn is_valid_base_url(raw: &str) -> bool {
    let trimmed = raw.trim();
    // Schemes are case-insensitive; the rest of the URL is not, so only
    // the prefix is folded.
    let scheme_len = ["https://", "http://"]
        .into_iter()
        .find(|s| trimmed.len() >= s.len() && trimmed[..s.len()].eq_ignore_ascii_case(s))
        .map(str::len);
    let Some(scheme_len) = scheme_len else {
        return false;
    };
    // A host must exist before the first `/`, `?` or `#`.
    !trimmed[scheme_len..]
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .is_empty()
}

/// `GET {base}/models` → `data[].id`. Any failure (unreachable, no
/// listing route, unexpected body) maps to an empty list — the model
/// field is free text for compatible endpoints, so an empty dropdown is a
/// valid outcome rather than an error.
pub async fn list_models(base_url: &str, api_key: Option<String>) -> Vec<String> {
    let base = normalize_base_url(base_url);
    if base.is_empty() {
        return Vec::new();
    }
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(VALIDATE_TIMEOUT)
        .build()
    else {
        return Vec::new();
    };
    let mut req = client.get(format!("{base}/models"));
    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(key);
    }
    let Ok(resp) = req.send().await else {
        return Vec::new();
    };
    if !resp.status().is_success() {
        return Vec::new();
    }
    let Ok(body) = resp.json::<serde_json::Value>().await else {
        return Vec::new();
    };
    body["data"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter_map(|m| m["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::Role;

    #[test]
    fn base_url_validation_requires_scheme_and_host() {
        for ok in [
            "https://api.groq.com/openai/v1",
            "http://localhost:1234/v1",
            "HTTPS://example.com",
            "  https://example.com/v1/  ",
        ] {
            assert!(is_valid_base_url(ok), "{ok:?}");
        }
        for bad in [
            "",
            "api.groq.com/openai/v1",
            "ftp://example.com",
            "https://",
            "http:///v1",
            "localhost:11434",
        ] {
            assert!(!is_valid_base_url(bad), "{bad:?}");
        }
    }

    #[test]
    fn base_url_is_normalized_to_a_single_join_point() {
        let p = CompatProvider::new(None, "m".into(), Some("https://x.dev/v1///".into()));
        assert_eq!(p.url("models").unwrap(), "https://x.dev/v1/models");
        assert_eq!(
            p.url("chat/completions").unwrap(),
            "https://x.dev/v1/chat/completions"
        );
    }

    #[test]
    fn missing_base_url_is_a_no_endpoint_error_not_a_request() {
        let p = CompatProvider::new(Some("k".into()), "m".into(), None);
        assert!(matches!(p.url("models"), Err(LlmError::NoEndpoint)));
    }

    #[tokio::test]
    async fn no_endpoint_fails_before_any_network_io() {
        // A compat provider with no base URL must report the config
        // problem, NOT a network error — the UI shows this verbatim.
        let p = CompatProvider::new(None, "m".into(), Some("   ".into()));
        let err = p.validate().await.unwrap_err();
        assert!(matches!(err, LlmError::NoEndpoint), "{err:?}");
        let mut on_token = |t: &str| panic!("emitted {t:?} without an endpoint");
        let err = p
            .stream_chat(&[ChatMessage::text(Role::User, "hi")], &mut on_token)
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::NoEndpoint), "{err:?}");
    }

    #[test]
    fn body_reuses_the_openai_wire_format() {
        let body = request_body("llama-3.3-70b", &[ChatMessage::text(Role::User, "hi")]);
        assert_eq!(body["model"], "llama-3.3-70b");
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn debug_never_leaks_api_key() {
        let p = CompatProvider::new(
            Some("sk-SENTINEL".into()),
            "m".into(),
            Some("https://x.dev/v1".into()),
        );
        assert!(!format!("{p:?}").contains("sk-SENTINEL"));
    }

    #[test]
    fn openrouter_uses_the_fixed_endpoint_and_requires_a_key() {
        let p = CompatProvider::openrouter(Some("sk-or-x".into()), "m".into());
        assert_eq!(
            p.url("chat/completions").unwrap(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            p.url("models").unwrap(),
            "https://openrouter.ai/api/v1/models"
        );
        assert!(p.preflight().is_ok());

        // Missing or empty key on a require_key provider is an auth
        // failure, decided before any network I/O.
        for key in [None, Some(String::new()), Some("   ".into())] {
            // "   " still counts as a key (auth header sent verbatim);
            // only absent/empty short-circuit.
            let expect_auth = key.as_deref().is_none_or(str::is_empty);
            let p = CompatProvider::openrouter(key, "m".into());
            assert_eq!(matches!(p.preflight(), Err(LlmError::Auth)), expect_auth);
        }
    }

    #[tokio::test]
    async fn openrouter_fails_auth_before_any_network_io() {
        // No server, no DNS — a keyless OpenRouter provider must report
        // `Auth` immediately from both entry points.
        let p = CompatProvider::openrouter(None, "m".into());
        assert!(matches!(p.validate().await, Err(LlmError::Auth)));
        let mut on_token = |t: &str| panic!("emitted {t:?} without a key");
        assert!(matches!(
            p.stream_chat(&[ChatMessage::text(Role::User, "hi")], &mut on_token)
                .await,
            Err(LlmError::Auth)
        ));
    }

    #[test]
    fn openrouter_requests_carry_x_title_and_bearer() {
        let p = CompatProvider::openrouter(Some("sk-or-x".into()), "m".into());
        let req = p
            .authed(p.client.get(p.url("models").unwrap()))
            .build()
            .unwrap();
        assert_eq!(req.headers()["x-title"], "Marvis");
        assert_eq!(req.headers()["authorization"], "Bearer sk-or-x");

        // The generic compatible adapter sends no attribution header.
        let p = CompatProvider::new(None, "m".into(), Some("https://x.dev/v1".into()));
        let req = p
            .authed(p.client.get(p.url("models").unwrap()))
            .build()
            .unwrap();
        assert!(!req.headers().contains_key("x-title"));
        assert!(!req.headers().contains_key("authorization"));
    }
}
