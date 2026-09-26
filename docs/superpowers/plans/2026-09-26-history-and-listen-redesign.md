# Session History + Listen Section Redesign — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A third card mode (History) reachable in 1 click from idle, resumable chat sessions, a read-only listen document for past/stopped sessions, and a structured live-listen UI (header + timer + pause + speaker filter + copy + pinned summary strip).

**Architecture:** Backend gains computed session titles, `session_reopen`, and a soft-pause (`paused` flag drops audio chunks — sources stay up). Frontend: `Bar.tsx` gains a `pinned` section state + `listenViewing` id; `ListenSection` is rebuilt as a document view under `src/components/listen/`; `HistorySection` is new.

**Tech Stack:** Tauri v2 (Rust), React + Tailwind, rusqlite, lucide-react via `@marvis/ui` barrel.

## Global Constraints

- Specs: `docs/superpowers/specs/2026-09-26-session-history-design.md` and `2026-09-26-listen-section-redesign-design.md`.
- Icons: `Icon`-suffixed lucide names exported through `packages/ui/src/index.ts` only (AGENTS.md §9).
- Components: arrow functions, named exports (AGENTS.md §2-3).
- All window geometry in logical pixels; the capsule IS the window (`BAR_IDLE_W` × 64).
- `bar_rect` stays the canonical pill — no card geometry in persistence.
- Session kinds: `'ask'` | `'listen'`. Chat resumes mutate the session row; listen sessions never reopen.
- Verify per task: `cargo test` / `cargo clippy` in `apps/native/src-tauri`, `bun run check-types` + `bun run build` in `apps/native`.

---

### Task 1: Storage — computed titles + `session_reopen` + `session_started_at`

**Files:**

- Modify: `apps/native/src-tauri/src/storage.rs` (session_list ~L225, session impl block)

**Interfaces:**

- Produces: `Db::session_reopen(&self, id: i64, kind: &str) -> anyhow::Result<bool>` — atomically ends every OTHER open session of `kind` and clears the target's `ended_at`; `false` when `id` isn't a `kind` session. `Db::session_started_at(&self, id: i64) -> anyhow::Result<Option<i64>>`. `Session.title` now COALESCE-computed (ask → first user message prefix; listen → latest summary topic).

- [ ] **Step 1: Failing tests** — append inside `#[cfg(test)] mod tests` in storage.rs:

```rust
#[test]
fn session_list_computes_titles() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let ask = db.session_get_or_create_active("ask").unwrap();
    db.message_add(ask, "user", "how do I fix the capsule width").unwrap();
    let listen = db.session_get_or_create_active("listen").unwrap();
    db.summary_add(listen, "tldr", &["b".to_string()], &[], Some("release review")).unwrap();
    let bare = db.session_get_or_create_active("ask").unwrap_or_else(|_| {
        db.session_end(ask).unwrap();
        db.session_get_or_create_active("ask").unwrap()
    });
    let list = db.session_list().unwrap();
    let a = list.iter().find(|s| s.id == ask).unwrap();
    assert_eq!(a.title.as_deref(), Some("how do I fix the capsule width"));
    let l = list.iter().find(|s| s.id == listen).unwrap();
    assert_eq!(l.title.as_deref(), Some("release review"));
    let _ = bare; // an empty session keeps title NULL — UI falls back
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn session_reopen_swaps_the_active_ask_session() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let a = db.session_get_or_create_active("ask").unwrap();
    let b = { db.session_end(a).unwrap(); db.session_get_or_create_active("ask").unwrap() };
    // Reopening a listen id is rejected and ends nothing.
    let l = db.session_get_or_create_active("listen").unwrap();
    assert!(!db.session_reopen(l, "ask").unwrap());
    assert_eq!(db.session_active_id("ask").unwrap(), Some(b));
    // Real resume: b ends, a reopens and becomes the active ask session.
    assert!(db.session_reopen(a, "ask").unwrap());
    assert_eq!(db.session_active_id("ask").unwrap(), Some(a));
    assert_eq!(db.session_active_id("listen").unwrap(), Some(l));
    let _ = std::fs::remove_dir_all(&dir);
}
```

Check `summary_add`'s real signature first (`grep -n "fn summary_add" storage.rs`) — adapt the test args if the param order differs.

- [ ] **Step 2: Run tests, confirm they fail**

Run: `cd apps/native/src-tauri && cargo test session_`
Expected: FAIL — `session_reopen`/`session_started_at` don't exist; titles are NULL.

- [ ] **Step 3: Implement**

In `impl Db`:

```rust
/// `session_list` title: COALESCE(stored title, first ask user message,
/// latest listen summary topic) — NULL only for content-less sessions.
pub fn session_list(&self) -> anyhow::Result<Vec<Session>> {
    let conn = self.conn.lock();
    let mut stmt = conn.prepare(
        "SELECT s.id, s.type,
                COALESCE(s.title,
                  CASE s.type
                    WHEN 'ask' THEN (
                      SELECT substr(m.content, 1, 60) FROM messages m
                      WHERE m.session_id = s.id AND m.role = 'user'
                      ORDER BY m.ts ASC, m.id ASC LIMIT 1)
                    WHEN 'listen' THEN (
                      SELECT sm.topic FROM summaries sm
                      WHERE sm.session_id = s.id AND sm.topic IS NOT NULL
                      ORDER BY sm.ts DESC, sm.id DESC LIMIT 1)
                  END),
                s.started_at, s.ended_at, s.last_active_at
         FROM sessions s ORDER BY s.last_active_at DESC, s.id DESC",
    )?;
    // …row mapping unchanged…
}

/// Reopen `id` when it is a `kind` session: every OTHER open `kind`
/// session ends and the target's `ended_at` clears, atomically. Returns
/// false (no mutation) when `id` isn't a `kind` session.
pub fn session_reopen(&self, id: i64, kind: &str) -> anyhow::Result<bool> {
    let mut conn = self.conn.lock();
    let tx = conn.transaction()?;
    let matches: bool = tx
        .query_row(
            "SELECT type = ?2 FROM sessions WHERE id = ?1",
            params![id, kind],
            |row| row.get(0),
        )
        .optional_extension()?
        .unwrap_or(false);
    if !matches {
        return Ok(false);
    }
    tx.execute(
        "UPDATE sessions SET ended_at = ?1 WHERE type = ?2 AND ended_at IS NULL AND id != ?3",
        params![now(), kind, id],
    )?;
    tx.execute(
        "UPDATE sessions SET ended_at = NULL, last_active_at = ?1 WHERE id = ?2",
        params![now(), id],
    )?;
    tx.commit()?;
    Ok(true)
}

/// Session start time — the elapsed-timer epoch for Listen.
pub fn session_started_at(&self, id: i64) -> anyhow::Result<Option<i64>> {
    Ok(self
        .conn
        .lock()
        .query_row("SELECT started_at FROM sessions WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .optional_extension()?)
}
```

