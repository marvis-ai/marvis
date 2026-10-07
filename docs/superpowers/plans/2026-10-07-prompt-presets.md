# Prompt Presets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-send prompt presets for Ask — built-in + user-defined —
applied via a native preset menu (wand button / input-row right-click)
or a `/name` shorthand, per spec
`docs/superpowers/specs/2026-10-07-prompt-presets-design.md`.

**Architecture:** `instruct` presets arm a composer chip and append
their text to the send's system prompt (id persisted on the user
message row so `ask_retry` re-applies); `template` presets expand
`{input}`/`{lang}` into the composer for editing (no hidden semantics).
Presets live in a Rust catalog (`b:` ids) + `config.toml`
`[[prompts.custom]]` (`u:` ids). The picker is a native `popup_menu`
(the pill is a fixed 64px window — no webview popover fits).

**Tech Stack:** Rust/Tauri 2 (`tauri::menu`), rusqlite, TOML config;
React/TS webview (`bun`), lucide icons via `@marvis/ui` barrel.

## Global Constraints

- `bun` for all JS/package commands; `cargo` in `apps/native/src-tauri`.
- IPC: typed wrappers in `src/lib/commands.ts`, event names as `EV_*`
  in `src/lib/events.ts` AND `const EV_*: &str` in Rust — never raw
  `invoke`/`listen` in components; subscribe via `useTauriEvent` only.
- Rust command params snake_case / JS args camelCase; commands return
  `Result<T, String>`; `parking_lot`/`std` guards never held across
  `.await`, `run_on_main_thread`, or `set_menu`.
- Webview imports: `@/` alias cross-directory, `./` same-dir; arrow
  functions + named exports; icons only `Icon`-suffixed lucide names
  through the `@marvis/ui` barrel (`packages/ui/src/index.ts`).
- Markdown tables use `| --- | --- |` (space after `|`).
- Verification: `cargo test` + `cargo clippy` in `src-tauri`;
  `bun test` + `bun run check-types` + `bun run build` in `apps/native`.

---

### Task 1: `presets.rs` — model, catalog, resolve, validate

**Files:**

- Create: `apps/native/src-tauri/src/presets.rs`
- Modify: `apps/native/src-tauri/src/lib.rs` (module list)

**Interfaces:**

- Produces: `Preset { id: String, name: String, kind: PresetKind, text: String }`, `PresetKind::{Instruct, Template}`, `builtins() -> Vec<Preset>`, `all(custom: &[Preset]) -> Vec<Preset>`, `find(id, custom) -> Option<Preset>`, `resolve_instruct(id, custom) -> Option<Preset>`, `validate(&Preset) -> Result<(), String>`, `validate_custom(Vec<Preset>) -> Result<Vec<Preset>, String>`. Consumed by Tasks 2, 4, 5.

- [ ] **Step 1: Write the failing test** — append to a new `#[cfg(test)] mod tests` in the file created in step 3, but first create `apps/native/src-tauri/src/presets.rs` with ONLY:

```rust
//! presets.rs — named per-send prompt presets (ROADMAP Phase 1 → the
//! seed of Phase 3 skill bundles). Built-ins ship in the catalog below
//! (`b:` ids); user presets persist as `[[prompts.custom]]` in
//! `config.toml` (`u:` ids). `instruct` presets append their text to a
//! send's system prompt; `template` presets expand `{input}`/`{lang}`
//! in the composer and never reach `ask_send` armed.

use serde::{Deserialize, Serialize};
```

Register `mod presets;` in `lib.rs` next to the other module
declarations (find the `mod`/`pub mod` block near the top — e.g.
beside `mod prompts;`).

- [ ] **Step 2: Verify compile fails / tests missing**

Run: `cd apps/native/src-tauri && cargo check`
Expected: PASS (empty module compiles). The test comes next.

- [ ] **Step 3: Implement the module + its tests**

Full file contents (append everything below to the stub):

```rust
/// `instruct` presets append `text` to the ask's system prompt for
/// that send only; `template` presets are composer-side text
/// expansions (`{input}`/`{lang}`) — they never ride `ask_send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresetKind {
    Instruct,
    Template,
}

impl Default for PresetKind {
    fn default() -> Self {
        Self::Instruct
    }
}

/// One preset — built-in (`b:` id) or user-defined (`u:` id).
/// `#[serde(default)]` keeps a hand-edited `[[prompts.custom]]` row
/// with a missing field from failing the whole `Config` load —
/// `validate`/`normalize` then drop the malformed row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub kind: PresetKind,
    pub text: String,
}

impl Default for Preset {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: PresetKind::Instruct,
            text: String::new(),
        }
    }
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
        assert!(
            validate_custom(vec![
                custom("u:1", "a", PresetKind::Instruct),
                custom("u:1", "b", PresetKind::Instruct),
            ])
            .is_err()
        );
        let mut empty_text = custom("u:2", "x", PresetKind::Instruct);
        empty_text.text = "  ".into();
        assert!(validate_custom(vec![empty_text]).is_err());
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd apps/native/src-tauri && cargo test presets::`
Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add apps/native/src-tauri/src/presets.rs apps/native/src-tauri/src/lib.rs
git commit -m "Add presets module: model, built-in catalog, resolve + validation"
```

---

### Task 2: `config.toml` `[prompts]` section + `prompts.custom` write key

**Files:**

- Modify: `apps/native/src-tauri/src/config.rs`
- Modify: `apps/native/src-tauri/src/lib.rs` (`config_set` arm)

**Interfaces:**

- Consumes: `crate::presets::{Preset, validate, validate_custom}` (Task 1)
- Produces: `Config.prompts: PromptPrefs { custom: Vec<Preset> }`; writable key `prompts.custom` (JSON array → validated `Vec<Preset>`); used by Tasks 4–5, 10.

- [ ] **Step 1: Write the failing test** — in `config.rs`'s `#[cfg(test)] mod tests` (file already has one — find it), add:

```rust
#[test]
fn prompts_custom_roundtrips_and_normalizes() {
    // Round-trip: a custom preset survives save/load.
    let dir = tmp_dir();
    let path = dir.join("config.toml");
    let mut cfg = Config::default();
    cfg.prompts.custom = vec![crate::presets::Preset {
        id: "u:test".into(),
        name: "Test".into(),
        kind: crate::presets::PresetKind::Instruct,
        text: "Be terse.".into(),
    }];
    cfg.save_to(&path).unwrap();
    let loaded = Config::load_from(&path).unwrap();
    assert_eq!(loaded.prompts.custom.len(), 1);
    assert_eq!(loaded.prompts.custom[0].id, "u:test");

    // A hand-edited file keeps valid rows and drops malformed ones.
    std::fs::write(
        &path,
        "[[prompts.custom]]\n\
         id = \"u:ok\"\nname = \"Ok\"\nkind = \"instruct\"\ntext = \"t\"\n\
         [[prompts.custom]]\n\
         id = \"\"\nname = \"bad\"\nkind = \"instruct\"\ntext = \"t\"\n\
         [[prompts.custom]]\n\
         id = \"u:ok\"\nname = \"dup\"\nkind = \"instruct\"\ntext = \"t\"\n",
    )
    .unwrap();
    let loaded = Config::load_from(&path).unwrap();
    assert_eq!(loaded.prompts.custom.len(), 1);
    assert_eq!(loaded.prompts.custom[0].id, "u:ok");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prompts_custom_write_validates() {
    let mut prompts = PromptPrefs::default();
    let good = serde_json::json!([{
        "id": "u:a1", "name": "A", "kind": "template", "text": "do {input}"
    }]);
    assert!(apply_prompts_config(&mut prompts, "prompts.custom", good).unwrap());
    assert_eq!(prompts.custom[0].kind, crate::presets::PresetKind::Template);
    assert!(!apply_prompts_config(&mut prompts, "other.key", serde_json::json!([])).unwrap());

    let bad = serde_json::json!([{ "id": "u:a1", "name": "", "kind": "instruct", "text": "x" }]);
    assert!(apply_prompts_config(&mut prompts, "prompts.custom", bad).is_err());
}
```

