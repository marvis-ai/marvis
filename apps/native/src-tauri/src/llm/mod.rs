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
//! The four adapters under this module are stubs: they hold credentials and
//! a shared `reqwest::Client` (120s timeout) but every call returns
//! [`LlmError::NoModel`]. Real HTTP/SSE streaming lands in Task 6.

mod anthropic;
mod gemini;
mod ollama;
mod openai;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Request timeout shared by every adapter's client — generous because a
/// slow local Ollama model may take a while to produce its first token.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

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
    async fn every_stub_provider_returns_no_model_through_dyn() {
        // Calls go through `Box<dyn Provider>` — proves the trait is
        // object-safe and the factory dispatches every kind.
        for kind in [
            ProviderKind::OpenAi,
            ProviderKind::Anthropic,
            ProviderKind::Gemini,
            ProviderKind::Ollama,
        ] {
            let provider: Box<dyn Provider> = make_provider(kind, None, "some-model".into());
            let mut on_token = |tok: &str| panic!("stub emitted token {tok:?}");
            let err = provider
                .stream_chat(&[ChatMessage::text(Role::User, "hi")], &mut on_token)
                .await
                .unwrap_err();
            assert!(matches!(err, LlmError::NoModel), "{kind:?}");
            let err = provider.validate().await.unwrap_err();
            assert!(matches!(err, LlmError::NoModel), "{kind:?}");
        }
    }
}
