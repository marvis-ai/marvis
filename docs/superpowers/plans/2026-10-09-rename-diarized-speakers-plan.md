# Rename Diarized Speakers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add session-local inline names for diarized speakers while keeping
voiceprint-backed `You` fixed and anonymous identities stable.

**Architecture:** Keep Rust transcript identities unchanged. Add a pure
first-seen display-name resolver and a session-local override map owned by
`ListenSection`; feed the resolved names into the existing `TurnBlock` model
so filters, transcript rows, copy, and export share one value. A shared inline
editor is used by filter chips and transcript headers.

**Tech Stack:** React 19, TypeScript 6, Bun tests, date-fns, Tailwind CSS,
existing `@marvis/ui` icon barrel.

## Global Constraints

- “You” is fixed; only labeled non-You diarized identities are renameable.
- Speaker names are session-local and must not be persisted in SQLite.
- Existing `speakerKey` identity keys remain unchanged.
- React components and hooks use arrow-function syntax.
- Non-UI authored components use named exports only.
- Apps import icons through `@marvis/ui` with `Icon`-suffixed names.
- Production code must be preceded by a failing test.
- Do not add a dependency for the inline editor.
- Do not change Rust diarization, voiceprint, audio seeking, or transcript
  persistence behavior.

---

## File map

| File | Responsibility |
| --- | --- |
| `apps/native/src/components/listen/model.ts` | Identity defaults, overrides, blocks, copy/export names |
| `apps/native/src/components/listen/model.test.ts` | Pure naming and export behavior tests |
| `apps/native/src/components/listen/SpeakerNameEditor.tsx` | Reusable inline edit control |
| `apps/native/src/components/listen/SpeakerNameEditor.test.tsx` | Keyboard and blur behavior |
| `apps/native/src/components/listen/SpeakerFilter.tsx` | Filter chips and chip rename trigger |
| `apps/native/src/components/listen/TranscriptBlocks.tsx` | Transcript headers and rename trigger |
| `apps/native/src/components/ListenSection.tsx` | Session-local override state and callbacks |
| `packages/ui/src/index.ts` | `PencilIcon` barrel export |

---

### Task 1: Add pure identity-name resolution

**Files:**

- Modify: `apps/native/src/components/listen/model.ts`
- Test: `apps/native/src/components/listen/model.test.ts`

**Interfaces:**

```ts
export type SpeakerNameOverrides = ReadonlyMap<string, string>;

export const isRenameableSpeaker: (
  turn: TurnIdentity,
) => boolean;

export const resolveSpeakerNames: (
  turns: readonly Turn[],
  overrides?: SpeakerNameOverrides,
) => ReadonlyMap<string, string>;

export const buildBlocks: (
  turns: Turn[],
  overrides?: SpeakerNameOverrides,
) => TurnBlock[];
```

`TurnBlock` gains `canRename: boolean`. `resolveSpeakerNames` assigns
`You` to `me:0` (including `me:null` through the existing `speakerKey` merge),
`Speaker` to unlabelled `them`, and first-seen `Speaker N` labels to all other
identities. Overrides replace only the resolved display value.

- [ ] **Step 1: Write the failing model tests**

Add these tests to `model.test.ts`:

