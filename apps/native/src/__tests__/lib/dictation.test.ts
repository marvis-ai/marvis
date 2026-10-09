/// <reference types="bun-types" />
import { describe, expect, test } from 'bun:test';
import {
  applyDictationDraft,
  reconcileDictationEdit,
  selectionAfterDictationDraft,
} from '@/lib/dictation';

describe('applyDictationDraft', () => {
  // The tracked range marks the dictated slice; each live draft replaces it
  // wholesale so interim STT corrections never duplicate text.
  test('inserts at the caret when the range is empty', () => {
    expect(
      applyDictationDraft('hello ', { start: 6, length: 0 }, 'world'),
    ).toEqual({
      value: 'hello world',
      caret: 11,
      range: { start: 6, length: 5 },
    });
  });

  test('replaces a selected range like typed text', () => {
    // Dictation start over a selection behaves like typing: the selection
    // becomes the tracked range and the first draft replaces it.
    expect(
      applyDictationDraft('say world now', { start: 4, length: 5 }, 'there'),
    ).toEqual({
      value: 'say there now',
      caret: 9,
      range: { start: 4, length: 5 },
    });
  });

  test('rewrites only the tracked range on repeated live updates', () => {
    let input = 'note: ';
    let range = { start: 6, length: 0 };
    for (const draft of ['the', 'the quick', 'the quick brown']) {
      const applied = applyDictationDraft(input, range, draft);
      input = applied.value;
      range = applied.range;
    }
    expect(input).toBe('note: the quick brown');
    expect(range).toEqual({ start: 6, length: 15 });
  });

  test('preserves the prefix and suffix around the dictated range', () => {
    const applied = applyDictationDraft(
      'todo old draft done',
      { start: 5, length: 9 },
      'new',
    );
    expect(applied.value).toBe('todo new done');
    expect(applied.caret).toBe(8);
    expect(applied.range).toEqual({ start: 5, length: 3 });
  });

  test('clears the tracked slice on an empty draft', () => {
    // An empty final draft (nothing recognized) must remove the interim
    // text rather than leave a stale tail behind.
    expect(
      applyDictationDraft('abc interim xyz', { start: 4, length: 7 }, ''),
    ).toEqual({
      value: 'abc  xyz',
      caret: 4,
      range: { start: 4, length: 0 },
    });
  });
});

describe('reconcileDictationEdit', () => {
  // 'note: ' is the user's own prefix; 'the quick brown' is the live draft.
  const prev = 'note: the quick brown';
  const range = { start: 6, length: 15 };

  test('shifts the tracked range when the edit lands before it', () => {
    // Insert '> ' at position 0 — the anchor must keep pointing at the same
    // dictated characters so the next draft still rewrites only them.
    expect(reconcileDictationEdit(prev, `> ${prev}`, 2, range)).toEqual({
      range: { start: 8, length: 15 },
      intersects: false,
    });
  });

  test('shifts the range backwards on a deletion before it', () => {
    // Deleting 'note' pulls the dictated text left with its anchor.
    expect(reconcileDictationEdit(prev, ': the quick brown', 0, range)).toEqual(
      {
        range: { start: 2, length: 15 },
        intersects: false,
      },
    );
  });

  test('leaves the range alone when the edit lands after it', () => {
    expect(reconcileDictationEdit(prev, `${prev}!`, 22, range)).toEqual({
      range,
      intersects: false,
    });
  });

  test('reports an insertion inside the range as a stop condition', () => {
    // 'X' typed mid-draft — the next live update would clobber the user's
    // character, so the caller stops dictation and keeps the edit.
    const next = 'note: thXe quick brown';
    expect(reconcileDictationEdit(prev, next, 9, range)).toEqual({
      range,
      intersects: true,
    });
  });

  test('reports a selection replace inside the range as a stop condition', () => {
    // Selecting 'quick' and typing 'slow' rewrites dictated characters —
    // dictation must stop; the user's text wins.
    const next = 'note: the slow brown';
    expect(reconcileDictationEdit(prev, next, 14, range)).toEqual({
      range,
      intersects: true,
    });
  });

  test('anchors boundary inserts on the caret, not the raw diff', () => {
    // A bare diff reports the 'a' as appended at the end; the caret proves
    // it was typed at position 0 — shifting keeps it out of the next draft.
    expect(
      reconcileDictationEdit('aa', 'aaa', 1, { start: 0, length: 2 }),
    ).toEqual({
      range: { start: 1, length: 2 },
      intersects: false,
    });
  });
});

describe('selectionAfterDictationDraft', () => {
  const range = { start: 6, length: 15 };

  test('moves a caret at an empty insertion anchor with the draft', () => {
    // The standard dictation start is `{start: caret, length: 0}` — the
    // caret must land after the inserted draft, not stay pinned before it.
    expect(
      selectionAfterDictationDraft(
        { start: 6, end: 6 },
        { start: 6, length: 0 },
        3,
      ),
    ).toEqual({ start: 9, end: 9 });
  });

  test('follows a caret at the end of the dictated slice', () => {
    expect(
      selectionAfterDictationDraft({ start: 21, end: 21 }, range, 18),
    ).toEqual({ start: 24, end: 24 });
  });

  test('keeps a caret before the dictated slice', () => {
    expect(
      selectionAfterDictationDraft({ start: 2, end: 2 }, range, 18),
    ).toEqual({ start: 2, end: 2 });
  });

  test('shifts a caret after the dictated slice by the draft delta', () => {
    // A user character typed just after 'brown' must remain after it when
    // the next draft grows — restoring to the draft end would reorder it.
    expect(
      selectionAfterDictationDraft({ start: 22, end: 22 }, range, 18),
    ).toEqual({ start: 25, end: 25 });
  });

  test('clamps a caret inside the dictated slice into its replacement', () => {
    expect(
      selectionAfterDictationDraft({ start: 20, end: 20 }, range, 4),
    ).toEqual({ start: 10, end: 10 });
  });

  test('maps a selection across the replaced slice', () => {
    expect(
      selectionAfterDictationDraft({ start: 4, end: 22 }, range, 18),
    ).toEqual({ start: 4, end: 25 });
  });

  test('uses the dictated caret when no selection was observed', () => {
    expect(selectionAfterDictationDraft(null, range, 18)).toEqual({
      start: 24,
      end: 24,
    });
  });
});