Ensure `use rusqlite::OptionalExtension;` is present at the top (it's used by `session_active_id` already — check before adding).

- [ ] **Step 4: Run tests**

Run: `cd apps/native/src-tauri && cargo test session_`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/native/src-tauri/src/storage.rs
git commit -m "feat(native): computed session titles + atomic session_reopen"
```

---

### Task 2: `session_resume` command

**Files:**

- Modify: `apps/native/src-tauri/src/lib.rs` (sessions command block ~L1645, `invoke_handler` list ~L2067, contract test ~L2116)

**Interfaces:**

- Consumes: `Db::session_reopen` (Task 1).
- Produces: Tauri command `session_resume(id: i64) -> Result<bool, String>` — true when `id` became the active ask session.

- [ ] **Step 1: Implement command + registration**

After `session_end_active` in lib.rs:

```rust
/// Resume a past chat: ends the open `ask` session and reopens `id`
/// (`session_reopen` kind-guards — non-ask ids change nothing and
/// return false). The next `ask_send` appends to the reopened session.
#[tauri::command]
fn session_resume(state: State<'_, AppState>, id: i64) -> Result<bool, String> {
    state.db.session_reopen(id, "ask").map_err(|e| e.to_string())
}
```

Add `session_resume,` to the `tauri::generate_handler![…]` list near `session_end_active,`. In the `removed_listen_placeholder_is_absent_from_command_contract` test, add `assert!(source.contains("session_resume,"));`.

- [ ] **Step 2: Verify**

Run: `cd apps/native/src-tauri && cargo test && cargo clippy`
Expected: PASS, no warnings.

- [ ] **Step 3: Commit**

```bash
git add apps/native/src-tauri/src/lib.rs
git commit -m "feat(native): session_resume command for chat history"
```

---

### Task 3: Listen soft-pause + status timer fields

**Files:**

- Modify: `apps/native/src-tauri/src/listen.rs` (ListenStatus ~L285, `Running` ~L323, `start` ~L363, `stop` ~L645, tests ~L926)
- Modify: `apps/native/src-tauri/src/lib.rs` (`emit_listen_state` ~L1043, commands ~L1124, handler list, contract test)

**Interfaces:**

- Consumes: `Db::session_started_at` (Task 1).
- Produces: `ListenStatus` gains `started_at: Option<i64>`, `paused_secs: i64`, `paused_since: Option<i64>`; `state` may be `"paused"`; `ListenService::pause()/resume() -> Option<ListenStatus>`; commands `listen_pause`, `listen_resume`.

- [ ] **Step 1: Update failing wire-keys test first** — in `listen_status_serializes_documented_wire_field_names`, extend the status literal with `started_at: Some(100), paused_secs: 0, paused_since: None` and the expected keys to `["error","mic","paused_secs","paused_since","provider","session_id","started_at","state","turns"]`. Also add a pause/resume unit test:

```rust
#[test]
fn pause_freezes_and_resume_accumulates() {
    let svc = ListenService::new();
    // No running session → both are no-ops.
    assert!(svc.pause().is_none());
    assert!(svc.resume().is_none());
}
```

(Deeper pause coverage needs a running session — covered manually; the unit surface is the no-op path + wire shape.)

- [ ] **Step 2: Run test, confirm new-keys assertion fails**

Run: `cd apps/native/src-tauri && cargo test listen_status_serializes`
Expected: FAIL — literal lacks the new fields.

- [ ] **Step 3: Implement**

`ListenStatus` struct:

```rust
pub struct ListenStatus {
    pub state: String,
    pub provider: Option<String>,
    pub session_id: Option<i64>,
    pub turns: usize,
    pub mic: bool,
    pub error: Option<ListenError>,
    /// Session start epoch — the header timer's zero point.
    pub started_at: Option<i64>,
    /// Accumulated pause seconds; `paused_since` is Some while paused.
    pub paused_secs: i64,
    pub paused_since: Option<i64>,
}
```

`is_listening()`: `self.state == "listening" || self.state == "paused"`.

`Running` gains `paused: Arc<AtomicBool>`.

In `start()`: create `let paused = Arc::new(AtomicBool::new(false));` before `add`; inside the worker closure capture `let paused_rx = paused.clone();` and change the `Ok(chunk)` arm:

```rust
Ok(chunk) => {
    // Soft-pause: drain the source but starve STT — nothing transcribes
    // and nothing persists while paused.
    if paused_rx.load(Ordering::Acquire) {
        continue;
    }
    if !stt.enqueue(chunk) { /* existing dropped_chunks log */ }
}
```

Populate `status.started_at` at the end of `start()`:

```rust
status.started_at = db.session_started_at(session_id).ok().flatten();
```

All `ListenStatus` literals (idle/error paths, `revalidate_setup`, `stop`) gain `started_at: None, paused_secs: 0, paused_since: None` — but `stop()`/`start()` should preserve nothing; fresh literals are correct. For the error-mid-run path inside `stt.start` error callback (~L487): keep `started_at`/`paused_*` — change that status mutation to field updates only (it already mutates `status.state`/`status.error` in place — no literal there, verify).

New methods on `ListenService`:

```rust
/// Pause without tearing down sources: workers keep draining (and
/// dropping) chunks so resume is instant. The open turn flushes first —
/// a pause is a clean transcript boundary.
pub fn pause(&self) -> Option<ListenStatus> {
    let running = self.running.lock();
    let running = running.as_ref()?;
    if running.paused.swap(true, Ordering::AcqRel) {
        return Some(self.status());
    }
    for turn in running.assembler.lock().flush() {
        persist_turn(&running.context, turn);
    }
    let mut status = self.state.lock();
    status.state = "paused".into();
    status.paused_since = Some(now_unix());
    Some(status.clone())
}

pub fn resume(&self) -> Option<ListenStatus> {
    let running = self.running.lock();
    let running = running.as_ref()?;
    if !running.paused.swap(false, Ordering::AcqRel) {
        return Some(self.status());
    }
    let mut status = self.state.lock();
    if let Some(since) = status.paused_since.take() {
        status.paused_secs += now_unix() - since;
    }
    status.state = "listening".into();
    Some(status.clone())
}
```

lib.rs `emit_listen_state` json gains:

```rust
"started_at": state.started_at,
"paused_secs": state.paused_secs,
"paused_since": state.paused_since,
```

New commands after `listen_stop`:

```rust
#[tauri::command]
fn listen_pause(app: AppHandle) {
    let state = app.state::<AppState>();
    if let Some(status) = state.listen.pause() {
        emit_listen_state(&app, &status);
    }
}

#[tauri::command]
fn listen_resume(app: AppHandle) {
    let state = app.state::<AppState>();
    if let Some(status) = state.listen.resume() {
        emit_listen_state(&app, &status);
    }
}
```

Register both in `generate_handler!`; add `assert!(source.contains("listen_pause,")); assert!(source.contains("listen_resume,"));` to the contract test.

- [ ] **Step 4: Verify**

Run: `cd apps/native/src-tauri && cargo test && cargo clippy`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/native/src-tauri/src/listen.rs apps/native/src-tauri/src/lib.rs
git commit -m "feat(native): listen soft-pause + session timer fields"
```

---

### Task 4: Frontend plumbing — commands, event payloads, icon barrel

**Files:**

- Modify: `apps/native/src/lib/commands.ts` (`ListenStatus` ~L260, listen block ~L305, sessions block ~L509)
- Modify: `apps/native/src/lib/events.ts` (`ListenStatePayload` ~L45)
- Modify: `packages/ui/src/index.ts` (icon export list ~L25)

**Interfaces:**

- Produces: `listenPause()`, `listenResume()`, `sessionResume(id)`; `ListenStatus.state` includes `'paused'` + timer fields; `ListenStatePayload` mirrors the wire; icons `HistoryIcon`, `PauseIcon`, `PlayIcon`, `SquareIcon`, `CopyIcon`, `ChevronDownIcon`, `ChevronUpIcon`, `Trash2Icon`, `MessageSquareTextIcon`.

- [ ] **Step 1: commands.ts**

```ts
export interface ListenStatus {
  state: 'idle' | 'listening' | 'paused' | 'error';
  provider: string | null;
  session_id: number | null;
  turns: number;
  mic: boolean;
  error: ListenErrorPayload | null;
  /** Session start epoch — elapsed = (paused_since ?? now) - started_at - paused_secs. */
  started_at: number | null;
  paused_secs: number;
  paused_since: number | null;
}

export const listenPause = () => invoke<void>('listen_pause');
export const listenResume = () => invoke<void>('listen_resume');

/** Resume a past chat session — ends the open one, reopens `id`. */
export const sessionResume = (id: number) =>
  invoke<boolean>('session_resume', { id });
```

- [ ] **Step 2: events.ts** — `ListenStatePayload` gains the same additions:

```ts
export interface ListenStatePayload {
  state: 'idle' | 'listening' | 'paused' | 'error';
  provider: string | null;
  session_id: number | null;
  mic: boolean;
  error: ListenErrorPayload | null;
  started_at: number | null;
  paused_secs: number;
  paused_since: number | null;
}
```

- [ ] **Step 3: `useBarActivity.ts`** — treat `paused` as a live listen session: in the `EV_LISTEN_STATE` handler and the mount status read, use `p.state === 'listening' || p.state === 'paused'` wherever `'listening'` currently drives `setListenWanted(true)`. `idle` → `setListenWanted(false)` unchanged.

- [ ] **Step 4: packages/ui icon barrel** — add to the existing `export { … } from "lucide-react"` (alphabetical):

```ts
CheckIcon,          // already present
ChevronDownIcon,
ChevronUpIcon,
CopyIcon,
HistoryIcon,
MessageSquareTextIcon,
PauseIcon,
PlayIcon,
SquareIcon,
Trash2Icon,
```

- [ ] **Step 5: Verify + commit**

Run: `cd apps/native && bun run check-types && bun run build`
Expected: PASS (may surface unused-import errors elsewhere — fix minimally).

```bash
git add apps/native/src/lib apps/native/src/hooks packages/ui/src/index.ts
git commit -m "feat(native): pause/resume + session_resume client plumbing"
```

---

### Task 5: `listen/` module + `ListenSection` document rebuild

**Files:**

- Create: `apps/native/src/components/listen/model.ts`
- Create: `apps/native/src/components/listen/ListenHeader.tsx`
- Create: `apps/native/src/components/listen/SpeakerFilter.tsx`
- Create: `apps/native/src/components/listen/TranscriptBlocks.tsx`
- Modify: `apps/native/src/components/ListenSection.tsx` (rebuilt)
- Modify: `apps/native/src/views/Bar.tsx` (`pinned` machine + `listenViewing` + props)

**Interfaces:**

- Produces:
  - `model.ts`: `Turn`, `TurnIdentity`, `TurnBlock`, `ListenViewing {id, startedAt, endedAt}`, `speakerKey`, `speakerName`, `speakerColor`, `elapsedLabel(secs)`, `blockCopyText(block)`, `transcriptCopyText(blocks, summary?)`, `buildBlocks(turns)`.
  - `ListenSection({ viewing, onSessionEnded }: { viewing: ListenViewing | null; onSessionEnded: (v: ListenViewing) => void })`.
  - Bar: `pinned: 'chat'|'listen'|'history'|null`, `listenViewing`, `setPinned`, `setListenViewing`.

- [ ] **Step 1: `model.ts`** — move `speakerKey`/`speakerName`/`speakerColor`/`SPEAKER_COLOR_CLASSES`/`TurnBlock`/`blocks` logic out of the old `ListenSection.tsx`, plus:

```ts
export type Turn = ListenTurnPayload & { interim?: boolean };

export interface ListenViewing {
  id: number;
  startedAt: number;
  endedAt: number | null;
}

/** `m:ss`, uncapped minutes (mockup's 107:36). */
export const elapsedLabel = (secs: number) => {
  const s = Math.max(0, Math.floor(secs));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
};

export const blockText = (b: TurnBlock) =>
  b.finals.map((t) => t.text).join(' ') +
  (b.interim ? (b.finals.length ? ' ' : '') + b.interim.text : '');

export const blockCopyText = (b: TurnBlock) => `${b.name}: ${blockText(b)}`;

/** Whole-document copy: `[m:ss] Name: text` lines + a summary tail. */
export const transcriptCopyText = (
  blocks: TurnBlock[],
  startedAt: number | null,
  summary: ListenSummaryPayload | null,
) => {
  const lines = blocks.map((b) => {
    const stamp =
      startedAt != null ? elapsedLabel(b.ts - startedAt) : timeLabel(b.ts);
    return `[${stamp}] ${b.name}: ${blockText(b)}`;
  });
  if (summary) {
    lines.push('---', `TLDR: ${summary.tldr}`, ...summary.bullets.map((b) => `- ${b}`));
  }
  return lines.join('\n');
};

export const relTime = (ts: number) => {
  const diff = Date.now() / 1000 - ts;
  if (diff < 60) return 'now';
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  const d = new Date(ts * 1000);
  return `${d.toLocaleString('en', { month: 'short' })} ${d.getDate()}`;
};
```

(`timeLabel`/`buildBlocks` move over as-is; `buildBlocks` is the existing `blocks` useMemo body as a pure function.)

- [ ] **Step 2: `ListenHeader.tsx`**

```tsx
export const ListenHeader = ({
  title, subtitle, badge, elapsedSecs, live, listening, paused,
  copiedAll, onPause, onResume, onStop, onCopyAll,
}: {
  title: string; subtitle: string;
  badge: 'LISTENING' | 'PAUSED' | 'STOPPED' | null;
  elapsedSecs: number; live: boolean;
  listening: boolean; paused: boolean; copiedAll: boolean;
  onPause: () => void; onResume: () => void; onStop: () => void; onCopyAll: () => void;
}) => (
  <header className={PANEL_HEAD}>
    <div className='min-w-0 flex-1'>
      <p className='truncate text-xs font-[550] select-text'>{title}</p>
      <p className={cn(META, 'truncate text-[10.5px]')}>{subtitle}</p>
    </div>
    {badge && (
      <span className={cn(CHIP, badge === 'LISTENING' && 'border-accent/40 text-accent')}>
        {badge === 'LISTENING' && <i aria-hidden className='size-1.25 animate-capture-ping rounded-full bg-accent' />}
        {badge}
      </span>
    )}
    {badge && (
      <span className={cn(NUM, 'text-xs text-foreground')}>{elapsedLabel(elapsedSecs)}</span>
    )}
    {listening && (
      <button type='button' className={ICON_BTN} title='Pause' aria-label='Pause recording' onClick={onPause}>
        <PauseIcon className='size-3.5' />
      </button>
    )}
    {paused && (
      <button type='button' className={cn(ICON_BTN, 'text-accent')} title='Resume' aria-label='Resume recording' onClick={onResume}>
        <PlayIcon className='size-3.5' />
      </button>
    )}
    {(listening || paused) && (
      <button type='button' className={ICON_BTN} title='Stop' aria-label='Stop recording' onClick={onStop}>
        <SquareIcon className='size-3.5' />
      </button>
    )}
    <button type='button' className={ICON_BTN} title='Copy transcript' aria-label='Copy transcript' onClick={onCopyAll}>
      {copiedAll ? <CheckIcon className='size-3.5 text-accent' /> : <CopyIcon className='size-3.5' />}
    </button>
  </header>
);
```

- [ ] **Step 3: `SpeakerFilter.tsx`** — `all` chip + one chip per distinct block identity + right count:

```tsx
export const SpeakerFilter = ({
  speakers, active, count, onPick,
}: {
  speakers: { key: string; name: string; color: string }[];
  active: string | null; count: number;
  onPick: (key: string | null) => void;
}) => (
  <div className='flex items-center gap-1.5 border-b border-border px-3 py-1.75'>
    {[{ key: null as string | null, name: 'all', color: '' }, ...speakers].map((s) => (
      <button key={s.key ?? 'all'} type='button' onClick={() => onPick(s.key)}
        className={cn(CHIP, 'cursor-pointer lowercase transition-colors',
          active === s.key ? 'border-foreground/60 bg-fg-soft text-foreground' : 'hover:text-foreground')}>
        {s.key !== null && <i aria-hidden className={cn('size-1.5 rounded-full bg-current', s.color)} />}
        {s.name}
      </button>
    ))}
    <span className={cn(NUM, 'ml-auto text-[10px] text-muted-foreground')}>{count} rows</span>
  </div>
);
```

- [ ] **Step 4: `TranscriptBlocks.tsx`** — the block list with per-block `m:ss` and hover copy:

```tsx
export const TranscriptBlocks = ({ blocks, startedAt }: { blocks: TurnBlock[]; startedAt: number | null }) => {
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const copyBlock = (b: TurnBlock) => {
    void navigator.clipboard.writeText(blockCopyText(b)).then(() => {
      setCopiedKey(b.key);
      window.setTimeout(() => setCopiedKey((k) => (k === b.key ? null : k)), 1500);
    }).catch(() => {});
  };
  return (
    <>
      {blocks.map((block) => (
        <div key={`${block.key}-${block.ts}`} className='group/row relative mb-3.5'>
          <div className={cn('flex items-center gap-1.5', block.color)}>
            <span className={cn(NUM, 'w-8 flex-none text-[10px] text-muted-foreground')}>
              {startedAt != null ? elapsedLabel(block.ts - startedAt) : timeLabel(block.ts)}
            </span>
            <span aria-hidden className='size-1.75 flex-none rounded-full bg-current' />
            <span className='text-[12px] font-[650] tracking-[-0.005em]'>{block.name}</span>
            <button type='button' onClick={() => copyBlock(block)} aria-label={`Copy ${block.name}'s turn`}
              className={cn(ICON_BTN, 'ml-auto size-5 opacity-0 transition-opacity group-hover/row:opacity-100 focus-visible:opacity-100')}>
              {copiedKey === block.key ? <CheckIcon className='size-3 text-accent' /> : <CopyIcon className='size-3' />}
            </button>
          </div>
          <p className='mt-1 wrap-break-word whitespace-pre-wrap select-text pl-8'>
            {block.finals.map((turn) => turn.text).join(' ')}
            {block.interim && (
              <span className='text-muted-foreground'>
                {block.finals.length > 0 ? ' ' : ''}
                {block.interim.text}
                <span
                  aria-hidden
                  className='animate-caret ml-0.5 inline-block h-[0.95em] w-[1.5px] translate-y-[0.15em] bg-current'
                />
              </span>
            )}
          </p>
        </div>
      ))}
    </>
  );
};
```

`blockText`/`blockCopyText` join finals + interim into plain text for the
clipboard only; the visible paragraph keeps the dimmed interim + caret JSX
above (same styling as the old `ListenSection` render).

- [ ] **Step 5: Rebuild `ListenSection.tsx`** — same outer shape, new parts; props `( { viewing, onSessionEnded } )`:

  - `const live = viewing === null;`
  - Live path (existing effects: `listenStatus` resync, `EV_LISTEN_*` handlers, config/whisper reads) — guard every event handler and the mount-load so a live event never mutates a viewed document:

```ts
const viewingRef = useRef(viewing);
viewingRef.current = viewing;
// first line of each EV_LISTEN_* handler + the mount resync effect:
//   if (viewingRef.current) return;
```

- Viewing path:

```ts
useEffect(() => {
  if (!viewing) return;
  let cancelled = false;
  void Promise.all([transcriptsFor(viewing.id), summaryLatest(viewing.id)])
    .then(([rows, latest]) => {
      if (cancelled) return;
      setTurns(rows.map((r) => ({
        speaker: r.speaker, speaker_idx: r.speaker_idx, text: r.content,
        ts: r.ts, session_id: r.session_id, final: true,
      })));
      if (latest) setSummary(latest);
    })
    .catch(() => {});
  return () => { cancelled = true; };
}, [viewing?.id]);
```

- Elapsed (live): `startedAt = live ? status.started_at : viewing.startedAt`; tick `setInterval(1s)` only while `live && status.state === 'listening'`; `elapsed = (paused_since ?? now) - startedAt - paused_secs` (live) or `(endedAt ?? now) - startedAt` (viewing).
- Badge: `viewing ? 'STOPPED' : status.state === 'paused' ? 'PAUSED' : status.session_id != null && listening ? 'LISTENING' : null` — idle live shows no badge.
- Title: `summary?.topic ?? 'Listen'`; subtitle live: `${status.mic ? 'mic + system audio' : 'system audio'} · ${provider ?? 'stt'}${model ?` ${model}`: ''}`; viewing: session date `"Sep 26 · 14:32"` built from `startedAt`.
- `stop()` reads `status.session_id`/`status.started_at` BEFORE `listenStop()`, then calls `onSessionEnded({ id, startedAt, endedAt: nowUnix })` — Bar stores it as `listenViewing`.
- `copyAll`: `navigator.clipboard.writeText(transcriptCopyText(blocks, startedAt, summary))` + 1.5 s check.
- Filter: `const [filterKey, setFilterKey] = useState<string | null>(null)`; `speakers` deduped from blocks; `shown = filterKey ? blocks.filter(b => b.key === filterKey) : blocks`; reset on session/viewing change.
- Render order: `ListenHeader` → error row (live only) → mic-unavailable notice (live only) → `SpeakerFilter` → scroll (`TranscriptBlocks` + empty state + provider chips + Jump-to-live) → `SummaryStrip` slot (Task 6 adds the component; for now render the old inline summary block REMOVED — replace with nothing).

- [ ] **Step 6: `Bar.tsx` deltas**

```tsx
import type { ListenViewing } from '@/components/listen/model';

