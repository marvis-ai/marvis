# Marvis Native — Unified Morphing Bar Window

Date: 2026-09-22
Status: Approved design (pending written-spec review)

## Goal

Merge the `bar`, `ask`, and `listen` windows into ONE window: the bar itself
expands into the chatbox. Today asking a question slides a separate `ask`
window out from under the bar — the "bump out a new window" feel this spec
removes.

## User decisions

| Question | Decision |
| --- | --- |
| Merge approach | One resizable bar window (Approach A) — panel windows deleted entirely |
| Idle window size | 480×64 (was 441×59); pill capsule ⇄ input morph inside unchanged |
| Expanded size | 600×(64 + content_h) |
| Listen scope | Structure only — Listen UI merges as a bar mode, still the Phase-2 stub; no dictation/STT built |
| `Cmd+/` (`toggle_visibility`) | Toggles chat open/closed; the bar window itself always stays visible |
| Chat model | True multi-turn — the LLM receives prior `ai_messages` as context; conversation persists in the active `ask` session |

## Window model

| Mode | Window size | Contents |
| --- | --- | --- |
| Idle / input / permission | 480×64 | Pill only — capsule ⇄ input morph unchanged |
| Chat | 600×(64 + content_h) | Bar row pinned at the docked edge + scrollable conversation |
| Listen | 600×(64 + content_h) | Bar row + Phase-2 waveform stub |

The `alert` toast (340×100) and `prefs` (720×520, decorated) windows are
unchanged. The bar stays `resizable(false)` — all resizes are programmatic,
animated by `windows/movement.rs` (180 ms).

## Expansion direction & anchoring

- Direction = larger free side: `space_below = work.bottom − bar.bottom` vs
  `space_above = bar.y − work.y`; grow toward the larger. Top-docked → down,
  bottom-docked → up.
- Grow down: `y` fixed, bar row at window top. Grow up: `bottom` fixed
  (`y = bottom − h`); the DOM stage uses `column-reverse` so the bar row sits
  at the window's bottom edge and never visually jumps.
- Width 480→600 recenters on the bar's center-x, clamped to the work area.
- Collapse is the exact reverse; the pill recenters on the card's center-x and
  re-anchors at the grow edge.
- `bar_rect` remains the canonical PILL rect — the source of truth for
  position, edge detection, and `Moved`-debounce persistence. The expanded
  rect is derived (`expanded_rect = pill anchor + dir + chat_height`). While
  expanded, `refresh_bar_rect` derives the pill rect back from the live window
  rect via the stored `expand_dir`, so persistence never records card
  geometry.
- Total window height clamp: `min(900, available space in expand direction)`;
  minimum chat content height 40 px (window ≥ 104 px).
- The alert toast anchors under the expanded card when chat is open, under the
  pill otherwise.

## Rust changes

### `windows/mod.rs` — WindowPool slims down

Deleted: `Panel` enum, `panels`, `visible`, `remembered`, `heights`,
`layout_targets`, `restack`, `create_feature_windows`, `toggle_all`,
`Panel::from_label`. `windows/layout.rs` shrinks to `clamp_to_work_area` +
`snap_edge` (all `panel_rects` code and tests die with it).

Added fields: `chat_open: bool`, `chat_height: f64` (last reported content
height, default 480), `expand_dir: Dir` (Up/Down, computed at expand time).

New methods:

| Method | Behavior |
| --- | --- |
| `set_chat_open(app, open)` | Animates the bar rect collapsed ⇄ expanded per §Expansion; emits nothing — the webview learns the mode from `ask:*`/its own action. No-op when gate ≠ `Main` (mirrors panels not existing pre-`Main`) |
| `toggle_chat()` | `set_chat_open(!chat_open)` |
| `adjust_height(px)` | Replaces the panel version: `px` is the desired TOTAL window height (frontend measures the whole card); clamps to `[64 + 40, min(900, free space)]`, animates the expanded window keeping the anchored edge fixed |

`snap_edge` / `recenter_bar` / `reclamp` keep operating on `bar_rect` (the
pill rect) but animate the window to the DERIVED rect — an expanded card
moves as a unit when the bar is snapped or recentered.

### `lib.rs`

| Change | Detail |
| --- | --- |
| `enter_main` | Drops `create_feature_windows` — the bar always exists |
| `leave_main` | `ask.close` (now also collapses chat) + hide alert; panel loop gone |
| `hotkey_dispatch` | `ToggleVisibility` → `pool.toggle_chat()` |
| `window_toggle_all` | Same `toggle_chat` |
| `window_adjust_height` | Signature drops `name` → `(height: f64)`; calls `pool.adjust_height` |
| `session_end_active` | NEW command `(kind: String)` → `db.session_end(active_id)` — powers "New chat" |
| `ask_current` | NEW command → `{state, question, response}` from `AskService` so a re-expanded chat resyncs mid-stream |

