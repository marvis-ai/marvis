//! presets.rs — named per-send prompt presets (ROADMAP Phase 1 → the
//! seed of Phase 3 skill bundles). Built-ins ship in the catalog below
//! (`b:` ids); user presets persist as `[[prompts.custom]]` in
//! `config.toml` (`u:` ids). `instruct` presets append their text to a
//! send's system prompt; `template` presets expand `{input}`/`{lang}`
//! in the composer and never reach `ask_send` armed.

use serde::{Deserialize, Serialize};

/// `instruct` presets append `text` to the ask's system prompt for
/// that send only; `template` presets are composer-side text
/// expansions (`{input}`/`{lang}`) — they never ride `ask_send`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresetKind {
    #[default]
    Instruct,
    Template,
}

/// One preset — built-in (`b:` id) or user-defined (`u:` id).
/// `#[serde(default)]` keeps a hand-edited `[[prompts.custom]]` row
/// with a missing field from failing the whole `Config` load —
/// `validate`/`normalize` then drop the malformed row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub kind: PresetKind,
    pub text: String,
}

/// The shipped catalog — `(id, name, kind, text)` rows. `{input}` =
/// the composer's text at pick time; `{lang}` = the configured main
/// language's English name (both expand webview-side).
const BUILTIN_ROWS: &[(&str, &str, PresetKind, &str)] = &[
    (
        "b:concise",
        "Concise",
        PresetKind::Instruct,
        "Answer briefly — a short paragraph or a tight list.",
    ),
    (
        "b:explain",
        "Explain",
        PresetKind::Instruct,
        "Explain for a newcomer — define terms, avoid jargon.",
    ),
    (
        "b:advocate",
        "Devil's advocate",
        PresetKind::Instruct,
        "Challenge this: strongest counterarguments first, then a verdict.",
    ),
    (
        "b:translate",
        "Translate",
        PresetKind::Template,
        "Translate the following into {lang}:\n\n{input}",
    ),
    (
        "b:reply",
        "Reply",
        PresetKind::Template,
        "Draft a reply to this message — match its tone:\n\n{input}",
    ),
    (
        "b:summarize",
        "Summarize",
        PresetKind::Template,
        "Summarize the following in 3–5 bullets:\n\n{input}",
    ),
];

/// The built-in catalog as owned `Preset`s.
pub fn builtins() -> Vec<Preset> {
    BUILTIN_ROWS
        .iter()
        .map(|(id, name, kind, text)| Preset {
            id: id.to_string(),
            name: name.to_string(),
            kind: *kind,
            text: text.to_string(),
        })
        .collect()
}

/// Built-ins then customs — the merged order the picker menu, the
/// `/`-shorthand matcher, and `ask_send`'s id resolution all share.
pub fn all(custom: &[Preset]) -> Vec<Preset> {
    builtins()
        .into_iter()
        .chain(custom.iter().cloned())
        .collect()
}

/// First preset with `id` — any kind. `preset.*` menu dispatch uses
/// this; the ask path wants `resolve_instruct` instead.
pub fn find(id: &str, custom: &[Preset]) -> Option<Preset> {
    all(custom).into_iter().find(|p| p.id == id)
}

/// `ask_send` resolution: instruct presets only — a template id sent
/// armed is a crafted-invoke edge (expansion lives in the composer).
pub fn resolve_instruct(id: &str, custom: &[Preset]) -> Option<Preset> {
    find(id, custom).filter(|p| p.kind == PresetKind::Instruct)
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

    fn custom(id: &str, name: &str, kind: PresetKind) -> Preset {
        Preset {
            id: id.to_string(),
            name: name.to_string(),
            kind,
            text: "preset text".to_string(),
        }
    }

    #[test]
    fn builtins_have_unique_ids_and_valid_rows() {
        let list = builtins();
        assert!(list.len() >= 5);
        let ids: std::collections::HashSet<_> = list.iter().map(|p| &p.id).collect();
        assert_eq!(ids.len(), list.len());
        for p in &list {
            assert!(p.id.starts_with("b:"));
            assert!(!p.name.is_empty() && !p.text.is_empty());
        }
        assert!(list.iter().any(|p| p.kind == PresetKind::Instruct));
        assert!(list.iter().any(|p| p.kind == PresetKind::Template));
    }

    #[test]
    fn find_searches_builtins_then_customs() {
        let customs = vec![custom("u:1", "Mine", PresetKind::Instruct)];
        assert_eq!(find("b:concise", &customs).unwrap().name, "Concise");
        assert_eq!(find("u:1", &customs).unwrap().name, "Mine");
        assert!(find("nope", &customs).is_none());
    }

    #[test]
    fn resolve_instruct_skips_templates_and_unknowns() {
        assert!(resolve_instruct("b:concise", &[]).is_some());
        assert!(resolve_instruct("b:translate", &[]).is_none());
        assert!(resolve_instruct("b:missing", &[]).is_none());
    }

    #[test]
    fn validate_custom_trims_and_rejects_bad_rows() {
        let ok = validate_custom(vec![Preset {
            id: " u:a1 ".into(),
            name: "  Named ".into(),
            kind: PresetKind::Template,
            text: " body ".into(),
        }])
        .unwrap();
        assert_eq!(ok[0].id, "u:a1");
        assert_eq!(ok[0].name, "Named");

        assert!(validate_custom(vec![custom("", "x", PresetKind::Instruct)]).is_err());
        assert!(validate_custom(vec![custom("b:concise", "x", PresetKind::Instruct)]).is_err());
        assert!(validate_custom(vec![
            custom("u:1", "a", PresetKind::Instruct),
            custom("u:1", "b", PresetKind::Instruct),
        ])
        .is_err());
        let mut empty_text = custom("u:2", "x", PresetKind::Instruct);
        empty_text.text = "  ".into();
        assert!(validate_custom(vec![empty_text]).is_err());

        // Limits: name ≤24 chars, text ≤2000 chars, and a custom id
        // must be a webview-minted `u:` string — `b:` is the catalog's
        // namespace even when the id isn't a shipped built-in.
        assert!(
            validate_custom(vec![custom("u:3", &"n".repeat(25), PresetKind::Instruct)]).is_err()
        );
        let mut long_text = custom("u:4", "x", PresetKind::Instruct);
        long_text.text = "t".repeat(2001);
        assert!(validate_custom(vec![long_text]).is_err());
        assert!(validate_custom(vec![custom("b:zzz", "x", PresetKind::Instruct)]).is_err());
        // The bare `u:` prefix with no suffix names nothing.
        assert!(validate_custom(vec![custom("u:", "x", PresetKind::Instruct)]).is_err());
    }
}