(Needs `use` of nothing extra — same module. If `tmp_dir` doesn't
exist in config.rs tests, mirror storage.rs's
`std::env::temp_dir().join(format!("marvis-cfg-test-{}-{}", pid, n))`
pattern.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/native/src-tauri && cargo test prompts_custom`
Expected: FAIL — `PromptPrefs`/`apply_prompts_config` don't exist.

- [ ] **Step 3: Implement**

In `config.rs`, after `VisionPrefs` (or near `RecordingPrefs`):

```rust
/// `[prompts]` — user-defined prompt presets; built-ins ship in
/// `presets.rs`, never in this file. Custom `id`s are webview-minted
/// `u:` strings; `prompts.custom` writes replace the whole list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PromptPrefs {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom: Vec<crate::presets::Preset>,
}

/// Apply the `prompts.*` config command keys — same handled-shape as
/// [`apply_stt_config`].
pub(crate) fn apply_prompts_config(
    prompts: &mut PromptPrefs,
    key: &str,
    value: &serde_json::Value,
) -> Result<bool, String> {
    match key {
        "prompts.custom" => {
            let raw: Vec<crate::presets::Preset> = serde_json::from_value(value.clone())
                .map_err(|e| format!("prompts.custom must be an array of presets: {e}"))?;
            *prompts = PromptPrefs {
                custom: crate::presets::validate_custom(raw)?,
            };
            Ok(true)
        }
        _ => Ok(false),
    }
}
```

Wire into `Config` — add field after `vision`:

```rust
    /// The dedicated screen reader — see [`VisionPrefs`].
    pub vision: VisionPrefs,
    /// User-defined prompt presets — see [`PromptPrefs`].
    pub prompts: PromptPrefs,
}
```

`Default for Config` gains `prompts: PromptPrefs::default()`.

`normalize()` gains (after the vision block):

```rust
        // A hand-edited `[[prompts.custom]]` keeps its valid rows —
        // malformed rows and duplicate ids drop.
        let mut seen = std::collections::HashSet::new();
        self.prompts.custom.retain_mut(|p| {
            p.id = p.id.trim().to_string();
            p.name = p.name.trim().to_string();
            p.text = p.text.trim().to_string();
            crate::presets::validate(p).is_ok() && seen.insert(p.id.clone())
        });
```

In `lib.rs` `config_set`'s match, add one arm beside the
`apply_recording_config` arm:

```rust
            key if config::apply_prompts_config(&mut cfg.prompts, key, &value)? => {}
```

Also update `config_set`'s doc comment key list to mention
`prompts.custom` (array of `{id, name, kind, text}`).

- [ ] **Step 4: Run tests**

Run: `cd apps/native/src-tauri && cargo test prompts_custom`
Expected: PASS (2 tests) — plus `cargo test config::` still green.

- [ ] **Step 5: Commit**

```bash
git add apps/native/src-tauri/src/config.rs apps/native/src-tauri/src/lib.rs
git commit -m "Config [prompts]: custom preset list + validated prompts.custom write"
```

---

### Task 3: `messages.preset` column

**Files:**

- Modify: `apps/native/src-tauri/src/storage.rs`

**Interfaces:**

- Produces: `Message.preset: Option<String>`, `MessageMeta.preset: Option<String>` — consumed by Task 4 (persist/retry) and Task 9 (`preset` on `session_get` rows).

- [ ] **Step 1: Write the failing test** — append to `storage.rs`'s test module:

```rust
#[test]
fn message_preset_roundtrips_and_migrates() {
    // Fresh schema: the column writes and reads.
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let sid = db.session_get_or_create_active("ask").unwrap();
    db.message_add_meta(
        sid,
        "user",
        "hi",
        &MessageMeta {
            preset: Some("b:concise".to_string()),
            ..MessageMeta::default()
        },
    )
    .unwrap();
    db.message_add(sid, "user", "plain").unwrap();
    let rows = db.messages_for(sid).unwrap();
    assert_eq!(rows[0].preset.as_deref(), Some("b:concise"));
    assert_eq!(rows[1].preset, None);

    // A pre-column database gains `preset` via migrate().
    let dir2 = tmp_dir();
    std::fs::create_dir_all(&dir2).unwrap();
    let path = dir2.join("marvis.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO sessions (type, started_at, last_active_at)
             VALUES ('ask', 1, 1);
             CREATE TABLE messages (
                id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL,
                role TEXT NOT NULL, content TEXT NOT NULL, ts INTEGER NOT NULL);
             INSERT INTO messages (session_id, role, content, ts)
             VALUES (1, 'user', 'old', 1);",
        )
        .unwrap();
    }
    let db = Db::at(&path).unwrap();
    assert_eq!(db.messages_for(1).unwrap()[0].preset, None);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}
```

(Find `V1_SCHEMA` in the test module — it's the legacy-shape constant
used by `migrate_merges_when_messages_table_already_exists`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/native/src-tauri && cargo test message_preset`
Expected: FAIL — `Message.preset`/`MessageMeta.preset` don't exist.

- [ ] **Step 3: Implement**

`SCHEMA` — `messages` table gains `preset TEXT` after `tokens_out`:

```sql
    CREATE TABLE IF NOT EXISTS messages (
        id         INTEGER PRIMARY KEY,
        session_id INTEGER NOT NULL,
        role       TEXT NOT NULL,
        content    TEXT NOT NULL,
        provider   TEXT,
        model      TEXT,
        tokens_in  INTEGER,
        tokens_out INTEGER,
        preset     TEXT,
        ts         INTEGER NOT NULL,
        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
    );
```

`migrate()` — after the `ai_messages` block (so a just-renamed legacy
table also gains the column):

```rust
    if table_exists("messages")? {
        let columns = columns("messages")?;
        if !columns.iter().any(|c| c == "preset") {
            // The armed prompt preset on a user turn (presets.rs) —
            // NULL on every row written before presets existed.
            conn.execute_batch("ALTER TABLE messages ADD COLUMN preset TEXT")?;
        }
    }
```

`Message` gains (after `tokens_out`):

```rust
    /// The `instruct` preset armed on this user send (presets.rs id) —
    /// `ask_retry` re-resolves it. NULL on assistant rows and rows
    /// written before presets existed.
    pub preset: Option<String>,
```

`MessageMeta` doc + field — update the doc to "provenance columns: the
armed preset rides user rows; provider/model/tokens ride assistant
replies" and add:

```rust
    /// The armed preset id — user rows only.
    pub preset: Option<String>,
```

`message_add_meta` — INSERT gains the column:

```rust
            "INSERT INTO messages (session_id, role, content, provider, model, tokens_in, tokens_out, preset, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                session_id,
                role,
                content,
                meta.provider,
                meta.model,
                meta.tokens_in,
                meta.tokens_out,
                meta.preset,
                ts
            ],
```

`messages_for` — SELECT and row map gain `preset` (index 8, `ts` moves
to 9):

```rust
            "SELECT id, session_id, role, content, provider, model, tokens_in, tokens_out, preset, ts
```

and in the `query_map` closure:

```rust
                preset: row.get(8)?,
                ts: row.get(9)?,
```

Check `storage.rs` for any other `SELECT … FROM messages` or
`Message { … }` literal construction sites and update them the same
way (the `ai_messages` merge INSERT lists explicit columns — unchanged).

- [ ] **Step 4: Run tests**

Run: `cd apps/native/src-tauri && cargo test storage::`
Expected: PASS — including the new round-trip + migration test.

- [ ] **Step 5: Commit**

```bash
git add apps/native/src-tauri/src/storage.rs
git commit -m "Storage: messages.preset column for armed instruct presets"
```

---

### Task 4: Ask pipeline — instruction append, `preset` persist, retry fidelity

**Files:**

- Modify: `apps/native/src-tauri/src/prompts.rs`
- Modify: `apps/native/src-tauri/src/ask.rs`
- Modify: `apps/native/src-tauri/src/lib.rs` (`ask_send` signature)

**Interfaces:**

- Consumes: `presets::resolve_instruct` (Task 1), `Config.prompts` (Task 2), `MessageMeta.preset`/`Message.preset` (Task 3)
- Produces: `ask_send` invoke arg `presetId`; `ask:state{loading}` payload gains `preset: string | null`; `live_system_prompt_with(language, instruction)`; consumed by Tasks 7–9.

- [ ] **Step 1: Write the failing tests**

`prompts.rs` tests append:

```rust
    #[test]
    fn live_system_prompt_with_appends_instruction() {
        let base = live_system_prompt_for("en");
        assert_eq!(live_system_prompt_with("en", None), base);
        assert_eq!(live_system_prompt_with("en", Some("  ")), base);
        let with = live_system_prompt_with("en", Some("Be terse."));
        assert!(with.starts_with(&base));
        assert!(with.ends_with("\n\nBe terse."));
    }
```

`ask.rs` tests append — first find how existing `send_chain` tests
build their `ScreenInput`/`emit`/`cancel` (the test module has helpers
— mirror them; e.g. `fn no_screen()` / `fn test_emit()`-style helpers
already exist):

```rust
    /// An `instruct` preset reaches the provider's system message and
    /// its id persists on the user row.
    #[tokio::test]
    async fn send_chain_applies_instruct_preset() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let mock = MockProvider::new(vec![Behavior::Tokens(vec!["ok".into()])]);
        let calls = mock.calls();
        let emit = |_: &str, _: serde_json::Value| {};
        let screen = /* the tests' no-screen ScreenInput helper */;
        let cancel = CancellationToken::new();
        send_chain(
            vec![candidate("mock", mock)],
            None,
            &db,
            &emit,
            &screen,
            &cancel,
            ChainOpts {
                text: "hi",
                instruction: Some("Be terse."),
                preset_id: Some("b:concise"),
                ..ChainOpts::default()
            },
        )
        .await
        .unwrap();
        let sent = calls.lock()[0].clone();
        assert!(sent[0].content.contains("Be terse."));
        let rows = db.messages_for(db.session_active_id("ask").unwrap().unwrap()).unwrap();
        assert_eq!(rows[0].preset.as_deref(), Some("b:concise"));
        let _ = std::fs::remove_dir_all(&dir);
    }
```

