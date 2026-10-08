//! presets.rs — named per-send prompt presets (ROADMAP Phase 1 → the
//! seed of Phase 3 skill bundles). Built-ins ship in the catalog below
//! (`b:` ids); user presets persist as `[[prompts.custom]]` in
//! `config.toml` (`u:` ids). There is no preset "kind": a text that
//! contains `{input}` expands into the sent message (the composer shows
//! the armed preset + its `{lang}` param as badges); any other text is
//! a silent instruction appended to that send's system prompt.

use serde::{Deserialize, Serialize};

/// One preset — built-in (`b:` id) or user-defined (`u:` id).
/// `#[serde(default)]` keeps a hand-edited `[[prompts.custom]]` row
/// with a missing field from failing the whole `Config` load —
/// `validate`/`normalize` then drop the malformed row. A stale `kind`
/// key from an older schema deserializes harmlessly (unknown fields
/// are ignored).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub text: String,
}

/// `{input}` in the text is the whole contract: with it, the preset
/// expands into the composer's message (`{lang}` → the configured main
/// language, editable per-send); without it, the text is an
/// instruction appended to the send's system prompt.
pub fn is_template(p: &Preset) -> bool {
    p.text.contains("{input}")
}

/// The shipped catalog — `(id, name, text)` rows. `{input}` = the
/// composer's text at send time; `{lang}` = the configured main
/// language's English name.
const BUILTIN_ROWS: &[(&str, &str, &str)] = &[
    (
        "b:concise",
        "Concise",
        "Answer briefly — a short paragraph or a tight list.",
    ),
    (
        "b:explain",
        "Explain",
        "Explain for a newcomer — define terms, avoid jargon.",
    ),
    (
        "b:advocate",
        "Devil's advocate",
        "Challenge this: strongest counterarguments first, then a verdict.",
    ),
    (
        "b:translate",
        "Translate",
        "Translate the following into {lang}:\n\n{input}",
    ),
    (
        "b:reply",
        "Reply",
        "Draft a reply to this message — match its tone:\n\n{input}",
    ),
    (
        "b:summarize",
        "Summarize",
        "Summarize the following in 3–5 bullets:\n\n{input}",
    ),
];

/// The built-in catalog as owned `Preset`s.
pub fn builtins() -> Vec<Preset> {
    BUILTIN_ROWS
        .iter()
        .map(|(id, name, text)| Preset {
            id: id.to_string(),
            name: name.to_string(),
            text: text.to_string(),
        })
        .collect()
}

/// Built-ins then customs — the merged order the palette, the
/// `/`-shorthand matcher, and `ask_send`'s id resolution all share.
pub fn all(custom: &[Preset]) -> Vec<Preset> {
    builtins()
        .into_iter()
        .chain(custom.iter().cloned())
        .collect()
}

/// First preset with `id` — palette selects and `ask_send`'s preset
/// resolution both use this (the ask path then checks `is_template` to
/// decide between instruction text and pure provenance).
pub fn find(id: &str, custom: &[Preset]) -> Option<Preset> {
    all(custom).into_iter().find(|p| p.id == id)
}

/// `ask_send` resolution — alias of `find` kept for readability at the
/// send site: any resolvable id persists on the row for provenance;
/// only a non-template contributes `instruction` text.
pub fn resolve(id: &str, custom: &[Preset]) -> Option<Preset> {
    find(id, custom)
}

/// One preset's shape — trimmed fields checked by the caller first
/// (`validate_custom` trims, `normalize` retains via this same
/// predicate so a hand-edited config keeps its valid rows).
pub fn validate(p: &Preset) -> Result<(), String> {
    if p.id.is_empty()
        || p.id.len() > 40
        || !p
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'))
    {
        return Err(format!("invalid preset id {:?}", p.id));
    }
    if p.name.is_empty() || p.name.chars().count() > 24 {
        return Err(format!("invalid preset name {:?}", p.name));
    }
    if p.text.is_empty() || p.text.chars().count() > 2000 {
        return Err("preset text must be 1–2000 chars".to_string());
    }
    if BUILTIN_ROWS.iter().any(|(bid, ..)| *bid == p.id) {
        return Err(format!("preset id {:?} collides with a built-in", p.id));
    }
    // `validate` only ever sees custom rows (built-ins come from the
    // catalog, never through `validate_custom`/`normalize`), so every
    // id it accepts must be a webview-minted `u:` string — a hand-made
    // `b:`/`x:` id can't squat on the catalog's or any future namespace.
    if !p.id.starts_with("u:") {
        return Err(format!(
            "custom preset id {:?} must start with \"u:\"",
            p.id
        ));
    }
    // The bare prefix names nothing — `u:` must carry a suffix.
    if p.id.len() == 2 {
        return Err(format!("custom preset id {:?} has an empty suffix", p.id));
    }
    Ok(())
}

