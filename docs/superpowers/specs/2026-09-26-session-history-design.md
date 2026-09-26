# Marvis Native — Session History (Chat + Listen)

Date: 2026-09-26
Status: Draft (pending written-spec review)
Branch: feature/redesign_listening_with_summary

## Goal

Give both card surfaces a reachable past: a unified session history as a
third card mode, one click from idle. Tapping a chat row RESUMES that
conversation (it becomes the active `ask` session); tapping a listen row
opens a read-only transcript + summary document.

Step budget from idle:

| Target | Steps |
| --- | --- |
| History list | 1 (new capsule icon) |
| A specific past chat / meeting | 2 |
| Switch sections inside the card | 1 (segmented tabs) |

## User decisions

| Question | Decision |
| --- | --- |
| History location | Third card mode (Chat | Listen | History), NOT per-section sub-views |
| Idle entry | 4th capsule icon (clock) — capsule widens to fit |
| Past chat session | Resumable — reopening makes it the active `ask` session |
| Past listen session | Read-only transcript + summary (listen resume is meaningless — `listen_start` always mints a fresh session) |
| Session titles | Computed in the `session_list` query (COALESCE) — no write-path changes, back-fills old rows |

## Section state machine (Bar.tsx)

`listenWanted` becomes the unpinned default; an explicit user choice pins:

```ts
const [pinned, setPinned] = useState<'chat' | 'listen' | 'history' | null>(null);
const section = !cardOpen ? null
  : pinned ?? (listenWanted ? 'listen' : 'chat');
```

- **Pin set by**: capsule history icon → `'history'`; capsule mic → `'listen'`
  (existing flow adds the pin); tab clicks; `submitAsk` while card open →
  `'chat'`; history row taps → the row's kind.
- **Pin cleared**: when `cardOpen` goes false (each card open starts in auto
  mode again).
- **Auto-flips unchanged**: `listen:state`/`listen:error` and `ask:state`
  still write `listenWanted` — they move the default, never override a pin.
  Rationale: a background transition shouldn't yank a user who deliberately
  opened History; explicit actions always pin.

## Window / capsule changes

| Change | Detail |
| --- | --- |
| `BAR_IDLE_W` | 140 → ~172 px — four capsule controls (iris, capture, listen, history); `BAR_W` stays 600 |
| `barControls(false)` | adds `'history'` to the collapsed set (`lib/bar-state.ts`) |
| History capsule icon | `HistoryIcon` (lucide) — `setPinned('history')` + `windowSetChatOpen(true)`; disabled when `gate !== 'main'` like its siblings |

## Card chrome — `CardTabs`

A slim segmented strip (~32 px, `border-b`) rendered in card mode only:
`Chat | Listen | History`, icon + label per tab. Visually it sits at the
top of the card above the section content — under the card's
`flex-col-reverse` that means it is the LAST DOM child (after the section).
The Listen tab shows a live dot while `listenState === 'listening'` so the
running session stays discoverable from History/Chat. Per-section headers
are unchanged.

## `HistorySection` (new `src/components/HistorySection.tsx`)

- Mount: `sessionList()` → rows sorted `last_active_at` DESC (backend
  already orders). Refetch on `listen:state` transitions while mounted so a
  newly-started meeting appears without a remount.
- Row: kind icon (waveform / chat bubble), title, relative time
  (`<24h` → "2h ago", else date), `Live` badge when `ended_at === null`.
- Row tap:
  - `kind === 'ask'` → `sessionResume(id)` → `setPinned('chat')`. Disabled
    while `askState !== 'idle'` (an in-flight run belongs to the current
    session — resuming mid-stream would mis-attribute it).
  - `kind === 'listen'` → ended row: `setListenViewing(id)` +
    `setPinned('listen')`; the LIVE row just pins `'listen'` (live mode,
    no viewing id).
- Row delete: hover affordance → `sessionDelete(id)` + local removal.
  Disabled on the live row.
- Empty state: "No history yet — ask Marvis or start listening."

## `ListenSection` — viewing mode

New prop `viewing: { id, startedAt } | null` (cleared by pinning `'listen'`
or closing the card). When set:

- Loads `transcriptsFor(id)` + `summaryLatest(id)` instead of the live
  status path; skips all live-event subscription effects.
- Same block/summary rendering; header shows the session's start time
  instead of the waveform + Stop button; mic-warning and error rows hidden.

## `ChatSection` — unchanged

The existing mount resync (`session_list` → active `ask` → `session_get` +
`ask_current` fold) already loads whichever session is active — resume +
tab-pin remounts it onto the reopened session. "New chat"
(`session_end_active('ask')`) still works: it ends the resumed session and
the next send mints a fresh one.

## Rust changes

### `storage.rs`

| Change | Detail |
| --- | --- |
| `session_list` SQL | `title` becomes `COALESCE(s.title, CASE s.type WHEN 'ask' THEN (SELECT substr(m.content,1,60) FROM messages m WHERE m.session_id=s.id AND m.role='user' ORDER BY m.ts LIMIT 1) WHEN 'listen' THEN (SELECT sm.topic FROM summaries sm WHERE sm.session_id=s.id AND sm.topic IS NOT NULL ORDER BY sm.ts DESC LIMIT 1) END)` — still `NULL` only when a session has neither messages nor a titled summary; the UI falls back to "Chat"/"Meeting" + time |
| `session_reopen(id)` | `UPDATE sessions SET ended_at = NULL, last_active_at = now WHERE id = ?` |

### `lib.rs`

| Command | Behavior |
| --- | --- |
| `session_resume(id)` | Session must exist with `type='ask'` → `session_end_open('ask')` + `session_reopen(id)` → `true`; anything else → `false`. The reopened row is then what `session_get_or_create_active('ask')` returns, so `ask_send` appends to it |

### `commands.ts`

`sessionResume(id: number)` → `invoke<boolean>('session_resume', { id })`.
`Session.title` is already `string | null` — no type change.

## Data flow — resuming a chat

```text
tap chat row (idle askState)
  → sessionResume(id): end open 'ask' sessions; reopen selected
  → setPinned('chat') → ChatSection remounts
  → mount resync: session_list finds reopened session active
    → session_get loads the thread; ask_current folds any live tail
  → next askSend → session_get_or_create_active('ask') → reopened row
    → user/assistant messages append to the SAME session
```

## Testing

- Rust: `session_reopen` clears `ended_at` + bumps `last_active_at`;
  `session_resume` ends other open `ask` sessions and rejects non-`ask`
  ids; `session_list` computed titles (first user message / latest topic /
  NULL fallback).
- Verify: `cargo test`, `cargo clippy`, `bun run check-types` +
  `bun run build` in `apps/native`.
- Manual: 4-icon capsule layout at rest; idle→history in 1 click; tabs
  switch all three sections; resume a past chat and continue it; open a
  past meeting read-only; live badge + live row tap; delete a row;
  Esc/collapse resets the pin.

## Out of scope

- Search, kind filters, pagination (sessions are few; revisit if the list
  outgrows a screenful regularly)
- Listen session resume (each listen run is its own document by design)
- Title editing / pinning sessions
- Bulk delete
- Any second window — History lives in the unified card