```ts
test('assigns one You identity and unique anonymous names by first sighting', () => {
  const turns = [
    turn(0, { speaker: 'them', speaker_idx: 0 }),
    turn(1, { speaker: 'me', speaker_idx: null }),
    turn(2, { speaker: 'me', speaker_idx: 1 }),
    turn(3, { speaker: 'them', speaker_idx: 1 }),
  ];

  expect([...resolveSpeakerNames(turns)]).toEqual([
    ['them:0', 'Speaker 1'],
    ['me:0', 'You'],
    ['me:1', 'Speaker 2'],
    ['them:1', 'Speaker 3'],
  ]);
});

test('overrides one identity without changing its stable filter key', () => {
  const blocks = buildBlocks(
    [
      turn(0, { speaker: 'them', speaker_idx: 0 }),
      turn(1, { speaker: 'me', speaker_idx: 1 }),
      turn(2, { speaker: 'them', speaker_idx: 0 }),
    ],
    new Map([['them:0', 'Alice']]),
  );

  expect(blocks.map((block) => [block.key, block.name])).toEqual([
    ['them:0', 'Alice'],
    ['me:1', 'Speaker 2'],
    ['them:0', 'Alice'],
  ]);
  expect(blocks[0]!.canRename).toBe(true);
  expect(blocks[1]!.canRename).toBe(true);
});

test('You and an unlabelled system turn are not renameable', () => {
  const blocks = buildBlocks([
    turn(0, { speaker: 'me', speaker_idx: null }),
    turn(1, { speaker: 'them', speaker_idx: null }),
  ]);

  expect(blocks.map((block) => [block.name, block.canRename])).toEqual([
    ['You', false],
    ['Speaker', false],
  ]);
});

test('copy and Markdown export use the resolved names', () => {
  const blocks = buildBlocks(
    [turn(1_700_000_042, { speaker: 'them', speaker_idx: 0 })],
    new Map([['them:0', 'Alice']]),
  );

  expect(transcriptCopyText(blocks, 1_700_000_000, null)).toContain(
    'Alice: t1700000042',
  );
  expect(
    transcriptMarkdown(
      blocks,
      { startedAt: 1_700_000_000 },
      null,
    ),
  ).toContain('Alice: t1700000042');
});
```

- [ ] **Step 2: Run the focused tests and verify they fail**

Run:

```bash
cd apps/native
bun test src/components/listen/model.test.ts
```

Expected: FAIL because `resolveSpeakerNames` and `TurnBlock.canRename` do not
exist yet, and `buildBlocks` does not accept overrides.

- [ ] **Step 3: Implement the smallest pure model change**

Add the identity predicates and resolver before `buildBlocks`:

```ts
const isYouIdentity = (turn: TurnIdentity) =>
  turn.speaker === 'me' && (turn.speaker_idx === null || turn.speaker_idx === 0);

export const isRenameableSpeaker = (turn: TurnIdentity) =>
  turn.speaker_idx !== null && !isYouIdentity(turn);

export const resolveSpeakerNames = (
  turns: readonly Turn[],
  overrides: SpeakerNameOverrides = new Map(),
) => {
  const defaults = new Map<string, string>();
  let next = 1;
  for (const turn of turns) {
    const key = speakerKey(turn);
    if (defaults.has(key)) continue;
    defaults.set(
      key,
      isYouIdentity(turn)
        ? 'You'
        : turn.speaker_idx === null
          ? 'Speaker'
          : `Speaker ${next++}`,
    );
  }
  return new Map(
    [...defaults].map(([key, name]) => [key, overrides.get(key) ?? name]),
  );
};
```

Update `buildBlocks` to accept the optional override map, resolve names once
before the loop, set `name` from the resolved map, and set `canRename` from the
first turn that creates the block. Remove the old `Guest N` branch; no other
block-splitting or color logic changes.

- [ ] **Step 4: Run the focused tests and verify they pass**

Run:

```bash
cd apps/native
bun test src/components/listen/model.test.ts
```

Expected: PASS, including all pre-existing block, copy, export, audio-offset,
and seeking tests.

- [ ] **Step 5: Commit the pure model change**

```bash
git add apps/native/src/components/listen/model.ts \
  apps/native/src/components/listen/model.test.ts
git commit -m "feat: resolve session-local diarized speaker names"
```

---

### Task 2: Build the reusable inline speaker editor

**Files:**

- Create: `apps/native/src/components/listen/SpeakerNameEditor.tsx`
- Test: `apps/native/src/components/listen/SpeakerNameEditor.test.tsx`
- Modify: `packages/ui/src/index.ts`

**Interfaces:**

```ts
export interface SpeakerNameEditorProps {
  label: string;
  editable: boolean;
  onCommit: (label: string) => void;
}

export const SpeakerNameEditor = (
  props: SpeakerNameEditorProps,
) => JSX.Element;
```

