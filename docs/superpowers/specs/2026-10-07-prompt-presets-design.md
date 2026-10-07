# Prompt Presets — Design

Date: 2026-10-07
Branch: `feature/deepen_the_loop` (Phase 1, item 1 of 6 for the user)
Scope: `apps/native` — Rust core, bar webview, settings webview.

## Context

Named in the v0.1.0 notes and the roadmap's Phase 1 ("becomes the seed
of skill bundles in Phase 3"). A preset is a named, reusable prompt —
two kinds, applied per send:

- `instruct` — steers HOW Marvis replies (system-prompt append for that
  send only); the user still types their own request.
- `template` — wraps/fills WHAT the user typed (the preset is the
  request scaffold); expanded into the composer so the sent text is
  literally what the model saw.

User decisions: presets are per-send (no pinned/session state); sources
are a built-in catalog + user-defined presets in `config.toml`; the
picker is a native popup menu plus a `/name` shorthand.

The pill window is fixed at `BAR_W × BAR_H` (600×64, `resizable(false)`)
— a webview popover can't open inside it, which is why the picker rides
the native-menu machinery (`popup_menu`, `menu_dispatch`, emit-back)
that the idle context menu already proves.

## Data model

```text
Preset = { id, name, kind, text }
id   : 'b:' + slug (built-in) | 'u:' + 8 alphanumerics (custom, webview-minted)
name : display label + /shorthand token — 1..=24 chars
kind : 'instruct' | 'template'
text : the instruction / template body — 1..=2000 chars
```

- **Built-ins**: `src-tauri/src/presets.rs` catalog — the single source
  of truth (the backend resolves ids at send time, the webview only
  ever sees the merged list).
- **Customs**: `Config.prompts.custom: Vec<Preset>` — new `[prompts]`
  section serializing as `[[prompts.custom]]`; `#[serde(default)]` so
  old configs load clean. Written via a new `config_set` key
  `prompts.custom` taking the full JSON array (CRUD = webview computes
  the next array, one write). `config:changed` broadcast is the
  existing mechanism.
- Write validation (server-side): `id` non-empty ≤ 40 chars; `name`
  non-empty ≤ 24; `kind` one of the two; `text` non-empty ≤ 2000;
  duplicate `id`s rejected, and a custom `id` equal to a built-in id is
  rejected (the built-in always resolves first — such a preset would be
  unreachable). Duplicate NAMES are allowed — the `/` shorthand
  resolves first-match in list order (built-ins sort first).

### Built-in catalog (v1)

| id | kind | text |
| --- | --- | --- |
| `b:concise` | instruct | `Answer briefly — a short paragraph or a tight list.` |
| `b:explain` | instruct | `Explain for a newcomer — define terms, avoid jargon.` |
| `b:advocate` | instruct | `Challenge this: strongest counterarguments first, then a verdict.` |
| `b:translate` | template | `Translate the following into {lang}:\n\n{input}` |
| `b:reply` | template | `Draft a reply to this message — match its tone:\n\n{input}` |
| `b:summarize` | template | `Summarize the following in 3–5 bullets:\n\n{input}` |

Placeholders: `{input}` = the composer's current text; `{lang}` =
`prompts::language_name(cfg.app.main_language)` — both expand at pick
time in the webview.

## Apply semantics

