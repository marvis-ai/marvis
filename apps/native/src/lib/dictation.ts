/**
 * Pure helpers for tracking the dictated slice of the Ask input.
 * `applyDictationDraft` rewrites only the tracked range on every live
 * draft; `reconcileDictationEdit` classifies a user edit against that
 * range — shift the anchor when the edit lands before it, report a stop
 * condition when it touches the dictated text. No React/Tauri imports so
 * the whole file stays unit-testable.
 */

/** Half-open `[start, start + length)` span of dictated text inside the
 * input value. `length: 0` is the caret/selection anchor before the first
 * draft arrives. */
export interface DictationRange {
  start: number;
  length: number;
}

export interface ApplyDictationDraftResult {
  /** New input value with the tracked range replaced by `draft`. */
  value: string;
  /** Caret position — always right after the dictated text. */
  caret: number;
  /** Updated tracked range covering exactly the inserted draft. */
  range: DictationRange;
}

/**
 * Replace the tracked dictated range with the latest draft. Dictation
 * drafts are cumulative snapshots (committed finals + current interim),
 * so each one rewrites the whole tracked slice — prefix and suffix text
 * outside the range is never touched.
 */
export const applyDictationDraft = (
  input: string,
  range: DictationRange,
  draft: string,
): ApplyDictationDraftResult => ({
  value:
    input.slice(0, range.start) +
    draft +
    input.slice(range.start + range.length),
  caret: range.start + draft.length,
  range: { start: range.start, length: draft.length },
});

export interface ReconcileDictationEditResult {
  /** Tracked range after the edit — shifted by the edit's length delta
   * when the change landed entirely before it. */
  range: DictationRange;
  /** `true` when the edit touches the dictated slice: the caller must
   * stop dictation and keep the user's edit (the next draft would
   * otherwise clobber it). */
  intersects: boolean;
}

export interface DictationSelection {
  start: number;
  end: number;
}

/**
 * Map the input's selection across a draft replacement. The dictated
 * slice is the only changed text, so positions before it stay put,
 * positions after it shift by the draft-length delta, and positions
 * inside it are clamped into the replacement. `null` means no usable
 * selection was observed — fall back to the normal dictation caret
 * after the inserted draft.
 */
export const selectionAfterDictationDraft = (
  selection: DictationSelection | null,
  range: DictationRange,
  draftLength: number,
): DictationSelection => {
  const rangeEnd = range.start + range.length;
  const map = (position: number) => {
    if (position <= range.start) {
      return position;
    }
    if (position >= rangeEnd) {
      return position + draftLength - range.length;
    }
    return range.start + Math.min(position - range.start, draftLength);
  };
  if (selection === null) {
    const caret = range.start + draftLength;
    return { start: caret, end: caret };
  }
  const start = map(selection.start);
  const end = map(selection.end);
  return { start: Math.min(start, end), end: Math.max(start, end) };
};

/**
 * Classify a user edit (`prev` → `next`, caret at `nextCaret`) against the
 * live dictated range.
 *
 * The minimal changed region comes from the shared prefix/suffix, but it
 * is anchored at `nextCaret` rather than the raw diff offset: typed or
 * pasted text always ends at the caret and deletions leave the caret at
 * the removal point. Anchoring matters when boundary characters repeat —
 * an 'a' typed inside a dictated 'aa' diffs as an append at the end, yet
 * the caret still places it where the user actually typed.
 *
 * Boundary convention: an insertion exactly at `range.start` counts as
 * "before" (the anchor shifts so the new text stays a prefix) and one at
 * the range's end counts as "after" — either way the user's characters
 * land outside the dictated slice. Anything overlapping the slice itself
 * is `intersects: true`. A non-finite caret falls through to `intersects`
 * — stopping is the safe direction.
 */
export const reconcileDictationEdit = (
  prev: string,
  next: string,
  nextCaret: number,
  range: DictationRange,
): ReconcileDictationEditResult => {
  if (prev === next) {
    return { range, intersects: false };
  }

  let prefix = 0;
  const maxPrefix = Math.min(prev.length, next.length);
  while (prefix < maxPrefix && prev[prefix] === next[prefix]) {
    prefix += 1;
  }

  let suffix = 0;
  const maxSuffix = Math.min(prev.length - prefix, next.length - prefix);
  while (
    suffix < maxSuffix &&
    prev[prev.length - 1 - suffix] === next[next.length - 1 - suffix]
  ) {
    suffix += 1;
  }

  const removed = prev.length - suffix - prefix;
  const inserted = next.length - suffix - prefix;
  const start = Math.max(
    0,
    Math.min(nextCaret - inserted, prev.length - removed),
  );
  const end = start + removed;
  const rangeEnd = range.start + range.length;

  if (end <= range.start) {
    return {
      range: {
        start: range.start + (inserted - removed),
        length: range.length,
      },
      intersects: false,
    };
  }
  if (start >= rangeEnd) {
    return { range, intersects: false };
  }
  return { range, intersects: true };
};