The component renders the label normally when `editable` is false. When it is
editable, clicking the label replaces it with an input. The input commits on
Enter or blur, cancels on Escape, trims whitespace, and caps the value at 40
Unicode scalar values. An empty value is passed to `onCommit` so the parent can
remove the override.

- [ ] **Step 1: Add the failing component behavior test**

Use the existing Bun + `happy-dom` + `createRoot` pattern from
`SessionPlayer.test.tsx`:

```tsx
test('edits on Enter, trims, and caps the committed label', async () => {
  const committed: string[] = [];
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);

  await act(async () =>
    root.render(
      <SpeakerNameEditor
        label='Speaker 1'
        editable
        onCommit={(value) => committed.push(value)}
      />,
    ),
  );
  (host.querySelector('button') as HTMLButtonElement).click();
  const input = host.querySelector('input') as HTMLInputElement;
  input.value = `  ${'x'.repeat(50)}  `;
  await act(async () =>
    input.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Enter',
      bubbles: true,
    })),
  );

  expect(committed).toEqual(['x'.repeat(40)]);
  await act(async () => root.unmount());
  host.remove();
});

test('Escape cancels without committing', async () => {
  const committed: string[] = [];
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);

  await act(async () =>
    root.render(
      <SpeakerNameEditor
        label='Speaker 1'
        editable
        onCommit={(value) => committed.push(value)}
      />,
    ),
  );
  await act(async () =>
    (host.querySelector('button') as HTMLButtonElement).click(),
  );
  const input = host.querySelector('input') as HTMLInputElement;
  input.value = 'Alice';
  await act(async () =>
    input.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Escape',
      bubbles: true,
    })),
  );

  expect(committed).toEqual([]);
  expect(host.textContent).toContain('Speaker 1');
  await act(async () => root.unmount());
  host.remove();
});
```

- [ ] **Step 2: Run the component test and verify it fails**

Run:

```bash
cd apps/native
bun test src/components/listen/SpeakerNameEditor.test.tsx
```

Expected: FAIL because the component and the test file do not exist.

- [ ] **Step 3: Export the pencil icon through the workspace barrel**

Modify the lucide export list in `packages/ui/src/index.ts`:

```ts
  PencilIcon,
```

Keep the export sorted with the existing icon list and do not import lucide
icons directly from the app.

- [ ] **Step 4: Implement the editor**

Use an arrow-function component with a local `editing` state, a draft state,
and an input ref. The commit path must be equivalent to:

```ts
const commit = () => {
  onCommit(draft.trim().slice(0, 40));
  setEditing(false);
};

const cancel = () => {
  setDraft(label);
  setEditing(false);
};
```

When the label prop changes while not editing, synchronize the draft. When
editing begins, focus and select the input on the next effect. Stop propagation
from the editor's trigger/input when it is embedded inside a filter chip so
editing does not also change the filter.

- [ ] **Step 5: Run the component test and verify it passes**

```bash
cd apps/native
bun test src/components/listen/SpeakerNameEditor.test.tsx
```

Expected: PASS with no console errors.

- [ ] **Step 6: Commit the editor**

```bash
git add packages/ui/src/index.ts \
  apps/native/src/components/listen/SpeakerNameEditor.tsx \
  apps/native/src/components/listen/SpeakerNameEditor.test.tsx
git commit -m "feat: add inline speaker name editor"
```

---

### Task 3: Wire names through Listen UI surfaces

**Files:**

- Modify: `apps/native/src/components/ListenSection.tsx`
- Modify: `apps/native/src/components/listen/SpeakerFilter.tsx`
- Modify: `apps/native/src/components/listen/TranscriptBlocks.tsx`

**Interfaces:**

```ts
// SpeakerFilter additions
onRename: (key: string, label: string) => void;

// TranscriptBlocks additions
onRename: (key: string, label: string) => void;
```

The `speakers` entries gain `canRename: boolean`. `TurnBlock.canRename` drives
both child surfaces; the identity key remains the callback argument.

