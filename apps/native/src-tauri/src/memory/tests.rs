use super::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::llm::{ChatMessage, ContentPart, LlmError, Provider, Role, StreamReply};
use crate::storage::Memory;

fn assert_request_contains(message: &ChatMessage, expected: &str) {
    let text = message
        .content
        .iter()
        .find_map(|part| match part {
            ContentPart::Text(text) => Some(text.as_str()),
            ContentPart::ImageJpeg(_) => None,
        })
        .unwrap();
    assert!(text.contains(expected), "missing {expected:?} in {text:?}");
}

#[test]
fn parser_accepts_explicit_and_inferred_identity_facts() {
    let facts = parse_response(
        r#"{
          "facts": [
            {"category":"identity","attribute":"name","value":"The user's name is Allen.","confidence":0.98,"basis":"explicit"},
            {"category":"preference","attribute":"response_style","value":"The user prefers concise answers.","confidence":0.86,"basis":"inferred"}
          ]
        }"#,
    )
    .unwrap();

    assert_eq!(facts.len(), 2);
    assert_eq!(facts[0].attribute, "name");
    assert_eq!(facts[1].basis, "inferred");
}

#[test]
fn parser_rejects_invalid_categories_attributes_confidence_and_secrets() {
    for json in [
        r#"{"facts":[{"category":"project","attribute":"name","value":"x","confidence":1.0,"basis":"explicit"}]}"#,
        r#"{"facts":[{"category":"identity","attribute":"bad-key","value":"x","confidence":1.0,"basis":"explicit"}]}"#,
        r#"{"facts":[{"category":"identity","attribute":"name","value":"x","confidence":1.2,"basis":"explicit"}]}"#,
        r#"{"facts":[{"category":"identity","attribute":"name","value":"my API key is sk-test","confidence":1.0,"basis":"explicit"}]}"#,
        r#"not json"#,
    ] {
        assert!(parse_response(json).is_err(), "accepted {json}");
    }
}

#[test]
fn parser_allows_ordinary_secret_words_but_rejects_credential_phrases() {
    for (value, allowed) in [
        ("The user works as a secretary.", true),
        ("The user studies secretions.", true),
        ("The user enjoys Secret Santa.", true),
        ("The user likes secret gardens.", true),
        ("The user's password is example.", false),
        ("The user's API key is example.", false),
        ("The user's access token is example.", false),
        ("The user's private key is example.", false),
        ("The user's CLIENT SECRET is example.", false),
        ("The user's secret key is example.", false),
        ("The user's secret token is example.", false),
        ("Secret: example", false),
        ("secret=example", false),
    ] {
        let response = serde_json::json!({"facts": [{
            "category": "identity", "attribute": "detail", "value": value,
            "confidence": 1.0, "basis": "explicit"
        }]});
        assert_eq!(
            parse_response(&response.to_string()).is_ok(),
            allowed,
            "{value}"
        );
    }
}

/// Small models compress "nothing to store" to a bare `[]` (and some
/// emit the fact array without the `{"facts":…}` wrapper at all). The
/// container shape isn't part of the safety contract — every element
/// still gets the full per-fact validation — so a bare array parses.
#[test]
fn parser_tolerates_a_bare_fact_array_from_small_models() {
    assert_eq!(parse_response("[]").unwrap().len(), 0);
    let facts = parse_response(
        r#"[{"category":"identity","attribute":"name","value":"The user's name is Allen.","confidence":0.9,"basis":"explicit"}]"#,
    )
    .unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].attribute, "name");
    // …but a bare array does NOT relax per-fact validation.
    assert!(
        parse_response(
            r#"[{"category":"project","attribute":"name","value":"x","confidence":1.0,"basis":"explicit"}]"#,
        )
        .is_err()
    );
}