- **instruct** — picking arms a removable chip in the composer row.
  `ask_send` gains `preset: Option<String>` (the id). Server resolves;
  the send's system prompt becomes
  `live_system_prompt_for(language) + "\n\n" + preset.text`. One-shot:
  the chip clears when the send fires, on ✕, or on Esc (shares the
  field's existing discard).
- **template** — webview-side expansion only: the preset's text with
  `{input}`/`{lang}` substituted lands in the composer for editing
  (`{input}` absent → typed text is appended after the template). The
  user edits, then sends normally — the bubble and history show the
  expanded request. Template ids are never armed/sent as `preset`; a
  template id reaching `ask_send` is ignored with a warn log.
- **Unknown/deleted id** reaching `ask_send` → the send proceeds
  without it (`warn`) — a stale chip must never block a message.

## Picker

### Native menu

- `presets_menu` command (gated `Main`, same as `bar_context_menu`):
  builds a `Menu` of two `Submenu`s — "Templates" then "Instructions" —
  items `preset.<id>` labeled by `name`, customs after built-ins within
  each; `popup_menu` on the bar window at the cursor.
- `menus.rs` hosts the builder (preset list comes from
  `AppState.config` + the catalog); `menu_dispatch` gains a `preset.*`
  arm → `emit_to(BAR_LABEL, EV_PRESET_PICK, preset)` with the full
  preset payload (the webview needs `kind`/`text` to apply it).
- Trigger surface: a wand `BarButton` (`WandIcon`) in the composer row
  — pill and card alike, before the dictation control — plus
  `Presets ▸` gaining the same two submenus on the input-row context
  menu for discoverability. (The idle capsule's context menu stays as
  is.)

### `/` shorthand

- When composer text starts with `/` + `[a-z0-9-]+` at caret 0, the
  token (the run up to the first whitespace, or end-of-text) is matched
  case-insensitively against `id`-suffix and `name`, first hit in
  built-ins-then-customs order. It applies in two moments, whichever
  comes first:
  - **Eager** — the token exactly equals a match AND is followed by
    whitespace: the preset applies (template inserts, instruct arms)
    and the `/token` is removed along with one following space, leaving
    the rest intact. An end-of-text token does NOT apply eagerly —
    `/sum` must stay typeable when `summarize` also exists (the longer
    name would be unreachable otherwise).
  - **At send** — a still-unresolved leading `/token` tries the same
    exact match once: hit → apply, then the send proceeds (an
    `instruct` hit with nothing after the token arms the chip and the
    send early-returns on the now-empty text — the field stays armed
    for the next input); miss → the text is sent literally, `/`
    included — the shortcut never eats text.
- Text equal to exactly `/` + Enter opens the preset menu instead of
  sending a bare slash.
- Both paths pause while `dictation.state === 'listening'` (the tracker
  owns the text); a live dictation draft makes `/name` inert.

## Rust changes

| Where | Change |
| --- | --- |
| `presets.rs` (new) | `Preset`, `PresetKind`, `BUILTINS`, `resolve(id)`, `expand(text, input, lang)` |
| `storage.rs` | `messages` gains `preset TEXT` via the `has()`-migration pattern; `Message.preset: Option<String>`; `MessageMeta.preset` and `message_add_meta` write it |
| `prompts.rs` | `live_system_prompt_with(language, instruction)` — `live_system_prompt_for` + optional instruction block |
| `ask.rs` | `SendOpts.preset: Option<String>`; `build_messages` takes the resolved instruction; `persist_user_message` writes the preset id via `message_add_meta`; `retry` re-reads the last user row's `preset` and re-resolves (deleted preset → warn + proceed) |
| `lib.rs` | `presets_menu`, `presets_list` commands; `config_set` key `prompts.custom`; `menu_dispatch` `preset.*` arm; `generate_handler!` + contract-test updates |
| `menus.rs` | `build_preset_menu`, `preset.*` item ids, `Presets ▸` submenu on the input-row context menu |

## Webview changes

| Where | Change |
| --- | --- |
| `commands.ts` | `Preset`/`PresetKind` types, `presetsList()`, `presetsMenu()`, `askSend(text, withScreen?, listenId?, presetId?)` |
| `events.ts` | `EV_PRESET_PICK = 'bar:preset-pick'` + payload type |
| `Bar.tsx` | preset list state (`presetsList` on mount, refetch on `config:changed`); armed chip before `AskInput` (✕ disarms, Esc clears chip+text, send clears); `/` shorthand in `onChange`; `/`+Enter → menu; `bar:preset-pick` apply handler; wand `BarButton` |
| `ChatSection.tsx` | user bubble suffix `· {name}` when `m.preset` resolves against the list (muted; hidden when unresolvable) |
| `prefs/PromptsTab.tsx` (new) | built-ins read-only list; customs list + add/edit/delete (name input, kind `Seg`, text textarea); writes `prompts.custom` |
| `SettingsMode.tsx` | `Prompts` tab after Recording (`MessageSquareTextIcon`) |

## Testing

- `presets.rs`: catalog integrity (unique ids/names), `resolve`,
  `expand` (`{input}` present/absent, `{lang}`).
- `storage.rs`: `preset` round-trips through `message_add_meta` /
  `messages_for`; migration adds the column on an old-schema db.
- `prompts.rs`: instruction block appends after the language directive.
- `ask.rs`: preset instruction reaches the provider's system message;
  unknown id sends without it; retry re-applies the stored preset;
  user row persists `preset`.
- `config.rs`/`lib.rs`: `prompts.custom` write validation (bad kind,
  empty name/text, dup id) + the command contract test.
- `bun test` (`apps/native`) for the `/`-token matcher if it's
  extracted to `lib/`; `cargo test`, `cargo clippy`,
  `bun run check-types`.
- Manual (`bun run build:dev`): wand button in pill + card; `/sum…`
  shorthand; instruct chip arm/disarm/send; template expansion +
  `{input}` pre-filled; retry keeps the instruction; custom preset
  CRUD round-trips through `config.toml`.

## Out of scope

- Pinned/session presets; preset hotkeys; ordering/folders;
  sharing/export (Phase 3 skill bundles); `marvis://` preset params;
  template presets resolved server-side; editing built-ins (a custom
  can shadow a built-in's name but the built-in still resolves its id).