- [ ] **Step 1: Add the failing surface integration test**

Extend the editor/component test coverage with a render that mounts a
`SpeakerFilter` containing one renameable chip, activates its rename control,
commits `Alice`, and asserts that `onRename` receives `('them:0', 'Alice')`.
Also mount `TranscriptBlocks` with the same `TurnBlock` and assert that its
rename callback receives the same key. Use the existing `happy-dom` render
setup; do not add a testing-library dependency.

- [ ] **Step 2: Run the integration test and verify it fails**

```bash
cd apps/native
bun test src/components/listen/SpeakerNameEditor.test.tsx
```

Expected: FAIL because the two components do not yet accept `onRename` or
render the shared editor.

- [ ] **Step 3: Add session-local state in `ListenSection`**

Add:

```ts
const [speakerOverrides, setSpeakerOverrides] = useState<
  Map<string, string>
>(() => new Map());

useEffect(() => {
  setSpeakerOverrides(new Map());
}, [viewing?.id, status.session_id]);

const renameSpeaker = (key: string, label: string) => {
  setSpeakerOverrides((previous) => {
    const next = new Map(previous);
    if (label.trim() === '') next.delete(key);
    else next.set(key, label.trim().slice(0, 40));
    return next;
  });
};

const blocks = useMemo(
  () => buildBlocks(turns, speakerOverrides),
  [turns, speakerOverrides],
);
```

Update the speaker-chip memo to include `canRename`, pass `renameSpeaker` to
both child components, and leave `filterKey` keyed by `block.key`.

- [ ] **Step 4: Add `SpeakerNameEditor` to both child surfaces**

In `SpeakerFilter`, keep filtering and renaming as separate controls in each
chip. The filter control calls `onPick(s.key)`; the editor calls
`onRename(s.key, label)`. Do not make the `all` item editable.

In `TranscriptBlocks`, replace the static `block.name` span with
`SpeakerNameEditor` and pass `block.canRename` and a callback that calls
`onRename(block.key, label)`. Keep timestamps, copy buttons, and audio seeking
unchanged.

- [ ] **Step 5: Run the native tests and type checks**

```bash
cd apps/native
bun test
bun run check-types
```

Expected: PASS. The existing user-provided repository type check already passed
before implementation; this step confirms the new props and icon export do not
break the workspace.

- [ ] **Step 6: Commit the Listen UI wiring**

```bash
git add apps/native/src/components/ListenSection.tsx \
  apps/native/src/components/listen/SpeakerFilter.tsx \
  apps/native/src/components/listen/TranscriptBlocks.tsx
git commit -m "feat: wire session-local speaker renames into Listen"
```

---

### Task 4: Verify the complete speaker feature

**Files:**

- No new production files.
- Review all files changed in Tasks 1–3.

- [ ] **Step 1: Run the focused model and editor tests**

```bash
cd apps/native
bun test src/components/listen/model.test.ts \
  src/components/listen/SpeakerNameEditor.test.tsx
```

Expected: PASS.

- [ ] **Step 2: Run the complete native frontend suite and type check**

```bash
cd apps/native
bun test
bun run check-types
```

Expected: PASS with no new warnings or errors.

- [ ] **Step 3: Run Rust regression tests**

```bash
cd apps/native/src-tauri
cargo test
```

Expected: PASS; no Rust source should have changed for this feature.

- [ ] **Step 4: Perform manual QA**

Run the native app and verify:

1. `You` remains fixed and has no rename control.
2. Other diarized identities begin as distinct `Speaker N` labels.
3. Renaming from a transcript header updates every matching block.
4. Renaming from a chip updates the chip and transcript.
5. Enter, blur, Escape, and empty-reset behavior match the spec.
6. Filtering still uses the same identity after a rename.
7. Clipboard and Markdown export contain the current labels.
8. Leaving and reopening a session clears the names.

- [ ] **Step 5: Commit only if manual QA required a fix**

If QA exposes a defect, add a focused failing test, fix it, rerun the full
suite, and commit the test and fix together. Do not commit unrelated cleanup.