(Adapt to the module's actual test helpers — `tmp_dir`, `candidate`,
`MockProvider`, `Behavior`, and the `ScreenInput` stub all exist in the
test module already; copy the closest existing `send_chain` test's
setup verbatim and add the `instruction`/`preset_id` fields.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/native/src-tauri && cargo test live_system_prompt_with`
Expected: FAIL — function doesn't exist.

- [ ] **Step 3: Implement**

`prompts.rs`, after `live_system_prompt_for`:

```rust
/// `live_system_prompt_for` plus an optional per-send instruction — a
/// preset's `instruct` text appended AFTER the language directive for
/// that send only. `None`/blank leaves the base untouched.
pub fn live_system_prompt_with(language: &str, instruction: Option<&str>) -> String {
    let base = live_system_prompt_for(language);
    match instruction.map(str::trim).filter(|s| !s.is_empty()) {
        Some(extra) => format!("{base}\n\n{extra}"),
        None => base,
    }
}
```

`ask.rs`:

- Import: `use crate::prompts::{live_system_prompt_with, live_user_prompt};`
  (replace the `live_system_prompt_for` import — the `for` variant is
  only called through `with` after this change; update
  `live_system_prompt_for(language)` call sites accordingly.)
- `SendOpts` gains:

```rust
    /// The armed `instruct` preset id — resolved against the catalog +
    /// `prompts.custom` in `kick`; `None` for a plain send.
    pub preset: Option<String>,
```

- `AskService::send` signature gains `preset: Option<String>`:

```rust
    pub fn send(
        self: &Arc<Self>,
        app: &AppHandle,
        deps: &Deps<'_>,
        text: &str,
        with_screen: bool,
        listen_id: Option<i64>,
        preset: Option<String>,
    ) {
        self.kick(
            app,
            deps,
            SendOpts {
                text,
                with_screen,
                listen_id,
                preset,
                ..SendOpts::default()
            },
        );
    }
```

- `AskService::retry` — read `preset` off the last user row alongside
  `content`:

```rust
        let last = deps
            .db
            .session_active_id("ask")
            .ok()
            .flatten()
            .and_then(|sid| deps.db.messages_for(sid).ok())
            .and_then(|rows| {
                rows.iter()
                    .rev()
                    .find(|r| r.role == "user")
                    .map(|r| (r.content.clone(), r.preset.clone()))
            });
        let Some((text, preset)) = last else {
            log::warn!("ask::retry: no user turn to regenerate");
            return;
        };
        self.kick(
            app,
            deps,
            SendOpts {
                text: &text,
                preset,
                regenerate: true,
                ..SendOpts::default()
            },
        );
```

- `kick` — destructure `preset` from `SendOpts`; resolve the
  instruction inside the existing `deps.config.lock()` pre-flight
  block (same lock as `candidates`/`language`):

```rust
        let (candidates, vision, language, instruction) = {
            let cfg = deps.config.lock();
            let ks = deps.keystore.lock();
            let instruction = preset.as_deref().and_then(|id| {
                let found = crate::presets::resolve_instruct(id, &cfg.prompts.custom);
                if found.is_none() {
                    log::warn!("ask: preset {id} missing or not instruct — sending without");
                }
                found.map(|p| p.text)
            });
            (
                crate::provider_candidates(&cfg, &ks),
                crate::vision_candidate(&cfg, &ks),
                cfg.app.main_language.clone(),
                instruction,
            )
        };
```

  then move both into the spawned task and hand to `send_chain` —
  `preset_id` for the user row, `instruction` for the system prompt:

```rust
        let instruction = instruction; // String → moved into the task
        let preset_id = preset;        // Option<String> → moved
        tauri::async_runtime::spawn(async move {
            ...
            let _ = send_chain(
                candidates,
                vision,
                db.as_ref(),
                &emit,
                &screen_input,
                &cancel,
                ChainOpts {
                    text: &text,
                    fresh_session,
                    regenerate,
                    listen_id,
                    language: &language,
                    instruction: instruction.as_deref(),
                    preset_id: preset_id.as_deref(),
                },
            )
            .await;
        });
```

- `ChainOpts` gains:

```rust
    /// Resolved `instruct` preset text — appended to the system prompt.
    pub instruction: Option<&'a str>,
    /// The armed preset id — persisted on the user row so `retry`
    /// re-resolves it.
    pub preset_id: Option<&'a str>,
```

- `send_chain` destructures both; the two `loading` emits carry it for
  the live bubble (`emit(EV_STATE, json!({"state": "loading", "question": text, "preset": preset_id}))`
  — both the initial emit and the failover re-emit);

  `persist_user_message` call becomes:

```rust
    if !re_asked {
        persist_user_message(db, session_id, text, preset_id);
    }
```

  with:

```rust
/// The new user row, next to its session. `preset` records the armed
/// instruct preset (presets.rs id) so `retry` re-resolves the same
/// steering. `None` session skips the write — the stream must not die
/// on a storage hiccup.
fn persist_user_message(
    db: &Db,
    session_id: Option<i64>,
    text: &str,
    preset: Option<&str>,
) {
    let Some(sid) = session_id else {
        return;
    };
    let meta = MessageMeta {
        preset: preset.map(str::to_string),
        ..MessageMeta::default()
    };
    if let Err(e) = db.message_add_meta(sid, "user", text, &meta) {
        log::warn!("ask: failed to persist user message: {e}");
    }
}
```

- `stream_candidate` gains `instruction: Option<&str>` (last param);
  both `build_messages` calls pass it.
- `build_messages` gains `instruction: Option<&str>` and its system
  message becomes `live_system_prompt_with(language, instruction)`;
  update its doc comment. Fix all test-module `build_messages` calls
  (existing tests pass `None`).

- `lib.rs` `ask_send`:

```rust
#[tauri::command]
fn ask_send(
    app: AppHandle,
    text: String,
    with_screen: Option<bool>,
    listen_id: Option<i64>,
    preset_id: Option<String>,
) {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("ask_send dropped while gate != Main");
        return;
    }
    state.ask.send(
        &app,
        &state.deps(),
        &text,
        with_screen.unwrap_or(false),
        listen_id,
        preset_id,
    );
}
```

(Doc comment: add "`presetId` (optional) arms an `instruct` preset —
its text appends to the system prompt for this send and rides the
user row so `ask_retry` re-applies it.")

- [ ] **Step 4: Run tests**

Run: `cd apps/native/src-tauri && cargo test`
Expected: PASS — existing `build_messages`/`send_chain` tests updated
with `None` fields, new preset tests green.

- [ ] **Step 5: Commit**

```bash
git add apps/native/src-tauri/src/prompts.rs apps/native/src-tauri/src/ask.rs apps/native/src-tauri/src/lib.rs
git commit -m "Ask: instruct presets append to system prompt, persist on user row, re-resolve on retry"
```

---

### Task 5: Commands + native menu — `presets_list`, `presets_menu`, dispatch

**Files:**

- Modify: `apps/native/src-tauri/src/menus.rs`
- Modify: `apps/native/src-tauri/src/lib.rs`

**Interfaces:**

- Consumes: `presets::{all, find}` (Task 1), `Config.prompts` (Task 2)
- Produces: commands `presets_list` / `presets_menu`; event `bar:preset-pick` (`EV_PRESET_PICK`) carrying the full `Preset`; consumed by Tasks 6–8.

- [ ] **Step 1: Write the failing tests**

`menus.rs` — append a test module (file has none yet):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_item_id_strips_the_prefix() {
        assert_eq!(preset_item_id("preset.b:concise"), Some("b:concise"));
        assert_eq!(preset_item_id("menu.ask"), None);
        assert_eq!(preset_item_id("preset."), Some(""));
    }
}
```

`lib.rs` test module — extend
`removed_listen_placeholder_is_absent_from_command_contract` (or add a
new test) with:

```rust
        assert!(source.contains("presets_list,"));
        assert!(source.contains("presets_menu,"));
        assert!(source.contains("preset_item_id"));
        assert!(source.contains("EV_PRESET_PICK"));
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/native/src-tauri && cargo test preset_item_id`
Expected: FAIL — `preset_item_id` doesn't exist. The lib.rs contract
test also fails — commands not registered yet.

- [ ] **Step 3: Implement**

`menus.rs` — constants + helpers after `MENU_QUIT`:

```rust
/// `preset.<id>` — one item per preset; dispatch emits the picked
/// preset to the bar as `bar:preset-pick`. Kept OUTSIDE the `menu.*`
/// namespace so the shared dispatcher's prefix rules stay untouched.
pub const PRESET_ITEM_PREFIX: &str = "preset.";
const MENU_PRESET_TEMPLATES: &str = "menu.presets.templates";
const MENU_PRESET_INSTRUCT: &str = "menu.presets.instruct";

/// The preset id a `preset.*` menu event names, else `None`.
pub fn preset_item_id(item_id: &str) -> Option<&str> {
    item_id.strip_prefix(PRESET_ITEM_PREFIX)
}

/// The wand button's popup (and the input-row right-click): two
/// submenus — Templates, then Instructions — built fresh so custom
/// presets always show; customs follow built-ins within each kind.
pub fn build_preset_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let custom = app.state::<AppState>().config.lock().prompts.custom.clone();
    let presets = crate::presets::all(&custom);
    let mut subs: Vec<Submenu<Wry>> = Vec::new();
    for (menu_id, label, kind) in [
        (MENU_PRESET_TEMPLATES, "Templates", crate::presets::PresetKind::Template),
        (MENU_PRESET_INSTRUCT, "Instructions", crate::presets::PresetKind::Instruct),
    ] {
        let mut items: Vec<MenuItem<Wry>> = Vec::new();
        for p in presets.iter().filter(|p| p.kind == kind) {
            items.push(MenuItem::with_id(
                app,
                format!("{PRESET_ITEM_PREFIX}{}", p.id),
                p.name.as_str(),
                true,
                None::<&str>,
            )?);
        }
        let sub = Submenu::with_id(app, menu_id, label, true)?;
        let refs: Vec<&dyn IsMenuItem<Wry>> =
            items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
        sub.append_items(&refs)?;
        subs.push(sub);
    }
    let refs: Vec<&dyn IsMenuItem<Wry>> =
        subs.iter().map(|s| s as &dyn IsMenuItem<Wry>).collect();
    Menu::with_items(app, &refs)
}
```

`lib.rs`:

- Next to `EV_BAR_SHOW_HISTORY` (~line 643):

```rust
/// Emitted to the `bar` window only — a `preset.*` menu pick
/// (menu_dispatch); payload is the full `Preset`.
const EV_PRESET_PICK: &str = "bar:preset-pick";
```

- `menu_dispatch` — add the arm BEFORE `match id` (next to the
  `pos_edge_dir` early return):

```rust
        if let Some(pid) = menus::preset_item_id(id) {
            let custom = app.state::<AppState>().config.lock().prompts.custom.clone();
            if let Some(p) = presets::find(pid, &custom) {
                let _ = app.emit_to(windows::BAR_LABEL, EV_PRESET_PICK, p);
            }
            return;
        }
```

