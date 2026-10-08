# Session Audio Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add ended-session WAV playback with two-way transcript sync and a
native Save audio… export action to the Listen document, per
`docs/superpowers/specs/2026-10-08-session-audio-design.md`.

**Architecture:** The webview streams the retained WAV through Tauri's scoped
`asset:` protocol and owns playback state, seek math, and active-block
highlighting. Rust owns session-id-to-file resolution, the native save dialog,
and byte-for-byte copying. The existing Listen status/viewing models are
extended with `audio_file`/`audioFile` so the same player works after stopping
and from History.

**Tech Stack:** Rust/Tauri 2, `rfd`, SQLite via the existing `Db`, Tauri
asset protocol, React/TypeScript, native HTML `<audio>`, Bun tests, Vitest/Bun
existing test conventions, lucide icons through `@marvis/ui`.

## Global Constraints

- `bun` for JavaScript/package commands; `cargo` in `apps/native/src-tauri`.
- IPC wrappers live in `apps/native/src/lib/commands.ts`; components never
  call raw `invoke`. Rust params use snake_case; JS wrapper args use camelCase.
- Rust commands return `Result<T, String>` and mutating commands guard with
  `*state.gate.lock() != Gate::Main`; no `parking_lot` guard crosses `.await`.
- Blocking dialogs and filesystem copies run through
  `tauri::async_runtime::spawn_blocking`.
- The save command accepts a session id, never a webview-supplied source path.
- Asset access is limited to `$HOME/.marvis/audios/**`; do not add broad fs
  permissions or edit `gen/schemas/`.
- Playback is available only when the session has ended and the WAV metadata
  reports `duration > 0`; missing/header-only files degrade silently.
- Keep the existing `recording_*.wav` names unchanged; saved copies use the
  existing `marvis-{slug}-YYYYMMDD-HHmm.wav` suggestion style.
- Webview cross-directory imports use `@/`; components/hooks use arrow
  functions and named exports; icons use `Icon`-suffixed names.
- Verification: `cargo test` + `cargo clippy -- -D warnings` in
  `apps/native/src-tauri`; `bun test` + `bun run check-types` + `bun run build`
  in `apps/native`.

---

### Task 1: Backend audio status and safe WAV save command

**Files:**

- Modify: `apps/native/src-tauri/src/listen.rs`
- Modify: `apps/native/src-tauri/src/lib.rs`
- Modify: `apps/native/src-tauri/src/storage.rs` (add the focused
  `session_audio_file` getter)
- Modify: `apps/native/src-tauri/src/lib.rs` tests

**Interfaces:**

- Produces `ListenStatus.audio_file: Option<String>`, serialized as
  `audio_file`, populated from the active session row when a session exists.
- Produces `save_audio_file(session_id: i64, suggested_name: String) ->
  Result<Option<String>, String>` registered in `generate_handler!`.
- The command returns `Ok(None)` for user cancel (and the existing gate-drop
  no-op convention), `Ok(Some(path))` after a successful copy, and an error
  string when the session has no usable audio or copying fails.

- [ ] **Step 1: Add failing Rust contract and status tests**

In the existing `lib.rs` source-contract test module, add a test beside the
`save_text_file` contract test:

```rust
#[test]
fn audio_export_command_is_registered_and_guarded() {
    let source = include_str!("lib.rs");
    assert!(source.contains(concat!("save", "_audio_file,")));
    let body = source
        .split("async fn save_audio_file(")
        .nth(1)
        .and_then(|rest| rest.split("\n/// ").next())
        .expect("save_audio_file body not found");
    assert!(body.contains("*state.gate.lock() != Gate::Main"));
    assert!(body.contains("spawn_blocking"));
    assert!(body.contains("audio_file"));
    assert!(!body.contains("source_path"));
}
```

Extend the existing `listen_status_serializes_documented_wire_field_names`
test fixture with `audio_file: None` and assert the sorted expected wire keys
include `"audio_file"`. In `storage.rs`, add a focused getter and test:

```rust
pub fn session_audio_file(&self, id: i64) -> anyhow::Result<Option<String>> {
    Ok(self
        .conn
        .lock()
        .query_row(
            "SELECT audio_file FROM sessions WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?
        .flatten())
}
```

The test creates a listen session, sets its audio file to a temporary WAV,
asserts the getter returns that path, and asserts a missing session returns
`None`. This getter is the only DB lookup used by the save command.

Run:

```bash
cd apps/native/src-tauri && cargo test audio_export_command
cd apps/native/src-tauri && cargo test \
  listen_status_serializes_documented_wire_field_names
```

