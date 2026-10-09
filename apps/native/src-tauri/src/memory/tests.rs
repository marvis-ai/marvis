use super::*;
use std::future::Future;
use std::pin::Pin;

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
    let messages = extraction_messages(&existing, "My name is Allen.");
    assert_eq!(messages.len(), 2);
    assert!(matches!(messages[0].role, Role::System));
    assert!(matches!(messages[1].role, Role::User));
    assert_request_contains(&messages[1], "My name is Allen.");
    assert_request_contains(&messages[1], "The user prefers concise answers.");
}

/// A stub `Provider`: `reply` returns the canned body, `error` fails.
struct ScriptedProvider {
    reply: Option<String>,
}

impl ScriptedProvider {
    fn reply(text: &str) -> Self {
        Self {
            reply: Some(text.to_string()),
        }
    }

    fn error() -> Self {
        Self { reply: None }
    }
}

impl Provider for ScriptedProvider {
    fn stream_chat<'a>(
        &'a self,
        _msgs: &'a [ChatMessage],
        _on_token: &'a mut (dyn FnMut(&str) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<StreamReply, LlmError>> + Send + 'a>> {
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
            .extract_once(&provider, &db, "My name is Allen.", None, None)
            .await
            .unwrap(),
        1
    );
    assert_eq!(db.memory_profile().unwrap()[0].attribute, "name");

    let failing = ScriptedProvider::error();
    assert!(service
        .extract_once(&failing, &db, "I prefer bullets.", None, None)
        .await
        .is_err());
    assert_eq!(db.memory_profile().unwrap().len(), 1);
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
    );
    assert!(hook.is_some());
    let _ = std::fs::remove_dir_all(dir);
}