- New commands — place beside `bar_context_menu`:

```rust
/// The merged preset list — built-ins first, then `prompts.custom`.
/// The bar's picker/chip matcher and the prefs tab read it; `ask_send`
/// resolves ids against the same order server-side.
#[tauri::command]
fn presets_list(state: State<'_, AppState>) -> Vec<presets::Preset> {
    presets::all(&state.config.lock().prompts.custom)
}

/// The composer wand's popup (and the input-row right-click): the
/// preset menu pops at the cursor, built fresh so customs always show.
/// Gate-guarded like `ask_send` — a crafted invoke during onboarding
/// must not pop chrome over the wizard.
#[tauri::command]
fn presets_menu(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("presets_menu dropped while gate != Main");
        return Ok(());
    }
    let menu = menus::build_preset_menu(&app).map_err(|e| e.to_string())?;
    let bar = app
        .get_webview_window(windows::BAR_LABEL)
        .ok_or("bar window missing")?;
    bar.popup_menu(&menu).map_err(|e| e.to_string())
}
```

- `generate_handler!` — add `presets_list,` and `presets_menu,` (slot
  them beside `bar_context_menu,`).

- [ ] **Step 4: Run tests**

Run: `cd apps/native/src-tauri && cargo test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/native/src-tauri/src/menus.rs apps/native/src-tauri/src/lib.rs
git commit -m "Preset menu + presets_list command; preset.* dispatch emits bar:preset-pick"
```

---

### Task 6: Webview plumbing — types, commands, event, icon

**Files:**

- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/lib/events.ts`
- Modify: `packages/ui/src/index.ts`

**Interfaces:**

- Produces: `Preset`, `PresetKind`, `PromptPrefs`, `presetsList()`,
  `presetsMenu()`, `askSend(text, withScreen?, listenId?, presetId?)`,
  `Config.prompts`, `Message.preset`, `EV_PRESET_PICK`,
  `WandSparklesIcon` — consumed by Tasks 7–10.

- [ ] **Step 1: Implement** (no testable behavior — pure plumbing)

`commands.ts` — near the other interfaces:

```ts
export type PresetKind = 'instruct' | 'template';

/** `presets_list` row — built-ins (`b:` ids) then `prompts.custom`
 *  (`u:` ids). `instruct` appends `text` to a send's system prompt;
 *  `template` expands `{input}`/`{lang}` into the composer. */
export interface Preset {
  id: string;
  name: string;
  kind: PresetKind;
  text: string;
}

/** `[prompts]` section — user presets only. */
export interface PromptPrefs {
  custom: Preset[];
}
```

`Config` gains `prompts: PromptPrefs;` (after `vision`).
`Message` gains `preset: string | null;` (after `tokens_out`, doc:
"the armed instruct preset id — user rows only").

`askSend` gains the param:

```ts
/** Fire-and-forget: returns after pre-flight; tokens stream as `ask:*`.
 *  `withScreen` (the bar's Cmd/Ctrl+Enter) is the explicit attach flag —
 *  a screen read runs even when the text shows no intent. `listenId`
 *  binds the send to a listen doc — its own ask session (one chat per
 *  doc), its summary+transcript as the meeting context. `presetId` arms
 *  an `instruct` preset for this send only. */
export const askSend = (
  text: string,
  withScreen = false,
  listenId?: number,
  presetId?: string,
) => invoke<void>('ask_send', { text, withScreen, listenId, presetId });
```

and the new commands:

```ts
/** The merged preset list — built-ins then customs. */
export const presetsList = () => invoke<Preset[]>('presets_list');

/** Pop the native preset menu at the cursor — picks arrive as
 *  `bar:preset-pick` events carrying the full `Preset`. */
export const presetsMenu = () => invoke<void>('presets_menu');
```

`events.ts` — after `EV_BAR_SHOW_HISTORY`:

```ts
/** Emitted to the `bar` window only — a `preset.*` menu pick
 * (lib.rs `menu_dispatch`); payload is the full `Preset`. */
export const EV_PRESET_PICK = 'bar:preset-pick';
```

`packages/ui/src/index.ts` — add `WandSparklesIcon` to the lucide
export list (alphabetical, after `VideoIcon`).

- [ ] **Step 2: Verify**

Run: `cd apps/native && bun run check-types`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add apps/native/src/lib/commands.ts apps/native/src/lib/events.ts packages/ui/src/index.ts
git commit -m "Webview plumbing: Preset types, presets_list/menu commands, bar:preset-pick"
```

---

### Task 7: `src/lib/presets.ts` — slash matcher + template expander

**Files:**

- Create: `apps/native/src/lib/presets.ts`
- Test: `apps/native/src/lib/presets.test.ts`

**Interfaces:**

- Consumes: `Preset` (Task 6)
- Produces: `slashToken`, `matchPreset`, `resolveSlash`, `expandTemplate`, `langName` — consumed by Task 8.

- [ ] **Step 1: Write the failing test** — `presets.test.ts`:

```ts
import { describe, expect, test } from 'bun:test';
import type { Preset } from './commands';
import {
  expandTemplate,
  langName,
  matchPreset,
  resolveSlash,
  slashToken,
} from './presets';

const P = (id: string, name: string, kind: Preset['kind']): Preset => ({
  id,
  name,
  kind,
  text: '',
});

const presets = [
  P('b:sum', 'Sum', 'instruct'),
  P('b:summarize', 'Summarize', 'instruct'),
  P('u:xx', 'Reply', 'template'),
];

describe('slashToken', () => {
  test('splits the caret-0 token from the rest', () => {
    expect(slashToken('hello')).toBeNull();
    expect(slashToken('/')).toEqual({ token: '', rest: '' });
    expect(slashToken('/sum')).toEqual({ token: 'sum', rest: '' });
    expect(slashToken('/sum hello world')).toEqual({
      token: 'sum',
      rest: ' hello world',
    });
    expect(slashToken('/sum.x')).toEqual({ token: 'sum', rest: '.x' });
    expect(slashToken('mid /sum')).toBeNull();
  });
});

describe('matchPreset', () => {
  test('matches id suffix, full id, or name — first in list order', () => {
    expect(matchPreset('sum', presets)?.id).toBe('b:sum');
    expect(matchPreset('SUM', presets)?.id).toBe('b:sum');
    expect(matchPreset('summarize', presets)?.id).toBe('b:summarize');
    expect(matchPreset('reply', presets)?.id).toBe('u:xx');
    expect(matchPreset('u:xx', presets)?.id).toBe('u:xx');
    expect(matchPreset('nope', presets)).toBeNull();
    expect(matchPreset('', presets)).toBeNull();
  });
});

describe('resolveSlash', () => {
  test('eager pass needs a whitespace terminator', () => {
    expect(resolveSlash('/sum ', presets, false)?.preset.id).toBe('b:sum');
    expect(resolveSlash('/sum', presets, false)).toBeNull();
    expect(resolveSlash('/sum', presets, true)?.preset.id).toBe('b:sum');
    expect(resolveSlash('/sumx ', presets, true)).toBeNull();
    expect(resolveSlash('no slash', presets, true)).toBeNull();
  });

  test('rest strips exactly one terminator space', () => {
    expect(resolveSlash('/sum  two', presets, false)?.rest).toBe(' two');
    expect(resolveSlash('/sum two', presets, false)?.rest).toBe('two');
  });
});

describe('expandTemplate', () => {
  test('substitutes {input} and {lang}', () => {
    expect(expandTemplate('T {lang}: {input}', 'hello', 'English')).toBe(
      'T English: hello',
    );
  });

  test('appends input when {input} is absent', () => {
    expect(expandTemplate('Do this', 'x', 'en')).toBe('Do this\n\nx');
    expect(expandTemplate('Do this', '', 'en')).toBe('Do this');
  });

  test('empty input clears the placeholder', () => {
    expect(expandTemplate('T: {input}', '', 'en')).toBe('T: ');
  });
});

test('langName mirrors prompts.rs', () => {
  expect(langName('zh')).toBe('Chinese');
  expect(langName('bogus')).toBe('English');
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/native && bun test src/lib/presets.test.ts`
Expected: FAIL — module doesn't exist.