Expected: FAIL because the command, field, and updated fixture do not yet
exist.

- [ ] **Step 2: Extend the backend status model**

Add the field to `listen::ListenStatus` immediately after `session_id`:

```rust
pub audio_file: Option<String>,
```

Initialize it in every `ListenStatus` literal. For active sessions, populate
it from the existing `Db` session row after `session_get_or_create_active`
has created/linked the session and after `session_set_audio_file` has run. For
idle/error/reset status literals, use `None`. Preserve the existing status
semantics; this field is metadata only and must not change capture behavior.

Ensure `listen_status` and emitted status payloads return the field through the
existing `ListenStatus` serialization. Update the Rust serialization fixture
and the TypeScript `ListenStatus` interface in Task 3's wrapper change.

Run:

```bash
cd apps/native/src-tauri && cargo test listen_status
```

Expected: PASS, including the exact `audio_file` wire key.

- [ ] **Step 3: Implement `save_audio_file`**

Add the command near `save_text_file` in `lib.rs`. Resolve the source path
from the database using `session_id`; do not accept or copy a path supplied by
the webview. Reuse `sanitize_suggested_name` from the transcript export
feature, and use the existing audio directory only as a defense-in-depth check
if the DB path is not null:

```rust
#[tauri::command]
async fn save_audio_file(
    app: AppHandle,
    session_id: i64,
    suggested_name: String,
) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("save_audio_file dropped while gate != Main");
        return Ok(None);
    }
    let source = state
        .db
        .session_audio_file(session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No audio recording for this session".to_string())?;
    let source = std::path::PathBuf::from(source);
    let audios = crate::paths::audios_dir();
    let source = source
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let audios = audios.canonicalize().map_err(|e| e.to_string())?;
    if !source.starts_with(&audios) {
        return Err(
            "Audio recording is outside the Marvis audio directory".into(),
        );
    }
    if std::fs::metadata(&source)
        .map_err(|e| e.to_string())?
        .len()
        <= 44
    {
        return Err("This session has no recorded audio".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(sanitize_suggested_name(&suggested_name))
            .add_filter("WAV audio", &["wav"])
            .save_file()
        else {
            return Ok(None);
        };
        std::fs::copy(&source, &path).map_err(|e| e.to_string())?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}
```

Add the `session_audio_file` getter from Step 1 to `storage.rs` and call it
through `state.db` before opening the dialog. The source path validation must
happen before spawning the dialog, and no AppState or DB lock may be held
inside the blocking closure. Register `save_audio_file,` beside
`save_text_file,` in `generate_handler!`.

Run:

```bash
cd apps/native/src-tauri && cargo test audio_export
cd apps/native/src-tauri && cargo test
```

Expected: PASS, including command registration, status serialization, and
existing storage tests.

- [ ] **Step 4: Run clippy and commit the backend task**

```bash
cd apps/native/src-tauri && cargo clippy -- -D warnings
```

Expected: clean. Commit:

```bash
git add apps/native/src-tauri/src/listen.rs \
  apps/native/src-tauri/src/lib.rs \
  apps/native/src-tauri/src/storage.rs
git commit -m "Audio: expose retained WAV and add safe session export command"
```

---

### Task 2: Asset protocol and pure audio timing helpers

**Files:**

- Modify: `apps/native/src-tauri/tauri.conf.json`
- Modify: `apps/native/src-tauri/capabilities/default.json`
- Modify: `apps/native/src/components/listen/model.ts`
- Modify: `apps/native/src/components/listen/model.test.ts`
- Modify: `apps/native/src/components/listen/model.ts` `ListenViewing` type

**Interfaces:**

- Produces `audioOffset(block: TurnBlock, startedAt: number | null): number`
  with a minimum result of `0`.
- Produces `activeBlockAt(blocks: TurnBlock[], startedAt: number | null,
  seconds: number): string | null`, returning the block React key
  (`${block.key}-${block.ts}`) or null when playback is before the first block,
  in a gap, or after the final block.
- Produces `exportFileName(topic, startedAt, extension = 'md'): string`,
  preserving existing Markdown output by default and supporting `.wav`.
- Extends `ListenViewing` with `audioFile: string | null`.

- [ ] **Step 1: Add failing helper tests**

Append tests to `model.test.ts` using existing `turn()` and `buildBlocks()`
fixtures:

```ts
test('audioOffset clamps a block before the session start', () => {
  const [block] = buildBlocks([
    turn(90, { speaker: 'me', speaker_idx: null }),
  ]);
  expect(audioOffset(block!, 100)).toBe(0);
  expect(audioOffset(block!, 42)).toBe(48);
});

test('activeBlockAt resolves blocks and leaves gaps inactive', () => {
  const blocks = buildBlocks([
    turn(100, { speaker: 'me', speaker_idx: null }),
    turn(110, { speaker: 'them', speaker_idx: 0 }),
  ]);
  expect(activeBlockAt(blocks, 100, 0)).toBe('me:0-100');
  expect(activeBlockAt(blocks, 100, 9)).toBe('me:0-100');
  expect(activeBlockAt(blocks, 100, 10)).toBe('them:0-110');
  expect(activeBlockAt(blocks, 100, -1)).toBe(null);
});

test('exportFileName supports a WAV extension', () => {
  expect(exportFileName('Weekly Standup', 1_700_000_000, 'wav'))
    .toMatch(/^marvis-weekly-standup-\d{8}-\d{4}\.wav$/);
});
```

Also update the viewed-session fixture shape in any existing tests to include
`audioFile: null` where `ListenViewing` is constructed.

Run:

```bash
cd apps/native && bun test model.test.ts
```

Expected: FAIL because the helpers and extension parameter do not exist.

- [ ] **Step 2: Implement the pure helpers**

Add the following model helpers beside `elapsedLabel`/`exportFileName`:

```ts
export const audioOffset = (
  block: TurnBlock,
  startedAt: number | null,
) => Math.max(0, block.ts - (startedAt ?? block.ts));

export const activeBlockAt = (
  blocks: TurnBlock[],
  startedAt: number | null,
  seconds: number,
) => {
  if (startedAt == null || seconds < 0) return null;
  const timestamp = startedAt + seconds;
  const index = blocks.findIndex((block, i) => {
    const next = blocks[i + 1];
    return block.ts <= timestamp && (!next || timestamp < next.ts);
  });
  return index < 0 ? null : `${blocks[index]!.key}-${blocks[index]!.ts}`;
};
```

Use the existing slug/stamp implementation for `exportFileName`, changing its
signature to `extension = 'md'` and returning:

```ts
return `marvis-${slug || 'listen'}-${stamp}.${extension}`;
```

Do not rename or migrate stored `recording_*.wav` paths. Add
`audioFile: string | null` to `ListenViewing`.

Run:

```bash
cd apps/native && bun test model.test.ts
cd apps/native && bun run check-types
```

Expected: all model tests pass and TypeScript is clean.

- [ ] **Step 3: Enable the narrowly scoped asset protocol**

In `tauri.conf.json`, replace the one-line security object without changing
CSP or window configuration:

```json
"security": {
  "csp": null,
  "assetProtocol": {
    "enable": true,
    "scope": ["$HOME/.marvis/audios/**"]
  }
}
```

Do not add a capability permission: `convertFileSrc` uses the configured core
asset protocol, and the existing capability remains least-privileged. Do not
add a broad filesystem plugin permission or edit `gen/schemas/`.

Run:

```bash
cd apps/native/src-tauri && cargo check
```

Expected: PASS with the config and capability schema accepted.

- [ ] **Step 4: Commit the model/config task**

```bash
git add apps/native/src-tauri/tauri.conf.json \
  apps/native/src-tauri/capabilities/default.json \
  apps/native/src/components/listen/model.ts \
  apps/native/src/components/listen/model.test.ts
git commit -m "Audio: add scoped asset playback and timing helpers"
```

---

### Task 3: SessionPlayer and transcript synchronization

**Files:**

- Create: `apps/native/src/components/listen/SessionPlayer.tsx`
- Modify: `apps/native/src/components/listen/TranscriptBlocks.tsx`
- Modify: `apps/native/src/components/ListenSection.tsx`
- Modify: `apps/native/src/components/listen/model.test.ts` only if a UI-facing
  helper test needs to be added

**Interfaces:**

- `SessionPlayer` consumes `audioFile: string`, `blocks: TurnBlock[]`,
  `startedAt: number`, `activeBlock: string | null`, and callbacks
  `onTime(seconds)`, `onSeek(seconds)`, `onReady(duration)`, `onError()`.
- `TranscriptBlocks` consumes optional `activeBlock: string | null` and
  `onSeekBlock: (block: TurnBlock) => void`; absent callback preserves existing
  copy-only behavior for any non-player caller.
