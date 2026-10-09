//! Identity/preference memory — the consent-gated extraction pipeline.
//!
//! What lives here:
//!
//! - `parse_response` — STRICT validation of the Memory LLM's JSON
//!   output. The contract is one `{"facts": [...]}` object; malformed
//!   output is discarded, never repaired. Every fact is normalized
//!   (trim/lowercase category+attribute), range-checked, screened for
//!   credential-shaped values, deduplicated, and capped.
//! - `prompt` — the extraction prompt pair and the bounded
//!   `<user_profile>` block `send_chain` injects into Ask system
//!   prompts.
//!
//! The provider plumbing (`MemoryService`, `MemoryHook`,
//! `prepare_hook`) rides the same module — extraction is serialized
//! through a `tokio::sync::Mutex` so overlapping asks can't double-write
//! a fact, and `EV_MEMORY_CHANGED` fires only when rows actually change.
//!
//! Privacy boundary (see `MemoryPrefs`): extraction runs only when
//! `[memory].enabled`, reads only the new Ask user message, and calls
//! only the dedicated memory provider — never the Ask failover chain.

mod prompt;

#[cfg(test)]
mod tests;

pub(crate) use prompt::*;

use std::collections::HashSet;
use std::sync::Arc;

use crate::config::Config;
use crate::keystore::Keystore;
use crate::llm::{make_provider, Provider, ProviderKind};
use crate::storage::{Db, MemoryCandidate};

/// One extraction response contributes at most this many facts.
const MAX_FACTS: usize = 8;
/// `attribute` is a slug: `[a-z0-9_]`, this many chars max.
const MAX_ATTRIBUTE_CHARS: usize = 64;
/// `value` length cap, in Unicode scalar values.
const MAX_VALUE_CHARS: usize = 500;
/// The `<user_profile>` block renders at most this many rows…
const MAX_PROFILE_ROWS: usize = 32;
/// …and never exceeds this many UTF-8 bytes, wrapper included.
const MAX_PROFILE_BYTES: usize = 4_000;

/// The extractor's wire shape — `{"facts":[{category, attribute, value,
/// confidence, basis}]}`. Strict: no `default`, no `deny_unknown_fields`
/// needed (extra keys are ignored, which is fine — the model's
/// contract demands the required ones).
#[derive(Debug, serde::Deserialize)]
struct ExtractionResponse {
    facts: Vec<RawFact>,
}

#[derive(Debug, serde::Deserialize)]
struct RawFact {
    category: String,
    attribute: String,
    value: String,
    confidence: f64,
    basis: String,
}

/// Values that look like secrets never become facts — the extractor
/// shouldn't be storing credentials even if the model emits them.
const FORBIDDEN_VALUE_SUBSTRINGS: &[&str] = &[
    "password",
    "api key",
    "access token",
    "secret",
    "private key",
];

fn normalize_slug(s: &str) -> String {
    s.trim().to_lowercase()
}