- [ ] **Step 3: Implement** — `presets.ts`:

```ts
/**
 * Composer preset helpers — the `/name` shorthand's token matcher and
 * the `template` kind's `{input}`/`{lang}` expansion. Mirrors
 * `src-tauri/src/presets.rs` (catalog order is the same list the
 * backend serves via `presets_list`).
 */
import type { Preset } from './commands';

/** The caret-0 `/token`: `text` starts with `/`; the token is the
 *  `[a-z0-9-]` run after it; `rest` is everything that follows
 *  (leading whitespace included). `null` when text isn't a slash
 *  command. */
export const slashToken = (
  text: string,
): { token: string; rest: string } | null => {
  const m = /^\/([a-z0-9-]*)([\s\S]*)$/i.exec(text);
  if (!m) return null;
  return { token: m[1], rest: m[2] };
};

/** First preset matching `token` — full id, id suffix (`b:sum` →
 *  `sum`), or name, all case-insensitive; list order is the catalog's
 *  (built-ins then customs), so a custom never shadows a built-in. */
export const matchPreset = (
  token: string,
  presets: Preset[],
): Preset | null => {
  const t = token.trim().toLowerCase();
  if (!t) return null;
  return (
    presets.find(
      (p) =>
        p.id.toLowerCase() === t ||
        p.id.split(':')[1]?.toLowerCase() === t ||
        p.name.toLowerCase() === t,
    ) ?? null
  );
};

export interface SlashHit {
  preset: Preset;
  /** Composer text after the consumed `/token` (+ one space). */
  rest: string;
}

/** Leading-`/token` resolution. The eager pass (per keystroke,
 *  `endOfText: false`) requires a whitespace terminator — an
 *  end-of-text token stays literal so a shorter prefix name can't
 *  fire mid-word (`/sum` while `summarize` also exists). The send-time
 *  pass (`endOfText: true`) additionally accepts the bare end-of-text
 *  token. */
export const resolveSlash = (
  text: string,
  presets: Preset[],
  endOfText: boolean,
): SlashHit | null => {
  const slash = slashToken(text);
  if (!slash) return null;
  const terminated = /^\s/.test(slash.rest);
  if (!terminated && !(endOfText && slash.rest === '')) return null;
  const preset = matchPreset(slash.token, presets);
  if (!preset) return null;
  return { preset, rest: terminated ? slash.rest.replace(/^\s/, '') : '' };
};

/** English names mirroring `prompts::language_name` — `{lang}` expands
 *  at pick time so the composer shows the concrete instruction. */
const LANG_NAMES: Record<string, string> = {
  zh: 'Chinese',
  ja: 'Japanese',
  ko: 'Korean',
  fr: 'French',
  es: 'Spanish',
};

export const langName = (code: string): string =>
  LANG_NAMES[code.trim()] ?? 'English';

/** `{lang}`/`{input}` substitution. `{input}` absent → `input`
 *  appended after the template; empty input clears the placeholder so
 *  the caret lands where the argument goes. */
export const expandTemplate = (
  text: string,
  input: string,
  lang: string,
): string => {
  const withLang = text.replaceAll('{lang}', lang);
  if (withLang.includes('{input}')) {
    return withLang.replaceAll('{input}', input);
  }
  return input ? `${withLang}\n\n${input}` : withLang;
};
```

- [ ] **Step 4: Run tests**

Run: `cd apps/native && bun test src/lib/presets.test.ts`
Expected: PASS (all tests)

- [ ] **Step 5: Commit**

```bash
git add apps/native/src/lib/presets.ts apps/native/src/lib/presets.test.ts
git commit -m "Composer preset helpers: /token matcher, {input}/{lang} expander"
```

---

### Task 8: `Bar.tsx` — chip, wand button, `/` shorthand, menu pick

**Files:**

- Modify: `apps/native/src/views/Bar.tsx`

**Interfaces:**

- Consumes: everything from Tasks 6–7; `dictation.discard()`,
  `dictation.state`, `textRef`, `setText`, `inputRef` (existing).
- Produces: user-facing feature.

- [ ] **Step 1: State + list fetch + config sync**

In `Bar`, add after the existing state declarations (~line 124):

```ts
  /** The merged preset list (built-ins + customs) behind the wand
   *  menu and the `/name` shorthand; `armed` is the per-send instruct
   *  preset chip. `mainLang` feeds `{lang}` template expansion. */
  const [presets, setPresets] = useState<Preset[]>([]);
  const [armedPreset, setArmedPreset] = useState<Preset | null>(null);
  const [mainLang, setMainLang] = useState('en');
```

Find the existing `configGet()` effect (sets `barLocked`); extend its
`.then` with `setMainLang(cfg.app.main_language)`, and the existing
`EV_CONFIG_CHANGED` handler — add `setMainLang(cfg.app.main_language)`
plus `void presetsList().then(setPresets).catch(() => {})`. Add a
mount fetch beside the configGet call:

```ts
    void presetsList()
      .then(setPresets)
      .catch(() => {});
```

Imports to add: `presetsList, presetsMenu, type Preset` in the
commands import; `EV_PRESET_PICK` in the events import;
`resolveSlash, expandTemplate, langName` from `@/lib/presets`;
`WandSparklesIcon, XIcon` in the `@marvis/ui` import.

- [ ] **Step 2: Apply + pick handlers**

Add near `sendAsk`:

```ts
  /** Menu pick (`bar:preset-pick`) — templates expand the composer's
   *  text into `{input}` for editing; instructions arm the chip.
   *  Paused while dictation owns the field (the tracker would splice
   *  its final draft over the rewrite). */
  const applyPreset = (p: Preset) => {
    dictation.discard();
    if (p.kind === 'template') {
      setText(expandTemplate(p.text, textRef.current, langName(mainLang)));
    } else {
      setArmedPreset(p);
    }
    inputRef.current?.focus();
  };
  useTauriEvent<Preset>(EV_PRESET_PICK, (p) => {
    if (dictation.state === 'listening') return;
    applyPreset(p);
  });
```

In the field's change path — `dictation.handleChange` is currently
passed straight to `AskInput`. Replace `onChange={dictation.handleChange}`
with `onChange={onFieldChange}` and add:

```ts
  /** Slash shorthand: an exact `/name` token followed by a space
   *  applies on the spot (end-of-text tokens wait for send — a prefix
   *  name can't swallow a longer one mid-typing). */
  const onFieldChange = (e: ChangeEvent<HTMLTextAreaElement>) => {
    dictation.handleChange(e);
    if (dictation.state === 'listening') return;
    const hit = resolveSlash(e.target.value, presets, false);
    if (!hit) return;
    dictation.discard();
    if (hit.preset.kind === 'template') {
      setText(expandTemplate(hit.preset.text, hit.rest.trimStart(), langName(mainLang)));
    } else {
      setArmedPreset(hit.preset);
      setText(hit.rest);
    }
  };
```

