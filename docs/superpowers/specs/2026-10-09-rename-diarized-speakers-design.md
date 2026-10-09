# Session-Local Diarized Speaker Names Design

**Date:** 2026-10-09

**Status:** Approved for implementation

## Goal

Let a user rename anonymous diarized speakers within the currently displayed
Listen session while keeping the enrolled voiceprint identity fixed as `You`.
Renames affect the visible transcript, speaker filters, clipboard copy, and
Markdown export, but do not change stored diarization data or persist across
sessions.

## Scope

### In scope

- Keep the stable diarization identity as `speaker + speaker_idx`.
- Keep the voiceprint-backed mic identity labeled `You`.
- Give other labeled diarized identities anonymous defaults (`Speaker 1`,
  `Speaker 2`, and so on).
- Let users rename non-You labels inline from transcript headers and
  speaker-filter chips.
- Apply names consistently to transcript rendering, filters, plain-text copy,
  and Markdown export.
- Reset overrides when the live session or viewed session changes.

### Out of scope

- Changing the Rust diarization or voiceprint clustering algorithm.
- Renaming speakers globally across meetings.
- Persisting speaker names in SQLite.
- Sending user-defined labels into Ask's meeting context.
- Naming an unlabelled system-audio turn (`them` with `speaker_idx = null`)
  beyond its existing `Speaker` fallback.

## Existing context

The Rust speaker tracker already seeds cluster `0` from the enrolled voiceprint
on the microphone channel. Transcript events and persisted rows carry `speaker`
(`me` or `them`) and an optional `speaker_idx`. The frontend currently uses
`speakerKey` to keep identities separate, but `speakerName` displays nonzero
microphone clusters as `Guest N` while system-audio clusters display as
`Speaker N`.

The Listen UI builds `TurnBlock` objects in `components/listen/model.ts`.
`ListenSection` derives speaker-filter chips from those blocks, and the same
block names feed the transcript copy and Markdown export helpers.

## Identity and naming model

The existing `speakerKey` remains the identity boundary:

```text
me:0       verified/primary microphone identity → You
me:<n>     another microphone cluster → renameable anonymous speaker
me:null    joins me:0 → You

them:<n>   system-audio diarized cluster → renameable anonymous speaker
them:null  unlabelled system audio → Speaker, not renameable
```

When building a session's display model, the first-seen traversal of the turns
assigns each distinct renameable non-You identity the next available anonymous
label. The first four use `Speaker 1` through `Speaker 4`; if a provider emits
more identities, numbering continues rather than creating duplicate labels. The
existing color palette continues to cycle by diarizer index.

A session-local override map is keyed by `speakerKey`:

```ts
Record<string, string>
```

The resolved name is the override when present, otherwise the generated
anonymous default. The map is held by `ListenSection`; a live-session restart,
viewed-session change, or return to live clears it.

The existing `me:null` → `me:0` merge remains so unlabelled microphone turns do
not create a second You identity. When an enrolled voiceprint exists, the Rust
tracker guarantees that cluster `0` is seeded from it.

## Inline rename interaction

Both transcript headers and speaker-filter chips expose the same inline editor
for renameable identities. `You`, `all`, and unlabelled `Speaker` remain plain
labels.

Editor behavior:

- Enter commits the trimmed value.
- Blur commits the trimmed value.
- Escape restores the value that existed before editing.
- An empty committed value removes the override and restores the anonymous
  default.
- Names are capped at 40 Unicode scalar values before being stored in the
  local map.
- The stable filter key remains the identity key, so renaming never changes
  which turns are filtered.

The editor is a shared `SpeakerNameEditor` component under `components/listen/`,
keeping the chip and transcript implementations from drifting. The component
receives the current label, whether it is editable, and `onCommit`/`onCancel`
callbacks; it does not own session state.

## Data flow

```text
Listen turns
    ↓
resolve session display names + local overrides
    ↓
buildBlocks(turns, resolvedNames)
    ├── SpeakerFilter chips
    ├── TranscriptBlocks headers
    ├── transcriptCopyText
    └── transcriptMarkdown
```

`TurnBlock.name` remains the single display value consumed by all downstream
surfaces. Copy and export require no separate rename-specific formatting path.

The Ask backend continues to receive the original `speaker` channel and numeric
`speaker_idx` in its transcript context. Session-local presentation names are
intentionally not persisted or sent to providers.

## File boundaries

### Modify

- `apps/native/src/components/listen/model.ts`
  - Add deterministic session display-name resolution and allow `buildBlocks` to
    consume resolved names.
  - Keep copy/export helpers driven by `TurnBlock.name`.
- `apps/native/src/components/ListenSection.tsx`
  - Own the override map, reset it on session changes, derive resolved names,
    and pass rename callbacks to child components.
- `apps/native/src/components/listen/SpeakerFilter.tsx`
  - Add inline editing for renameable chips while preserving filtering.
- `apps/native/src/components/listen/TranscriptBlocks.tsx`
  - Add inline editing for renameable block headers.
- `apps/native/src/components/listen/model.test.ts`
  - Cover default identity labels, stable numbering, overrides, and renamed
    copy/export output.

### Create

- `apps/native/src/components/listen/SpeakerNameEditor.tsx`
  - Shared inline editor component with keyboard, blur, empty-reset, and
    length-cap behavior.

## Error and edge behavior

- A missing or malformed `speaker_idx` never crashes rendering; it uses the
  existing anonymous fallback.
- A rename is presentation-only and cannot corrupt persisted transcript rows.
- Duplicate custom names are allowed because filtering remains keyed by
  identity, not label.
- A rename made while viewing a finished session disappears when leaving and
  reopening that session, matching the session-local requirement.

## Verification

Unit tests must prove:

1. Mic cluster `0` and unlabelled mic turns resolve to `You`.
2. Distinct non-You identities receive distinct first-seen anonymous labels.
3. The same identity keeps its label when it appears in multiple blocks.
4. An override changes every block and chip for that identity.
5. Empty overrides restore the generated default.
6. Plain-text copy and Markdown export use overridden names.
7. Existing block splitting, filtering keys, audio seeking, and color behavior
   remain unchanged.

Manual QA should cover live Listen output, a viewed historical session, inline
editing from both the transcript and chip row, Escape/blur/Enter behavior, copy,
Markdown export, and switching between sessions.

## Acceptance criteria

- The enrolled speaker is shown as `You`.
- Other diarized identities are anonymous until renamed by the user.
- Renaming one identity updates all of its visible transcript blocks and its
  filter chip immediately.
- Clipboard and Markdown exports contain the current session-local names.
- Leaving the session clears names without modifying SQLite or the raw
  transcript.
- Existing Listen behavior passes its full test suite and type checks.