- `ListenSection` owns one `HTMLAudioElement` ref, playback state, duration,
  current time, and active-block state. It renders the player only for an ended
  session with a non-null audio path; `<SessionPlayer>` hides itself after
  metadata reports `duration <= 0` or emits an error.

- [ ] **Step 1: Add the player component**

Create `SessionPlayer.tsx` as a named arrow component. Use
`convertFileSrc` from `@tauri-apps/api/core` (the installed Tauri 2 API) for
the source URL. Keep the native `<audio>` element visually hidden but present,
with `preload="metadata"`. Render a compact row containing a play/pause
button, a range input from `0` to `duration`, and `elapsed / duration` labels.
Use `PlayIcon`, `PauseIcon`, and `ICON_BTN` through existing conventions.
The component must:

- call `onReady(audio.duration)` on `loadedmetadata`;
- call `onTime(audio.currentTime)` on `timeupdate` and `seeking`;
- call `onError` on `error`;
- toggle `audio.play()`/`audio.pause()` from the button;
- assign `audio.currentTime` from the range input and call `onSeek`;
- reset/stop on unmount or `audioFile` change;
- render nothing when the parent marks it unavailable.

Use `duration > 0` as the parent's availability condition; a 44-byte WAV
must not render a player row.

- [ ] **Step 2: Wire player state and asset source in ListenSection**

Add `saveAudioFile` to the commands import in the next task's shared wiring,
but keep playback independent of save. Add refs/state:

```ts
const audioRef = useRef<HTMLAudioElement>(null);
const [audioReady, setAudioReady] = useState(false);
const [audioDuration, setAudioDuration] = useState(0);
const [audioTime, setAudioTime] = useState(0);
const [activeBlock, setActiveBlock] = useState<string | null>(null);
```

Derive `audioFile` and `audioEnded` from the live `status` or viewed
`viewing`. On live-to-viewed stop, carry `status.audio_file` into
`onSessionEnded`; History maps `Session.audio_file` into
`ListenViewing.audioFile`. Only render `SessionPlayer` when
`audioFile && audioEnded`; pass the asset URL
through the player, not a raw filesystem path to an `<audio>` element.

On each time update, call `activeBlockAt(blocks, startedAt, seconds)` and set
`activeBlock`. On timestamp click, set `audio.currentTime = audioOffset(...)`
and call `audio.play()`. On doc switch/unmount, pause and clear the audio ref
state. Keep playback blocks unfiltered: if a speaker filter hides the active
block, the active state remains derived from the full document.

Place the player below `ListenHeader` and above `SpeakerFilter`, so the audio
surface is visible without displacing transcript layout during playback.

- [ ] **Step 3: Add timestamp seeking and active styling**

Extend `TranscriptBlocks` with the optional props from the interface. Wrap the
existing timestamp `<span>` in a button only when `onSeekBlock` exists; retain
the same visual classes, add an accessible label such as `Seek to 0:42`, and
call `onSeekBlock(block)`. Apply a low-alpha background/ring class derived
from the existing `block.color` when its key equals `activeBlock`. Do not make
the copy button or text paragraph seek targets, preserving text selection and
copy behavior.

Pass `activeBlock` and the seek callback from `ListenSection` to the rendered
`TranscriptBlocks`, including the currently filtered `shown` array. The seek
callback still receives the selected block's absolute timestamp.

- [ ] **Step 4: Run focused web checks and commit**

```bash
cd apps/native && bun test model.test.ts
cd apps/native && bun run check-types
```

Expected: model tests pass and TypeScript is clean. Commit:

```bash
git add apps/native/src/components/listen/SessionPlayer.tsx \
  apps/native/src/components/listen/TranscriptBlocks.tsx \
  apps/native/src/components/ListenSection.tsx \
  apps/native/src/components/listen/model.ts
 git commit -m "Audio: add ended-session player with transcript sync"
```

---

### Task 4: Save audio menu integration and full verification

**Files:**

- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/components/ListenSection.tsx`
- Modify: `apps/native/src/components/listen/SpeakerFilter.tsx`
- Modify: `apps/native/src/components/HistorySection.tsx`
- Modify: `apps/native/src/components/listen/model.ts` if extension typing
  needs to be exported

**Interfaces:**

- Produces `saveAudioFile(sessionId: number, suggestedName: string):
  Promise<string | null>` invoking `save_audio_file` with `{ sessionId,
  suggestedName }`.
- Adds `Save audio…` below the two existing Markdown export items.
- Save is available only under the same ended-session/real-audio visibility
  rule as playback; missing/header-only files are not offered.

- [ ] **Step 1: Add the typed command wrapper and wire status/viewing data**

In `commands.ts`, extend `ListenStatus`:

```ts
export interface ListenStatus {
  // existing fields
  audio_file: string | null;
}
```

Add after `saveTextFile`:

```ts
/** Session WAV export: native save dialog + byte-for-byte copy. */
export const saveAudioFile = (sessionId: number, suggestedName: string) =>
  invoke<string | null>('save_audio_file', { sessionId, suggestedName });