(`ChangeEvent` needs importing from `react` — add to the type import.)

- [ ] **Step 3: Send-time resolution in `sendAsk`**

Replace `sendAsk`'s head (keep the `listenId` resolution + invoke tail):

```ts
  const sendAsk = async (withScreen = false, question?: string) => {
    let presetId = armedPreset?.id;
    if (question === undefined) {
      const raw = textRef.current;
      // Bare `/` opens the preset menu instead of sending a slash.
      if (raw.trim() === '/') {
        void presetsMenu().catch(() => {});
        return;
      }
      const hit = resolveSlash(raw, presets, true);
      if (hit) {
        const rest = hit.rest.trim();
        dictation.discard();
        if (hit.preset.kind === 'template') {
          // The expansion waits in the composer for editing — nothing
          // sends until the user submits it.
          setText(expandTemplate(hit.preset.text, rest, langName(mainLang)));
          return;
        }
        setArmedPreset(hit.preset);
        setText(rest);
        presetId = hit.preset.id;
        if (!rest) return; // armed — the request is still to come
      }
    }
    const t = (question ?? textRef.current).trim();
    if (!t) {
      return;
    }
    if (question === undefined) {
      setText('');
    }
    setArmedPreset(null); // one-shot: the chip clears when a send fires
    let listenId = section === 'listen' ? listenViewing?.id : undefined;
    // ... unchanged listenStatus fallback ...
    void askSend(t, withScreen, listenId, presetId).catch(() =>
      raise('Send failed'),
    );
  };
```

Wait — a subtlety: clearing `armedPreset` on a `question` send —
follow-up chips (`question` defined) also consume the armed preset.
That matches per-send semantics — keep it.

- [ ] **Step 4: Esc clears the chip**

In the `Escape` keydown branch (`dictation.discard(); setText('');
setOpen(false);`), add `setArmedPreset(null);` before `setText('')`.

- [ ] **Step 5: Chip + wand + right-click**

In the `<form>` row, before `<AskInput>`:

```tsx
        {/* The armed instruct preset — a one-shot chip: ✕ disarms,
            Esc shares the field discard, a fired send clears it. */}
        {armedPreset && (
          <span className='flex flex-none items-center gap-1 self-center rounded-full bg-accent-soft px-2 py-0.75 text-[11.5px] font-medium text-accent-text'>
            {armedPreset.name}
            <button
              type='button'
              aria-label={`Remove ${armedPreset.name} preset`}
              onClick={() => setArmedPreset(null)}
              className='-mr-0.5 rounded-full p-0.25 text-accent-text/70 transition-colors duration-(--motion-fast) hover:text-accent-text focus-visible:outline-2 focus-visible:outline-accent'>
              <XIcon className='size-3' />
            </button>
          </span>
        )}
```

After `<AskInput>` (before the capture/listen controls — gated on the
input row, not `controls`, same as `DictationWaveform`):

```tsx
        {/* Preset picker — a native popup (the pill's fixed 64px can't
            host a webview menu); picks arrive as bar:preset-pick. */}
        {showInputRow && (
          <BarButton
            label='Prompt presets'
            disabled={gate !== 'main'}
            onPress={() => void presetsMenu().catch(() => {})}>
            <WandSparklesIcon className='size-5' />
          </BarButton>
        )}
```

Update the stage's `onContextMenu` — right-click on the input row
pops the preset menu; the idle capsule keeps the shared menu:

```tsx
      onContextMenu={(e) => {
        e.preventDefault();
        if (gate !== 'main') return;
        // The input row's right-click is the preset picker; the idle
        // capsule's is the shared menu.
        if (showInputRow) {
          void presetsMenu().catch(() => {});
        } else {
          void barContextMenu().catch(() => {});
        }
      }}
```

- [ ] **Step 6: Typecheck**

Run: `cd apps/native && bun run check-types`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/native/src/views/Bar.tsx
git commit -m "Bar: preset chip, wand menu, /name shorthand"
```

---

### Task 9: `ChatSection` — preset suffix on user bubbles

**Files:**

- Modify: `apps/native/src/components/ChatSection.tsx`

**Interfaces:**

- Consumes: `Message.preset`, `presetsList`, `EV_CONFIG_CHANGED`, `EV_ASK_STATE` payload `preset` (Task 4 emit).
- Produces: `· {name}` provenance suffix on preset-armed user bubbles.

- [ ] **Step 1: Implement**

- `ChatMsg` gains `preset?: string | null;`.
- `AskStatePayload` gains `preset?: string | null;`.
- `applyLoading` gains a `preset` param — the appended user row gets
  `preset` (both call sites: `applyLoading(prev, p.question ?? '', p.preset)`
  in the `loading` handler; mount resync passes `undefined`).
- `rowsToMsgs` maps `preset: r.preset`.
- Preset list state:

```ts
  const [presets, setPresets] = useState<Preset[]>([]);
  useEffect(() => {
    void presetsList().then(setPresets).catch(() => {});
  }, []);
  useTauriEvent<Config>(EV_CONFIG_CHANGED, () => {
    void presetsList().then(setPresets).catch(() => {});
  });
```

- In the user-bubble meta row (beside the timestamp span), add:

```tsx
                  {m.preset && (
                    <span
                      className={cn(
                        NUM,
                        'text-[10px] text-muted-foreground opacity-0 transition-opacity group-hover/row:opacity-100',
                      )}>
                      · {presets.find((p) => p.id === m.preset)?.name ?? m.preset}
                    </span>
                  )}
```

- Imports: `presetsList`, `type Preset`, `type Config` in the commands
  import; `EV_CONFIG_CHANGED` in the events import.

- [ ] **Step 2: Typecheck + commit**

Run: `cd apps/native && bun run check-types && bun test`
Expected: PASS.

```bash
git add apps/native/src/components/ChatSection.tsx
git commit -m "Chat: preset provenance suffix on armed user bubbles"
```

---

### Task 10: `PromptsTab` — custom preset CRUD

**Files:**

- Create: `apps/native/src/components/prefs/PromptsTab.tsx`
- Modify: `apps/native/src/components/prefs/SettingsMode.tsx`

**Interfaces:**

- Consumes: `presetsList`, `configSet('prompts.custom', …)`, `PrefsData`
- Produces: Settings → Prompts tab.

- [ ] **Step 1: Implement `PromptsTab.tsx`**

```tsx
/**
 * Prompts — the Ask preset list. Built-ins ship with the app
 * (read-only); custom presets persist as `prompts.custom` and apply
 * per-send from the composer's wand menu or `/name` shorthand.
 * `instruct` presets steer the reply (system-prompt append);
 * `template` presets expand into the composer — `{input}` is the typed
 * text, `{lang}` the main language.
 */
import { useEffect, useState } from 'react';
import { Trash2Icon } from '@marvis/ui';
import {
  configSet,
  presetsList,
  type Preset,
  type PresetKind,
} from '@/lib/commands';
import {
  BTN_OUTLINE,
  BTN_PRIMARY,
  BTN_SM,
  H2,
  ICON_BTN,
  PRF_ROW,
  PRF_ROWS,
  PR_LABEL,
  PR_SUB,
  SUB,
  cn,
} from '@/lib/classes';
import { PrefRow, Seg, Tag } from './bits';
import type { PrefsData } from './types';