/// `memories` is UNIQUE on `(category, attribute)`, so the dedup key
/// must be that pair — not the triple. Two values for one key in a
/// single response can't both apply: first wins, the rest drop.
#[test]
fn parser_dedups_on_category_attribute_not_the_value() {
    let facts = parse_response(
        r#"{"facts":[
          {"category":"identity","attribute":"name","value":"Allen","confidence":0.9,"basis":"explicit"},
          {"category":"identity","attribute":"name","value":"Al","confidence":0.9,"basis":"explicit"},
          {"category":"preference","attribute":"name","value":"Al","confidence":0.9,"basis":"explicit"}
        ]}"#,
    )
    .unwrap();
    assert_eq!(facts.len(), 2);
    assert_eq!(facts[0].value, "Allen");
    assert_eq!(facts[1].category, "preference");
}

#[test]
fn parser_caps_facts_and_profile_is_bounded_untrusted_data() {
    let facts = (0..10)
        .map(|i| {
            format!(
                r#"{{"category":"preference","attribute":"style_{i}","value":"value {i}","confidence":0.8,"basis":"explicit"}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let parsed = parse_response(&format!(r#"{{"facts":[{facts}]}}"#)).unwrap();
    assert_eq!(parsed.len(), 8);

    let rows = parsed
        .into_iter()
        .enumerate()
        .map(|(id, fact)| Memory {
            id: id as i64,
            category: fact.category,
            attribute: fact.attribute,
            value: fact.value,
            confidence: fact.confidence,
            basis: fact.basis,
            source: "automatic".into(),
            source_session_id: None,
            source_message_id: None,
            created_at: 1,
            updated_at: 1,
        })
        .collect::<Vec<_>>();
    let profile = profile_prompt(&rows).unwrap();
    assert!(profile.starts_with("<user_profile>"));
    assert!(profile.ends_with("</user_profile>"));
    assert!(profile.contains("untrusted") == false);
    assert!(profile.len() <= 4_000);
}

#[test]
fn extraction_messages_keep_source_text_separate_from_profile() {
    let existing = vec![Memory {
        id: 1,
        category: "preference".into(),
        attribute: "response_style".into(),
        value: "The user prefers concise answers.".into(),
        confidence: 0.8,
        basis: "inferred".into(),
        source: "automatic".into(),
        source_session_id: None,
        source_message_id: None,
        created_at: 1,
        updated_at: 1,
    }];
    let messages = extraction_messages(&existing, "My name is Allen.", &[]);
    assert_eq!(messages.len(), 2);
    assert!(matches!(messages[0].role, Role::System));
    assert!(matches!(messages[1].role, Role::User));
    assert_request_contains(&messages[1], "My name is Allen.");
    assert_request_contains(&messages[1], "The user prefers concise answers.");
}

/// Prior turns render as `role: content` lines inside
/// `<recent_messages>` — truncated per message — and the block is
/// omitted entirely on a first-ever send.
#[test]
fn extraction_messages_render_the_tail_as_context_only() {
    let tail = vec![
        ("user".to_string(), "call me Al".to_string()),
        ("assistant".to_string(), "x".repeat(400)),
    ];
    let messages = extraction_messages(&[], "It's spelled Ailin.", &tail);
    let text = match &messages[1].content[0] {
        ContentPart::Text(t) => t.as_str(),
        ContentPart::ImageJpeg(_) => panic!("user prompt must be text"),
    };
    assert!(text.contains("<recent_messages>\nuser: call me Al\nassistant: "));
    // The tail is capped per message — a 400-char reply lands as 300.
    assert!(text.contains(&"x".repeat(300)));
    assert!(!text.contains(&"x".repeat(301)));
    // …and no tail at all → no block (an empty context tag would read
    // as meaningful context to the model).
    let messages = extraction_messages(&[], "hi", &[]);
    let text = match &messages[1].content[0] {
        ContentPart::Text(t) => t.as_str(),
        _ => panic!(),
    };
    assert!(!text.contains("recent_messages"));
}

/// The extractor gets a `YYYY-MM-DD` observation date to ground
/// relative references ("last week"), and the contract pins the
/// fact's language to the user's own — no silent translation to
/// English.
#[test]
fn extraction_messages_carry_observation_date_and_language_rule() {
    let messages = extraction_messages(&[], "anything", &[]);
    assert_request_contains(&messages[0], "same language and script");
    let text = match &messages[1].content[0] {
        ContentPart::Text(t) => t.as_str(),
        ContentPart::ImageJpeg(_) => panic!("user prompt must be text"),
    };
    let date = text
        .split("<observation_date>\n")
        .nth(1)
        .and_then(|rest| rest.split('\n').next())
        .expect("observation_date block");
    assert_eq!(date.len(), 10, "not YYYY-MM-DD: {date}");
    assert!(date.chars().all(|c| c.is_ascii_digit() || c == '-'));
    assert_eq!(date, super::today_utc());
}

/// A stub `Provider`: `reply` returns the canned body, `error` fails.
struct ScriptedProvider {
    reply: Option<String>,
    calls: AtomicUsize,
}

impl ScriptedProvider {
    fn reply(text: &str) -> Self {
        Self {
            reply: Some(text.to_string()),
            calls: AtomicUsize::new(0),
        }
    }

    fn error() -> Self {
        Self {
            reply: None,
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Provider for ScriptedProvider {
    fn stream_chat<'a>(
        &'a self,
        _msgs: &'a [ChatMessage],
        _on_token: &'a mut (dyn FnMut(&str) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<StreamReply, LlmError>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            match &self.reply {
                Some(full) => Ok(StreamReply {
                    full: full.clone(),
                    usage: None,
                }),
                None => Err(LlmError::Http {
                    status: 500,
                    message: "boom".into(),
                }),
            }
        })
    }

    fn validate<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), LlmError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn extraction_stores_valid_facts_and_provider_failure_keeps_db_unchanged() {
    let dir = std::env::temp_dir().join(format!("marvis-memory-test-{}", std::process::id()));
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let service = MemoryService::new();
    let provider = ScriptedProvider::reply(
        r#"{"facts":[{"category":"identity","attribute":"name","value":"The user's name is Allen.","confidence":0.98,"basis":"explicit"}]}"#,
    );

    assert_eq!(
        service
            .extract_once(&provider, &db, "My name is Allen.", None, None, &|| true)
            .await
            .unwrap(),
        1
    );
    assert_eq!(db.memory_profile().unwrap()[0].attribute, "name");

    let failing = ScriptedProvider::error();
    assert!(service
        .extract_once(&failing, &db, "I prefer bullets.", None, None, &|| true)
        .await
        .is_err());
    assert_eq!(db.memory_profile().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// A `false` consent read inside the gate drops the job before the
/// provider sees a byte — `Ok(0)` (nothing changed, no event) and no
/// fact lands. This is the mid-flight-disable guard: `prepare_hook`'s
/// snapshot `enabled` check can't cover a toggle that lands while the
/// ask streams or the extraction queues on `gate`.
#[tokio::test]
async fn extraction_rechecks_consent_inside_the_gate() {
    let dir = std::env::temp_dir().join(format!("marvis-memory-consent-{}", std::process::id()));
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let service = MemoryService::new();
    let provider = ScriptedProvider::reply(
        r#"{"facts":[{"category":"identity","attribute":"name","value":"x","confidence":0.9,"basis":"explicit"}]}"#,
    );

    assert_eq!(
        service
            .extract_once(&provider, &db, "My name is Allen.", None, None, &|| false)
            .await
            .unwrap(),
        0
    );
    assert_eq!(provider.calls(), 0, "provider saw text after disable");
    assert!(db.memory_profile().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn prepare_hook_uses_memory_selection_and_not_the_ask_failover_order() {
    let dir = std::env::temp_dir().join(format!("marvis-memory-config-{}", std::process::id()));
    let db = Arc::new(Db::at(dir.join("marvis.db")).unwrap());
    let mut config = Config::default();
    config.memory.enabled = true;
    config.memory.provider = "ollama".into();
    config.memory.model = "qwen3:8b".into();
    config.providers.disabled = vec!["ollama".into()];
    config.providers.order = vec!["openai".into()];
    let keystore = Keystore::at(dir.join("keys.json"));

    // Disabled in the chain and absent from `order` — yet the hook must
    // still resolve, because the memory pick is independent.
    let hook = prepare_hook(
        &config,
        &keystore,
        db,
        MemoryService::new(),
        Arc::new(|| {}),
        Arc::new(|| true),
    );
    assert!(hook.is_some());
    let _ = std::fs::remove_dir_all(dir);
}