```

In `HistorySection.tsx`, add `audioFile: s.audio_file` to the
`ListenViewing` object. In the live stop transition in `ListenSection.tsx`,
add `audioFile: status.audio_file` to `onSessionEnded`. Update the
live-to-viewed and History `ListenViewing` literals; any test-only literal
must also include `audioFile: null`.

- [ ] **Step 2: Add Save audio… to SpeakerFilter**

Extend the existing export props with:

```ts
onSaveAudio: () => void;
canSaveAudio: boolean;
```

Render a third menu button below `Save .md…`:

```tsx
<button
  type='button'
  disabled={!canSaveAudio}
  className={cn(
    'flex w-full cursor-pointer items-center border-0',
    'bg-transparent px-2.5 py-1.5 text-left text-foreground',
    'enabled:hover:bg-fg-soft disabled:opacity-40',
  )}
  onClick={() => {
    setExportOpen(false);
    onSaveAudio();
  }}>
  Save audio…
</button>
```

Prefer not rendering the item when `canSaveAudio` is false so missing and
header-only recordings silently degrade and cannot be invoked accidentally.
The parent can pass `canSaveAudio = Boolean(audioFile && audioEnded &&
audioReady)`, where `audioReady` means metadata duration is positive.

- [ ] **Step 3: Wire save behavior and feedback**

In `ListenSection`, add `saveAudioFile` to the command imports and a handler:

```ts
const saveAudio = () => {
  if (!audioFile || !audioReady || !audioEnded) return;
  void saveAudioFile(
    live ? status.session_id! : viewing.id,
    exportFileName(summary?.topic, startedAt, 'wav'),
  )
    .then((path) => {
      if (path !== null) {
        setExported(true);
        window.setTimeout(() => setExported(false), 1500);
      }
    })
    .catch((error) =>
      raise(typeof error === 'string' ? error : 'Audio export failed'),
    );
};
```

Because the player is ended-only, `live` should normally be false when the
menu is available; retain the guarded `session_id` branch for the live-to-viewed
transition and avoid invoking with null. Pass `onSaveAudio` and
`canSaveAudio` to `SpeakerFilter`. Keep Markdown copy/save unchanged and keep
plain-text Copy transcript unchanged.

- [ ] **Step 4: Run complete verification**

```bash
cd apps/native/src-tauri && cargo test
cd apps/native/src-tauri && cargo clippy -- -D warnings
cd apps/native && bun test
cd apps/native && bun run check-types
cd apps/native && bun run build
```

Expected: all existing and new tests pass, clippy is clean, type checking is
clean, and the production web build succeeds. Then run:

```bash
bun run build:dev
```

Manual checks:

- Ended session with a real WAV shows the player; live session does not.
- Header-only 44-byte recording and missing path show neither a useful player
  nor Save audio…; no error toast is shown solely for that absence.
- Play/pause works; duration is shown; seek slider changes current position.
- Clicking a transcript timestamp seeks to that block and starts playback.
- Playback highlights the containing block; scrubbing updates the highlight.
- Speaker filtering does not corrupt seek math or export scope.
- Save audio… opens the native WAV dialog and copies a byte-identical playable
  file; cancel is silent; filesystem/session errors appear through the alert
  toast.
- Existing Copy transcript, Copy markdown, and Save .md… remain unchanged.

- [ ] **Step 5: Commit integration and record status**

```bash
git add apps/native/src/lib/commands.ts \
  apps/native/src/components/ListenSection.tsx \
  apps/native/src/components/listen/SpeakerFilter.tsx \
  apps/native/src/components/listen/SessionPlayer.tsx \
  apps/native/src/components/listen/TranscriptBlocks.tsx \
  apps/native/src/components/listen/model.ts \
  apps/native/src/components/listen/model.test.ts \
  apps/native/src/components/HistorySection.tsx
git commit -m "Audio: add WAV export menu and complete playback surface"
```

Do not mark the roadmap item shipped until manual playback and save-dialog QA
has passed.