const KINDS: { id: PresetKind; label: string }[] = [
  { id: 'instruct', label: 'Instruction' },
  { id: 'template', label: 'Template' },
];

const INPUT =
  'w-full rounded-lg border border-border bg-input-well px-2.5 py-1.5 text-[12.5px] text-foreground outline-none transition-[border-color,box-shadow] duration-(--motion-fast) ease-(--ease) focus:border-accent focus:shadow-(--focus-ring)';

const mintId = () => `u:${Math.random().toString(36).slice(2, 10)}`;

export const PromptsTab = ({ data }: { data: PrefsData }) => {
  const customs = data.config?.prompts.custom ?? [];
  const [presets, setPresets] = useState<Preset[]>([]);
  useEffect(() => {
    void presetsList().then(setPresets).catch(() => {});
  }, []);
  const builtins = presets.filter((p) => p.id.startsWith('b:'));

  const [editing, setEditing] = useState<Preset | null>(null);
  const [isNew, setIsNew] = useState(false);

  const write = (next: Preset[]) =>
    void configSet('prompts.custom', next)
      .then(data.setConfig)
      .catch(() => {});

  const startNew = () => {
    setIsNew(true);
    setEditing({ id: mintId(), name: '', kind: 'instruct', text: '' });
  };
  const save = () => {
    if (!editing) return;
    const clean = {
      ...editing,
      name: editing.name.trim(),
      text: editing.text.trim(),
    };
    if (!clean.name || !clean.text) return;
    const next = isNew
      ? [...customs, clean]
      : customs.map((p) => (p.id === clean.id ? clean : p));
    write(next);
    setEditing(null);
  };

  return (
    <>
      <h2 className={H2}>Prompts</h2>
      <p className={SUB}>
        Presets apply to one Ask send — pick them from the composer's
        wand menu, or type <code>/</code> + a name (like{' '}
        <code>/summarize</code>). Instructions steer the reply;
        templates expand into your message.
      </p>

      <div className={PRF_ROWS}>
        {builtins.map((p) => (
          <PrefRow
            key={p.id}
            label={p.name}
            sub={p.text}>
            <Tag>{p.kind === 'instruct' ? 'Instruction' : 'Template'}</Tag>
          </PrefRow>
        ))}
      </div>

      <h3 className='mt-6 mb-2 text-[13px] font-[550]'>Your presets</h3>
      <div className={PRF_ROWS}>
        {customs.map((p) => (
          <PrefRow key={p.id} label={p.name} sub={p.text}>
            <button
              type='button'
              className={cn(BTN_SM, BTN_OUTLINE)}
              onClick={() => {
                setIsNew(false);
                setEditing(p);
              }}>
              Edit
            </button>
            <button
              type='button'
              aria-label={`Delete ${p.name}`}
              className={ICON_BTN}
              onClick={() => write(customs.filter((x) => x.id !== p.id))}>
              <Trash2Icon className='size-3.5' />
            </button>
          </PrefRow>
        ))}
        {customs.length === 0 && !editing && (
          <div className={cn(PRF_ROW, 'border-b-0')}>
            <div className={PR_SUB}>No custom presets yet.</div>
          </div>
        )}

        {editing ? (
          <div className={cn(PRF_ROW, 'flex-col items-stretch gap-2.5 border-b-0')}>
            <div className='flex items-center gap-3'>
              <input
                aria-label='Preset name'
                placeholder='Name — also the /name'
                className={cn(INPUT, 'w-48')}
                value={editing.name}
                onChange={(e) =>
                  setEditing({ ...editing, name: e.target.value })
                }
              />
              <Seg
                ariaLabel='Preset kind'
                options={KINDS}
                value={editing.kind}
                onChange={(kind) => setEditing({ ...editing, kind })}
              />
            </div>
            <textarea
              aria-label='Preset text'
              placeholder={
                editing.kind === 'instruct'
                  ? 'Instruction appended to the send — e.g. Answer like a skeptical reviewer.'
                  : 'Template text — {input} is the typed message, {lang} your main language.'
              }
              className={cn(INPUT, 'min-h-24 resize-y leading-relaxed')}
              value={editing.text}
              onChange={(e) => setEditing({ ...editing, text: e.target.value })}
            />
            <div className='flex justify-end gap-2'>
              <button
                type='button'
                className={cn(BTN_SM, BTN_OUTLINE)}
                onClick={() => setEditing(null)}>
                Cancel
              </button>
              <button
                type='button'
                className={cn(BTN_SM, BTN_PRIMARY)}
                disabled={!editing.name.trim() || !editing.text.trim()}
                onClick={save}>
                {isNew ? 'Add preset' : 'Save'}
              </button>
            </div>
          </div>
        ) : (
          <div className={cn(PRF_ROW, 'border-b-0')}>
            <div>
              <div className={PR_LABEL}>New preset</div>
              <div className={PR_SUB}>
                Name it something short — it becomes the <code>/name</code>{' '}
                shorthand too.
              </div>
            </div>
            <span className='inline-flex flex-none items-center gap-2'>
              <button
                type='button'
                className={cn(BTN_SM, BTN_OUTLINE)}
                onClick={startNew}>
                New preset
              </button>
            </span>
          </div>
        )}
      </div>
    </>
  );
};
```

- [ ] **Step 2: Register the tab** — `SettingsMode.tsx`:

Import `MessageSquareTextIcon` (already in the `@marvis/ui` import —
add to that statement) and `PromptsTab`; add to `TABS` between
`recording` and `providers`:

```ts
  { id: 'prompts', label: 'Prompts', icon: MessageSquareTextIcon },
```

and the render branch:

```tsx
        {tab === 'prompts' && <PromptsTab data={data} />}
```

- [ ] **Step 3: Verify**

Run: `cd apps/native && bun run check-types && bun run build`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add apps/native/src/components/prefs/PromptsTab.tsx apps/native/src/components/prefs/SettingsMode.tsx
git commit -m "Settings: Prompts tab — custom preset CRUD"
```

---

### Task 11: Full verification + manual pass

- [ ] **Step 1: Rust verification**

Run: `cd apps/native/src-tauri && cargo test && cargo clippy`
Expected: PASS, no new warnings.

- [ ] **Step 2: Webview verification**

Run: `cd apps/native && bun test && bun run check-types && bun run build`
Expected: PASS.

- [ ] **Step 3: Manual pass** — `bun run build:dev`, then:

- pill + card wand button → menu → pick each kind
- `/summarize` eager apply; `/concise`+Enter; `/bogus text` sends
  literally; `/`+Enter opens the menu
- instruct chip: arm, disarm via ✕, Esc clears, send clears; retry
  (`⋯` menu) keeps the steering; bubble shows `· {name}` on hover
- template: `/reply hello` expands; pick Translate with text typed;
  `{lang}` resolves to Settings' main language
- dictation mid-draft: `/x` doesn't fire while `listening`
- Settings → Prompts: add/edit/delete a custom preset; it appears in
  the wand menu + `/` matcher (bar picks it up via `config:changed`)
- right-click the input row → preset menu; right-click idle capsule →
  shared menu unchanged

- [ ] **Step 4: Final commit** — any fixes from the manual pass.
