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

use crate::llm::{ChatMessage, ContentPart, Role};
use crate::storage::{Memory, MemoryCandidate};

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
/// can't hold the contract isn't trusted to half-hold it.
pub(crate) fn parse_response(text: &str) -> anyhow::Result<Vec<MemoryCandidate>> {
    let response: ExtractionResponse = serde_json::from_str(text.trim())
        .map_err(|e| anyhow::anyhow!("memory extraction response is not strict JSON: {e}"))?;

    let mut seen = HashSet::new();
    let mut facts = Vec::new();
    for raw in response.facts {
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