const [pinned, setPinned] = useState<'chat' | 'listen' | 'history' | null>(null);
const [listenViewing, setListenViewing] = useState<ListenViewing | null>(null);

const section: 'chat' | 'listen' | 'history' | null = !cardOpen
  ? null
  : pinned ?? (listenWanted ? 'listen' : 'chat');

// Every card open starts unpinned with no viewed session.
useEffect(() => {
  if (!cardOpen) {
    setPinned(null);
    setListenViewing(null);
  }
}, [cardOpen]);
```

In `submitAsk`: `setPinned('chat')` before `dictation.submit(sendAsk)` (explicit send shows chat). In `pressMic`'s listen-start branch: `setPinned('listen')`. Treat `paused` as live: `listenState === 'listening' || listenState === 'paused'` → `listenStop()`; same for the `IrisButton` `active` prop.

Section render becomes:

```tsx
{section === 'chat' && <ChatSection />}
{section === 'listen' && (
  <ListenSection viewing={listenViewing} onSessionEnded={(v) => { setListenViewing(v); setPinned('listen'); }} />
)}
{/* section === 'history' renders in Task 7 */}
```

- [ ] **Step 7: Verify**

Run: `cd apps/native && bun run check-types && bun run build`
Manual smoke (if app runs): start listen → header badge/timer, pause freezes timer, stop → finished doc, copy-all pastes.

- [ ] **Step 8: Commit**

```bash
git add apps/native/src/components apps/native/src/views/Bar.tsx
git commit -m "feat(native): structured listen document — header, timer, pause, filter, copy"
```

---

### Task 6: `SummaryStrip` — pinned 2-line summary above the input row

**Files:**

- Create: `apps/native/src/components/listen/SummaryStrip.tsx`
- Modify: `apps/native/src/components/ListenSection.tsx` (render strip; delete the old inline `<section>` summary block ~L350)

**Interfaces:**

- Produces: `SummaryStrip({ summary }: { summary: ListenSummaryPayload | null })` — null renders nothing.

- [ ] **Step 1: Implement**

```tsx
export const SummaryStrip = ({ summary }: { summary: ListenSummaryPayload | null }) => {
  const [open, setOpen] = useState(false);
  if (!summary) return null;
  return (
    <div className='flex-none border-t border-border px-3.5 py-2'>
      <button type='button' onClick={() => setOpen((o) => !o)}
        className='flex w-full items-center justify-between gap-2 text-left'>
        <p className='text-xs font-semibold'>
          TLDR{summary.topic ? ` · ${summary.topic}` : ''}
        </p>
        {open ? <ChevronUpIcon className='size-3.5 text-muted-foreground' />
              : <ChevronDownIcon className='size-3.5 text-muted-foreground' />}
      </button>
      <p className={cn('mt-0.5 text-[12.5px] leading-[1.5] select-text', !open && 'line-clamp-2')}>
        {summary.tldr}
      </p>
      {open && (
        <>
          {summary.bullets.length > 0 && (
            <ul className='mt-1 list-disc pl-4 text-[12.5px] leading-[1.5]'>
              {summary.bullets.map((b) => <li key={b}>{b}</li>)}
            </ul>
          )}
          {summary.follow_ups.length > 0 && (
            <div className='mt-2 flex flex-wrap gap-1.5'>
              {summary.follow_ups.map((f) => <span key={f} className={CHIP}>{f}</span>)}
            </div>
          )}
        </>
      )}
    </div>
  );
};
```

- [ ] **Step 2: Wire + remove the old inline summary**

In `ListenSection.tsx`: delete the `{summary && (<section>…)}` block inside the scroll area; render `<SummaryStrip summary={summary} />` as the last child of the section column (after the scroll container, before nothing — it sits directly above the card's input row). The card's ResizeObserver picks up the height change automatically.

- [ ] **Step 3: Verify + commit**

Run: `cd apps/native && bun run check-types && bun run build`

```bash
git add apps/native/src/components
git commit -m "feat(native): pin the listen summary as a clamped strip above the input row"
```

---

### Task 7: History mode — `CardTabs`, `HistorySection`, capsule icon

**Files:**

- Create: `apps/native/src/components/HistorySection.tsx`
- Create: `apps/native/src/components/bar/CardTabs.tsx`
- Modify: `apps/native/src/views/Bar.tsx` (tabs render, capsule icon, history wiring)
- Modify: `apps/native/src/lib/bar-state.ts` (collapsed controls += `'history'`)
- Modify: `apps/native/src-tauri/src/windows/mod.rs` (`BAR_IDLE_W` 140 → 172)

**Interfaces:**

- Consumes: `sessionList`, `sessionResume`, `sessionDelete`, `ListenViewing`, Bar's `setPinned`/`setListenViewing` (Task 5), icons (Task 4).
- Produces: `HistorySection({ askBusy, onOpenChat, onOpenListen })`; `CardTabs({ section, listenLive, onPick })`.

- [ ] **Step 1: `bar-state.ts`** — `BarControl` += `'history'`; `barControls(false)` returns `['iris','capture','listen','history']`.

- [ ] **Step 2: `windows/mod.rs`** — `const BAR_IDLE_W: f64 = 172.0;` (comment: four capsule controls).

- [ ] **Step 3: `CardTabs.tsx`**

```tsx
export const CardTabs = ({
  section, listenLive, onPick,
}: {
  section: 'chat' | 'listen' | 'history';
  listenLive: boolean;
  onPick: (s: 'chat' | 'listen' | 'history') => void;
}) => {
  const tabs = [
    { id: 'chat' as const, label: 'Chat', icon: MessageSquareTextIcon },
    { id: 'listen' as const, label: 'Listen', icon: MicAudioLinesIcon },
    { id: 'history' as const, label: 'History', icon: HistoryIcon },
  ];
  return (
    <div className='flex flex-none items-center gap-1 border-b border-border px-3 py-1.75'>
      {tabs.map(({ id, label, icon: Icon }) => (
        <button key={id} type='button' onClick={() => onPick(id)}
          className={cn(CHIP, 'cursor-pointer gap-1 text-[10px] transition-colors',
            section === id ? 'border-foreground/60 bg-fg-soft text-foreground' : 'hover:text-foreground')}>
          <Icon className='size-3' />
          {label}
          {id === 'listen' && listenLive && (
            <i aria-hidden className='size-1.25 animate-capture-ping rounded-full bg-accent' />
          )}
        </button>
      ))}
    </div>
  );
};
```

- [ ] **Step 4: `HistorySection.tsx`**

```tsx
export const HistorySection = ({
  askBusy, onOpenChat, onOpenListen,
}: {
  askBusy: boolean;
  onOpenChat: () => void;
  onOpenListen: (v: ListenViewing | null) => void;
}) => {
  const [sessions, setSessions] = useState<Session[] | null>(null);
  const refresh = () => {
    void sessionList().then(setSessions).catch(() => {});
  };
  useEffect(refresh, []);
  // A listen session starting/stopping changes the list live.
  useTauriEvent(EV_LISTEN_STATE, refresh);
  useTauriEvent(EV_ASK_STATE, (p) => { if (p.state === 'idle') refresh(); });

  const openRow = (s: Session) => {
    if (s.kind === 'ask') {
      if (askBusy) return; // an in-flight run belongs to the open session
      void sessionResume(s.id).then((ok) => ok && onOpenChat()).catch(() => {});
      return;
    }
    onOpenListen(
      s.ended_at === null
        ? null // live session → live view
        : { id: s.id, startedAt: s.started_at, endedAt: s.ended_at },
    );
  };

  const removeRow = (s: Session) => {
    void sessionDelete(s.id)
      .then(() => setSessions((prev) => prev?.filter((x) => x.id !== s.id) ?? prev))
      .catch(() => {});
  };

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      <header className={PANEL_HEAD}>
        <p className='min-w-0 flex-1 text-xs font-[550]'>History</p>
      </header>
      <div className={PANEL_BODY}>
        {sessions === null ? null : sessions.length === 0 ? (
          <p className={EMPTY}>No history yet — ask Marvis or start listening.</p>
        ) : (
          sessions.map((s) => {
            const liveRow = s.ended_at === null;
            const disabled = s.kind === 'ask' && askBusy;
            return (
              <div key={s.id}
                className={cn('group/row -mx-1.5 flex items-center gap-2 rounded-lg px-1.5 py-1.5 transition-colors',
                  disabled ? 'opacity-50' : 'cursor-pointer hover:bg-fg-soft')}
                onClick={() => !disabled && openRow(s)}>
                {s.kind === 'listen'
                  ? <MicAudioLinesIcon className='size-3.5 flex-none text-muted-foreground' />
                  : <MessageSquareTextIcon className='size-3.5 flex-none text-muted-foreground' />}
                <span className='min-w-0 flex-1 truncate text-[12.5px]'>
                  {s.title ?? (s.kind === 'listen' ? 'Meeting' : 'Chat')}
                </span>
                {liveRow && (
                  <span className={cn(CHIP, 'border-accent/40 text-accent')}>Live</span>
                )}
                <span className={cn(NUM, 'flex-none text-[10px] text-muted-foreground')}>
                  {relTime(s.last_active_at)}
                </span>
                {!liveRow && (
                  <button type='button' aria-label='Delete session'
                    onClick={(e) => { e.stopPropagation(); removeRow(s); }}
                    className={cn(ICON_BTN, 'size-5 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100')}>
                    <Trash2Icon className='size-3' />
                  </button>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
};
```

- [ ] **Step 5: `Bar.tsx` wiring**

Capsule (inside the `controls.includes('listen')` block's sibling — add after it):

```tsx
{controls.includes('history') && (
  <BarButton label='History' disabled={gate !== 'main'}
    onPress={() => {
      setPinned('history');
      void windowSetChatOpen(true).catch(() => {});
    }}>
    <HistoryIcon className='size-5' />
  </BarButton>
)}
```

Card render: add `CardTabs` as the LAST DOM child of the card div (flex-col-reverse puts it on top) and the history section:

```tsx
{section === 'history' && (
  <HistorySection
    askBusy={askState !== 'idle'}
    onOpenChat={() => setPinned('chat')}
    onOpenListen={(v) => { setListenViewing(v); setPinned('listen'); }}
  />
)}
{cardOpen && (
  <CardTabs
    section={section ?? 'chat'}
    listenLive={listenState === 'listening' || listenState === 'paused'}
    onPick={(s) => {
      setPinned(s);
      if (s === 'listen') setListenViewing(null); // tab = back to live
    }}
  />
)}
```

Import `HistoryIcon` in Bar.tsx's `@marvis/ui` import; `CardTabs`, `HistorySection` imports too.

- [ ] **Step 6: Verify**

Run: `cd apps/native/src-tauri && cargo test` (width const is safe) then `cd apps/native && bun run check-types && bun run build`.
Manual: capsule shows 4 icons at ~172px; idle→history 1 click; tabs switch; resume a chat; open a past meeting; delete a row; collapse clears the pin.

- [ ] **Step 7: Commit**

```bash
git add apps/native apps/native/src-tauri/src/windows/mod.rs
git commit -m "feat(native): history card mode — unified session list, resumable chats"
```

---

### Task 8: Full verification

- [ ] **Step 1: Rust**

Run: `cd apps/native/src-tauri && cargo test && cargo clippy`
Expected: all PASS, zero warnings.

- [ ] **Step 2: Frontend**

Run: `cd apps/native && bun run check-types && bun run build`
Expected: PASS.

- [ ] **Step 3: Manual checklist** (run `bun run build:dev` or the app's dev loop)

```text
[ ] idle capsule shows 4 icons (~172px)
[ ] capsule clock icon → card opens on History (1 step)
[ ] card tabs switch Chat | Listen | History
[ ] listen: badge LISTENING + ticking m:ss; mic+system subtitle
[ ] pause → PAUSED + timer freezes; resume → LISTENING + timer continues
[ ] speaker filter chips filter blocks; "N rows" tracks
[ ] per-block hover copy; header copy-all incl. TLDR/bullets
[ ] summary strip: 2-line clamp pinned above input; chevron expands
[ ] stop → card stays on the finished doc (STOPPED)
[ ] history: tap ended listen row → same doc view; tap Live row → live
[ ] history: tap chat row → resumed; send appends to that session
[ ] history: delete removes row; Live row has no delete
[ ] Esc / iris collapse resets pin + viewing
[ ] dictation blocked while a listen session is paused
```

- [ ] **Step 4: Fix anything found, then commit**

```bash
git commit -am "fix(native): history + listen redesign polish"
```