Gate semantics unchanged: `Main` still requires `onboarding_done` && screen
permission; ask commands still gate-guarded.

### `ask.rs`

| Change | Detail |
| --- | --- |
| `kick` pre-flight | `pool.show(Panel::Ask)` → `pool.set_chat_open(&app, true)` |
| Event target | every `emit_to("ask", …)` → `emit_to("bar", …)`; event names unchanged (`ask:state`/`ask:chunk`/`ask:done`/`ask:error`) |
| `close` | `pool.hide(Panel::Ask)` → `pool.set_chat_open(false)` |
| Busy rule | unchanged — a send while Loading/Streaming is warn-logged and ignored |

### Multi-turn in `send_chain`

1. `sid = session_get_or_create_active("ask")`
2. `history = ai_messages_for(sid)`, capped to the last **20 messages** →
   text-only `ChatMessage`s (user/assistant roles only)
3. Persist the new user message (`ai_message_add`)
4. `messages = [system] + history + [user (+latest frame)]`
5. Stream through the failover chain → persist assistant reply

Only the current user turn carries an image; history is text-only (images
were never persisted anyway). `send_screen_only` unchanged (frame required).

## Frontend changes

### `App.tsx`

Routes shrink to `bar` (default) / `alert` / `prefs`; `?view=ask` and
`?view=listen` deleted.

### `Bar.tsx` becomes the shell

```text
<div stage data-pos={edge} data-dir={up|down}>
  <div card>            // column (grow-down) / column-reverse (grow-up)
    <BarRow/>           // existing pill internals — iris, input, camera,
                        // mic, settings; stays live as the card header
    {mode === 'chat'   && <ChatSection/>}
    {mode === 'listen' && <ListenSection/>}
  </div>
</div>
```

- `mode`: `mini | input | permission | chat | listen`, driven by gate + the
  existing open/text state + `ask:*` events (`ask:state{loading}` → chat).
- Bar row stays interactive in chat mode — typing + submit = follow-up ask.
- Esc in chat mode → `ask_close` (collapse). Mic button becomes enabled and
  opens `listen` mode showing the stub.
- `capture:permission-needed` → collapse chat + permission card (existing
  handler extended).

### `src/components/ChatSection.tsx` (from `views/AskPanel.tsx`)

- On expand: `session_list` → active `ask` session → `session_get` → render
  history; then `ask_current` to resync any in-flight run.
- Live run: `ask:state{loading}` appends the user bubble and resets the
  stream buffer; `ask:chunk` appends to the last assistant bubble;
  `ask:done` writes `full` authoritatively + updates the model chip;
  `ask:error` renders the error row (+ "Open settings" when `needs_setup`).
- Markdown (`ReactMarkdown` + `remarkGfm`), autoscroll-with-pin, caret, and
  the height-reporting ResizeObserver all carry over; the observer now
  measures the WHOLE card and reports total window height via
  `window_adjust_height`.
- Header gains "New chat" (`session_end_active("ask")` + clear local list)
  and close (`ask_close` → collapse).

### `src/components/ListenSection.tsx` (from `views/ListenPanel.tsx`)

The five-bar waveform + "Listen arrives in Phase 2" + `deepgram · stt` chip,
unchanged visually.

### Deleted

`views/AskPanel.tsx`, `views/ListenPanel.tsx` (as routes — internals move to
`src/components/`), and every `Panel`-related symbol on the Rust side.

## Ask data flow (after)

```text
user types in BarRow → askSend(text)
  → Rust ask_send: gate==Main guard → AskService.kick
      → pool.set_chat_open(true)            // window animates 480×64 → 600×(64+h)
      → provider_candidates (failover chain)
      → ring.latest() frame (dropped + warn if permission revoked)
      → emit_to("bar", ask:state{loading, question})
      → spawn: sid=get_or_create_active("ask")
              history = ai_messages_for(sid) last 20 (text-only)
              persist user msg
              stream candidates → ask:chunk* → ask:done{full,provider,model}
              persist assistant msg
  → Webview ChatSection: user bubble + streaming reply; ResizeObserver
    reports card height → window grows; autoscroll pinned
```

## Testing

- Rust unit tests: expand/collapse anchor math (grow-down/grow-up, work-area
  clamps, width recenter), capped-history message building (20-msg tail,
  image only on latest user turn), `session_end_active`.
- Panel-layout tests deleted with `panel_rects`; existing `lib.rs` provider-
  chain and `ask.rs` chain tests updated for the new emit target/history.
- Verify: `cargo test`, `cargo clippy`, `bun run check-types` + `bun run build`
  in `apps/native`.
- Manual: capsule ⇄ input morph, ask → expand down (top-docked) and up
  (bottom-docked), Cmd+/ toggle, New chat clears context, listen stub mode,
  permission-revocation path.

## Out of scope

- Real dictation / STT (Phase 2)
- `alert` and `prefs` window changes
- Landing page (`apps/web`) — the `.mv-*` mock views there stay as-is
