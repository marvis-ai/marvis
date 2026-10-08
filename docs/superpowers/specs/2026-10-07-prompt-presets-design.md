# Prompt Presets — Design

Date: 2026-10-07
Revised: 2026-10-08 — rewritten to document the shipped implementation.
The original draft specified a `kind` field (`instruct`/`template`)
and a native-menu picker; the shipped design drops `kind` (the
`{input}` placeholder is the whole contract) and replaces the native
menu with the glass palette window (`?view=palette`) plus the
`/`-typed session it powers. The plan doc
(`docs/superpowers/plans/2026-10-07-prompt-presets.md`) still
describes the superseded native-menu approach.
Branch: `feature/deepen_the_loop` (Phase 1, item 1 of 6 for the user)
Scope: `apps/native` — Rust core, bar webview, settings webview.

## Context

Named in the v0.1.0 notes and the roadmap's Phase 1 ("becomes the seed
of skill bundles in Phase 3"). A preset is a named, reusable prompt
applied per send — with exactly ONE behavioral switch:

- **`{input}` in the text** — the preset expands around the typed
  message at send time (the sent text is literally what the model
  saw). The composer shows the preset's name badge plus an editable
  `{lang}` badge when the text carries one.
- **No `{input}`** — the text is a silent instruction appended to
  that send's system prompt; the composer shows the name badge only.

User decisions: presets are per-send (no pinned/session state);
sources are a built-in catalog + user-defined presets in
`config.toml`; the picker is the palette overlay plus a `/name`
shorthand.

## Data model

```text
Preset = { id, name, text }
id   : 'b:' + slug (built-in) | 'u:' + 8 base36 chars (custom, webview-minted)
name : display label + /shorthand token — 1..=24 chars
text : the instruction / expansion body — 1..=2000 chars
```

- `is_template(p) = p.text.contains('{input}')` — the only kind
  distinction that exists, derived, never stored (`presets.rs` /
  `lib/presets.ts` agree). An instruction containing a literal
  `{input}` expands; that is the documented contract, not a bug.
- `has_lang_param(p) = p.text.contains('{lang}')` — flags the
  editable language badge while armed.
- **Built-ins**: `src-tauri/src/presets.rs` catalog — the single
  source of truth (the backend resolves ids at send time, the webview
  only ever sees the merged list).
- **Customs**: `Config.prompts.custom: Vec<Preset>` — serializes as
  `[[prompts.custom]]`; `#[serde(default)]` so old configs load clean
  (a stale `kind` key from the pre-revision schema deserializes
  harmlessly). Written via `config_set` key `prompts.custom` taking
  the full JSON array (CRUD = webview computes the next array, one
  write). `config:changed` broadcast is the existing mechanism.
- Write validation (server-side, `presets::validate_custom` — a
  write rejects on the first bad row): trimmed `id` ≤ 40 chars of
  `[a-zA-Z0-9:_-]`; `name` ≤ 24 chars; `text` ≤ 2000 chars;
  duplicate ids rejected; a custom `id` equal to a built-in id is
  rejected; and every custom id must be `u:`-prefixed with a
  non-empty suffix (the catalog owns `b:` and every other namespace).
  Duplicate NAMES are allowed — the `/` shorthand resolves
  first-match in list order (built-ins sort first).

### Built-in catalog

| id | name | text |
| --- | --- | --- |
| `b:concise` | Concise | `Answer briefly — a short paragraph or a tight list.` |
| `b:explain` | Explain | `Explain for a newcomer — define terms, avoid jargon.` |
| `b:advocate` | Devil's advocate | `Challenge this: strongest counterarguments first, then a verdict.` |
| `b:translate` | Translate | `Translate the following into {lang}:\n\n{input}` |
| `b:reply` | Reply | `Draft a reply to this message — match its tone:\n\n{input}` |
| `b:summarize` | Summarize | `Summarize the following in 3–5 bullets:\n\n{input}` |

Placeholders: `{input}` = the composer's text at send time; `{lang}` =
the `{lang}` badge's value, seeded from
`prompts::language_name(cfg.app.main_language)` / `langName`.

## Apply semantics