fn is_valid_attribute(attribute: &str) -> bool {
    !attribute.is_empty()
        && attribute.chars().count() <= MAX_ATTRIBUTE_CHARS
        && attribute
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Parse the extractor's reply into validated candidates. Any malformed
/// JSON or ANY invalid fact discards the whole response — a model that
/// can't hold the contract isn't trusted to half-hold it. The container
/// is lenient on purpose: the contract asks for `{"facts":[…]}` but
/// small models answer the bare array (`[]` for "nothing to store") —
/// the strictness lives in the per-fact validation below either way.
pub(crate) fn parse_response(text: &str) -> anyhow::Result<Vec<MemoryCandidate>> {
    let value: serde_json::Value = serde_json::from_str(text.trim())
        .map_err(|e| anyhow::anyhow!("memory extraction response is not strict JSON: {e}"))?;
    let raw_facts = if value.is_array() {
        serde_json::from_value::<Vec<RawFact>>(value)
            .map_err(|e| anyhow::anyhow!("memory extraction fact array is malformed: {e}"))?
    } else {
        serde_json::from_value::<ExtractionResponse>(value)
            .map_err(|e| anyhow::anyhow!("memory extraction response is not strict JSON: {e}"))?
            .facts
    };

    let mut seen = HashSet::new();
    let mut facts = Vec::new();
    for raw in raw_facts {
        let category = normalize_slug(&raw.category);
        if !matches!(category.as_str(), "identity" | "preference") {
            anyhow::bail!("unknown memory category {:?}", raw.category);
        }
        let attribute = normalize_slug(&raw.attribute);
        if !is_valid_attribute(&attribute) {
            anyhow::bail!("invalid memory attribute {:?}", raw.attribute);
        }
        let value = raw.value.trim();
        if value.is_empty() || value.chars().count() > MAX_VALUE_CHARS {
            anyhow::bail!("memory value must be 1..={MAX_VALUE_CHARS} characters");
        }
        let lower = value.to_lowercase();
        if let Some(hit) = FORBIDDEN_VALUE_SUBSTRINGS
            .iter()
            .find(|needle| lower.contains(*needle))
        {
            anyhow::bail!("memory value looks like a credential ({hit})");
        }
        if !raw.confidence.is_finite() || !(0.0..=1.0).contains(&raw.confidence) {
            anyhow::bail!("memory confidence must be finite and within 0..=1");
        }
        let basis = normalize_slug(&raw.basis);
        if !matches!(basis.as_str(), "explicit" | "inferred") {
            anyhow::bail!("unknown memory basis {:?}", raw.basis);
        }
        if seen.insert((category.clone(), attribute.clone(), value.to_string())) {
            facts.push(MemoryCandidate {
                category,
                attribute,
                value: value.to_string(),
                confidence: raw.confidence,
                basis,
            });
        }
    }
    facts.truncate(MAX_FACTS);
    Ok(facts)
}

/// Serialized extraction. `gate` is a `tokio::sync::Mutex` — the guard
/// is deliberately held across the provider `.await` so overlapping
/// successful asks can't interleave a read-modify-write on the same
/// facts (and a burst of replies can't fan out duplicate provider calls).
pub(crate) struct MemoryService {
    gate: tokio::sync::Mutex<()>,
}

impl MemoryService {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            gate: tokio::sync::Mutex::new(()),
        })
    }

    /// One extraction pass over `source_text` (the new Ask user message
    /// — never anything else). Loads the current profile for the
    /// prompt, calls the dedicated provider, strictly parses, and
    /// upserts. Returns the number of rows changed; `0` for a clean
    /// empty `facts` list (no db write, no event).
    pub(crate) async fn extract_once(
        &self,
        provider: &dyn Provider,
        db: &Db,
        source_text: &str,
        session_id: Option<i64>,
        message_id: Option<i64>,
    ) -> anyhow::Result<usize> {
        let _permit = self.gate.lock().await;
        let existing = db.memory_profile()?;
        let messages = extraction_messages(&existing, source_text);
        let mut on_token = |_token: &str| {};
        let reply = provider.stream_chat(&messages, &mut on_token).await?;
        // A malformed reply is unactionable without seeing it — carry a
        // bounded snippet in the (local) log line so the failure mode is
        // debuggable instead of a bare serde error.
        let facts = parse_response(&reply.full).map_err(|e| {
            let snippet: String = reply.full.chars().take(120).collect();
            anyhow::anyhow!("{e}; reply: {snippet:?}")
        })?;
        Ok(db.memory_apply(session_id, message_id, &facts)?)
    }
}

/// The scheduled extraction job handed to `send_chain` — fully owned
/// (Arc'd service/db, boxed provider, owned callback) so nothing
/// borrowed from `AppState`/config crosses the spawn boundary.
pub(crate) struct MemoryHook {
    service: Arc<MemoryService>,
    db: Arc<Db>,
    provider: Box<dyn Provider>,
    changed: Arc<dyn Fn() + Send + Sync>,
}

impl MemoryHook {
    pub(crate) fn new(
        service: Arc<MemoryService>,
        db: Arc<Db>,
        provider: Box<dyn Provider>,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            service,
            db,
            provider,
            changed,
        }
    }

    /// Fire-and-forget: extraction runs off the ask path, failures only
    /// log (never surfaced to the ask UI), and `changed` fires only when
    /// rows actually changed.
    pub(crate) fn schedule(
        self,
        source_text: String,
        session_id: Option<i64>,
        message_id: Option<i64>,
    ) {
        let Self {
            service,
            db,
            provider,
            changed,
        } = self;
        tauri::async_runtime::spawn(async move {
            match service
                .extract_once(
                    provider.as_ref(),
                    db.as_ref(),
                    &source_text,
                    session_id,
                    message_id,
                )
                .await
            {
                Ok(changed_rows) if changed_rows > 0 => changed(),
                Ok(_) => {}
                Err(error) => log::warn!("memory extraction skipped: {error}"),
            }
        });
    }
}

/// Resolve the configured Memory LLM into a runnable hook. `None` unless
/// `[memory].enabled` AND the pick is usable — a known provider, a key
/// where the provider requires one, `compat.base_url` for `compatible`,
/// and a non-empty model. `providers.order`/`disabled` are NOT consulted:
/// this selection is independent of the Ask failover chain by design.
pub(crate) fn prepare_hook(
    config: &Config,
    keystore: &Keystore,
    db: Arc<Db>,
    service: Arc<MemoryService>,
    changed: Arc<dyn Fn() + Send + Sync>,
) -> Option<MemoryHook> {
    if !config.memory.enabled {
        return None;
    }
    let kind = ProviderKind::from_str(&config.memory.provider)?;
    let api_key = keystore.key(kind.as_str());
    if api_key.is_none() && !kind.key_optional() {
        return None; // no key — can't extract
    }
    let base_url = Some(config.compat.base_url.clone()).filter(|u| !u.is_empty());
    if kind == ProviderKind::Compatible && base_url.is_none() {
        return None; // endpoint never configured
    }
    let model = config.memory.model.trim();
    if model.is_empty() {
        return None; // no model picked
    }
    let provider = make_provider(kind, api_key, model.to_string(), base_url);
    Some(MemoryHook::new(service, db, provider, changed))
}