/// `config_set('prompts.custom')` validation — a write REJECTS on the
/// first bad row (the UI submits the whole array). Returns the list
/// with every field trimmed.
pub fn validate_custom(list: Vec<Preset>) -> Result<Vec<Preset>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(list.len());
    for mut p in list {
        p.id = p.id.trim().to_string();
        p.name = p.name.trim().to_string();
        p.text = p.text.trim().to_string();
        validate(&p)?;
        if !seen.insert(p.id.clone()) {
            return Err(format!("duplicate preset id {:?}", p.id));
        }
        out.push(p);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(id: &str, name: &str) -> Preset {
        Preset {
            id: id.to_string(),
            name: name.to_string(),
            text: "preset text".to_string(),
        }
    }

    #[test]
    fn builtins_have_unique_ids_valid_rows_and_both_behaviors() {
        let list = builtins();
        assert!(list.len() >= 5);
        let ids: std::collections::HashSet<_> = list.iter().map(|p| &p.id).collect();
        assert_eq!(ids.len(), list.len());
        for p in &list {
            assert!(p.id.starts_with("b:"));
            assert!(!p.name.is_empty() && !p.text.is_empty());
        }
        // The catalog keeps both behaviors reachable: plain instructions
        // (no `{input}`) and expanding presets (contain `{input}`).
        assert!(list.iter().any(|p| is_template(p)));
        assert!(list.iter().any(|p| !is_template(p)));
    }

    #[test]
    fn is_template_is_purely_the_input_placeholder() {
        let template = custom("u:t", "T");
        let mut template = template;
        template.text = "Summarize:\n\n{input}".to_string();
        assert!(is_template(&template));
        let mut instruct = custom("u:i", "I");
        instruct.text = "Answer in {lang}.".to_string();
        assert!(!is_template(&instruct)); // `{lang}` alone stays an instruction
        assert!(!is_template(&custom("u:p", "P")));
    }

    #[test]
    fn find_searches_builtins_then_customs() {
        let customs = vec![custom("u:1", "Mine")];
        assert_eq!(find("b:concise", &customs).unwrap().name, "Concise");
        assert_eq!(find("u:1", &customs).unwrap().name, "Mine");
        assert!(find("nope", &customs).is_none());
    }

    #[test]
    fn resolve_hits_any_resolvable_id() {
        assert!(resolve("b:concise", &[]).is_some());
        assert!(resolve("b:translate", &[]).is_some());
        assert!(resolve("b:missing", &[]).is_none());
    }

    #[test]
    fn validate_custom_trims_and_rejects_bad_rows() {
        let ok = validate_custom(vec![Preset {
            id: " u:a1 ".into(),
            name: "  Named ".into(),
            text: " body ".into(),
        }])
        .unwrap();
        assert_eq!(ok[0].id, "u:a1");
        assert_eq!(ok[0].name, "Named");

        assert!(validate_custom(vec![custom("", "x")]).is_err());
        assert!(validate_custom(vec![custom("b:concise", "x")]).is_err());
        assert!(validate_custom(vec![custom("u:1", "a"), custom("u:1", "b"),]).is_err());
        let mut empty_text = custom("u:2", "x");
        empty_text.text = "  ".into();
        assert!(validate_custom(vec![empty_text]).is_err());

        // Limits: name ≤24 chars, text ≤2000 chars, and a custom id
        // must be a webview-minted `u:` string — `b:` is the catalog's
        // namespace even when the id isn't a shipped built-in.
        assert!(validate_custom(vec![custom("u:3", &"n".repeat(25))]).is_err());
        let mut long_text = custom("u:4", "x");
        long_text.text = "t".repeat(2001);
        assert!(validate_custom(vec![long_text]).is_err());
        assert!(validate_custom(vec![custom("b:zzz", "x")]).is_err());
        // The bare `u:` prefix with no suffix names nothing.
        assert!(validate_custom(vec![custom("u:", "x")]).is_err());
    }
}