- **Arm** — a pick (palette or `/name`) arms the preset as a badge in
  the composer row: the name chip (accent), plus a neutral editable
  `{lang}` badge when `has_lang_param`. One-shot: the badges clear
  when the send fires, on ✕, on Esc (shares the field's existing
  discard), or on Backspace/Delete at a collapsed caret-0.
- **Send, `{input}` preset** — webview-side expansion: `expandTemplate`
  substitutes `{lang}` (the badge's value, default main language) then
  `{input}` (absent → the typed text is appended after the template;
  empty input clears the placeholder). The EXPANDED text is sent — the
  bubble and history show it verbatim. The preset id still rides as
  `presetId`, persisting on the user row purely as provenance.
- **Send, non-`{input}` preset** — `ask_send` carries `presetId`;
  `kick` resolves it and the send's system prompt becomes
  `live_system_prompt_for(language)` + the text with `{lang}`
  substituted (`preset_lang` arg, else the configured main language).
- **`preset_lang`** — `ask_send`'s optional `presetLang` arg carries
  the badge's edited value. It is NOT persisted on the message row:
  `retry` re-reads the row's `preset` and re-resolves the id, but
  `{lang}` falls back to the configured main language.
- **Unknown/deleted id** reaching `ask_send` → the send proceeds
  without it (`warn`); an unresolved id is also dropped before
  persistence — a stale preset never claims provenance it never had
  and never blocks a message.

## Picker — the palette

The pick surface is the `?view=palette` window (`PALETTE_LABEL`,
300 px wide, borderless glass, lazy like `picker`) — a flat merged
list (built-ins then customs), each row `name` + its `/token` slug.
`is_template` is invisible here; a pick arms its badge either way.
The filter lives in the composer, not the palette.

### Anchoring and height

- Left edge tracks the composer's caret screen-x (`caretViewportX`
  viewport px + the window's `outer_position`); fallbacks: pointer x,
  then the bar's center.
- Vertical: card mode pops the palette ABOVE the composer row's top
  edge (`anchor_y` — the row is the card's bottom-anchored footer);
  collapsed, `layout::palette_rect` picks above/below the pill by free
  space. Always clamped to the work area.
- Height auto-fits: the view reports its natural content height
  (`presets_palette_height`), clamped to `[PALETTE_MIN_H=64,
  PALETTE_MAX_H=228]`; the list scrolls past the cap (~6 rows, the
  7th peeking as the scroll affordance). The last reported height is
  reused across opens so a reopened palette doesn't flicker.

### Focus modes

- **Focused** — the wand and right-click opens take key focus: this
  window's own keydown drives nav, click-away blur dismisses
  menu-style (`Focused(false)` while `palette_focused`).
- **Unfocused** (`/`-typed open) — ordered front without key (macOS
  `orderFront:`; `show()` elsewhere — if that activates, the
  `Focused(true)` arm promotes the palette into focused mode anyway);
  the composer keeps focus and the `/token`. Its filter streams in as
  `palette:query` on every edit; nav keys arrive forwarded as
  `palette:key` (`PALETTE_KEYS`: `Escape`, `ArrowDown`, `ArrowUp`,
  `Home`, `End`, `Enter`, `Tab` — the same list the palette's own
  keydown consumes). Leaving the `/` prefix closes it;
  a real click on the palette promotes it to focused
  (`accept_first_mouse`) so click-away still dismisses; the bar
  losing focus retires it (`hide_palette_unfocused`, deferred one
  main-queue turn so a palette click can't kill it mid-promotion).
- Every hide — pick, Esc, blur, bar blur — emits
  `bar:palette-closed`; the composer clears its key-forwarding gate on
  it.

### Commands and events

- `presets_palette_open` (`anchorX?`, `anchorY?`, `query?`,
  `focused?` — Gate::Main) → `show_palette` + `palette:open
  { query }` to the palette.
- `presets_palette_select` (`id`, Gate::Main) → hide, refocus the
  bar, emit `bar:preset-pick` with the full `Preset`.
- `presets_palette_close` / `_key` / `_query` / `_height` — the
  `/`-session plumbing (dismiss, key forward, filter push, height
  report); all Gate::Main, `Result<(), String>`.
- `presets_list` → the merged list (built-ins then customs).
- Events: `palette:open { query }`, `palette:query { query }`,
  `palette:key { key }` (all → palette), `bar:preset-pick { Preset }`,
  `bar:palette-closed` (→ bar).

### Apply on pick

`bar:preset-pick` → `applyPreset` (no-ops while dictation is
listening or the composer isn't rendered): arms the badge, then —
when the field holds the `/token` trigger text — consumes it
(`stripSlashToken`). `/name args` + pick sends straight through
(the pick is the run); a bare `/name` arms for editing.

### `/` shorthand

- A `/` typed at caret-0 (or as the type-to-wake char on the
  collapsed pill) opens the palette UNFOCUSED immediately — the
  `/token` is its live filter while it lasts.
- When composer text starts with `/` + `[a-z0-9-]+` at caret 0, the
  token (the run up to the first whitespace, or end-of-text) matches
  case-insensitively against full id, id suffix, name, and the name's
  `presetToken` slug, first hit in built-ins-then-customs order. It
  applies in two moments, whichever comes first:
  - **Eager** — the token exactly equals a match AND is followed by
    whitespace: the preset arms and the `/token` is removed along
    with one following space, leaving the rest intact. An
    end-of-text token does NOT apply eagerly — `/sum` stays typeable
    while `summarize` also exists.
  - **At send** — a still-unresolved leading `/token` tries the same
    exact match once: hit → arm, then the send proceeds (a hit with
    nothing after the token just arms and early-returns — the field
    stays armed for the next input); miss → the text is sent
    literally, `/` included — the shortcut never eats text.
- Text equal to exactly `/` + Enter opens the palette (unfocused)
  instead of sending a bare slash.
- Both paths pause while `dictation.state === 'listening'` (the
  tracker owns the text); a live dictation draft makes `/name`
  inert.

### Trigger surfaces

- A wand `BarButton` (`WandSparklesIcon`) in the composer row —
  pill and card alike, before the dictation control — opens the
  palette focused.
- Right-click on any composer-rendered surface opens the palette
  focused INSTEAD of the shared context menu; surfaces without a
  composer (idle capsule, history card, gate/error rows) keep the
  shared menu.
- The `/` keystroke paths above.

## Rust changes

| Where | Change |
| --- | --- |
| `presets.rs` | `Preset{id,name,text}`, `is_template`, `builtins`, `all`/`find`/`resolve`, `validate`/`validate_custom` |
| `storage.rs` | `messages` gains `preset TEXT` via the `has()`-migration pattern; `Message.preset: Option<String>`; `MessageMeta.preset` and `message_add_meta` write it |
| `prompts.rs` | `live_system_prompt_with(language, instruction)` — `live_system_prompt_for` + optional instruction block |
| `ask.rs` | `SendOpts{preset, preset_lang}`; resolved-id-only persistence + `loading` emits; an `is_template` preset contributes no instruction text (provenance only); `{lang}` substitutes `preset_lang`/`main_language`; `retry` re-reads the last user row's `preset` and re-resolves (deleted preset → warn + proceed) |
| `lib.rs` | `presets_list`; `presets_palette_{open,select,close,key,query,height}` (Gate::Main); `config_set` key `prompts.custom`; `EV_PRESET_PICK`/`EV_PALETTE_*` consts; `generate_handler!` + contract-test updates |
| `windows/mod.rs` + `layout.rs` | the palette window (`PALETTE_LABEL`, lazy, `accept_first_mouse`); `show_palette`/`hide_palette`/`hide_palette_unfocused`; `palette_focused`/`palette_h`/`PaletteAnchor`; `palette_placement` + `layout::palette_rect`; `set_palette_height`; focus/blur arms (`try_lock` on the main thread); `palette_key`/`palette_query` emits |

## Webview changes

| Where | Change |
| --- | --- |
| `commands.ts` | `Preset{id,name,text}`; `AskSendOpts`; `askSend(text, opts)`; `presetsList()`; `presetsPalette{Open,Select,Close,Key,Query,Height}()` |
| `events.ts` | `EV_PRESET_PICK`, `EV_PALETTE_{OPEN,QUERY,KEY,CLOSED}` + payload types |
| `lib/presets.ts` | `slashToken`/`presetToken`/`slashQuery`/`stripSlashToken`/`matchPreset`/`resolveSlash`/`isTemplate`/`hasLangParam`/`langName`/`expandTemplate`/`PALETTE_KEYS` |
| `lib/caret.ts` | `caretViewportX` — canvas-measured caret x for the palette anchor |
| `hooks/usePresets.ts` | merged-list fetch on mount + `config:changed` refetch — shared by Bar/ChatSection/PresetsTab |
| `views/Palette.tsx` | `?view=palette` — flat filterable list, name + `/token` rows, `PALETTE_KEYS` nav (own keydown in focused mode, `palette:key` forwarded in `/` mode), `presetsPaletteHeight` auto-fit report |
| `Bar.tsx` | `usePresets`; armed badge group (name chip + editable `{lang}` badge with Esc-revert); `applyPreset`; `openPalette` (caret/`anchorY` anchors, focused flag); `/`-session plumbing (auto-open, `palette:query`, `presetsPaletteClose`); Esc ownership (palette first, then card/field); `sendAsk` slash resolution + `expandTemplate`; composer right-click → palette |
| `components/bar/AskInput.tsx` | forwards `PALETTE_KEYS` while `paletteOpen` (consumed, not bubbled); `onDisarm` caret-0 Backspace/Delete disarm |
| `ChatSection.tsx` | user bubble suffix `· {name}` — muted, rendered on row hover; resolves the id against the list, falls back to the raw id when unresolvable |
| `prefs/PresetsTab.tsx` | built-ins read-only list; customs list + add/edit/delete (name + text only — the `{input}`/`{lang}` hint explains behavior); writes `prompts.custom`, keeps the editor open on a rejected save |
| `SettingsMode.tsx` | `Presets` tab after Recording (`MessageSquareTextIcon`) |

## Testing

- `presets.rs`: catalog integrity (unique `b:` ids, both behaviors
  reachable), `is_template`, `find`/`resolve`, `validate_custom`
  (bad id/name/text, dup, `b:` squat, bare `u:`).
- `storage.rs`: `preset` round-trips through `message_add_meta` /
  `messages_for`; migration adds the column on an old-schema db.
- `prompts.rs`: instruction block appends after the language
  directive.
- `ask.rs`: preset instruction reaches the provider's system
  message; unknown id sends without it; `retry` re-applies the
  stored preset; the user row persists a resolved id only.
- `lib.rs`: the contract test asserts the palette commands are
  registered and the `EV_*` consts declared; the palette commands
  carry the Gate::Main check.
- `layout.rs`: `palette_rect` — below/above by free space, left-edge
  anchor tracking, work-area clamp.
- `bun test` (`apps/native`): `lib/presets.test.ts` — the `/`-token
  splitter/matcher, eager-vs-send resolution, expansion, `langName`,
  `stripSlashToken`.
- Manual (`bun run build:dev`): wand opens the focused palette in
  pill + card; `/` opens the unfocused palette and live-filters;
  ↑↓/Enter/Tab/Esc forward correctly; badge arm/✕/Esc/caret-0
  Backspace; `{lang}` badge edit + send; `{input}` preset expands
  into the sent bubble; provenance `· name` on the user row; retry
  keeps the instruction (and falls back on `{lang}`); custom preset
  CRUD round-trips through `config.toml`; right-click opens the
  palette over a composer, the shared menu elsewhere.

## Out of scope

- Pinned/session presets; preset hotkeys; ordering/folders;
  sharing/export (Phase 3 skill bundles); `marvis://` preset params.
- A persisted `preset_lang`: `retry` re-arms the preset but the
  edited `{lang}` is not stored — the fallback is the configured
  main language.
- Kind grouping in the picker (no kind exists) and a native-menu
  preset menu — both superseded by the palette + badge design.
- Editing built-ins (a custom can shadow a built-in's name but the
  built-in still resolves its id); template expansion server-side
  (it is webview-side by design).
