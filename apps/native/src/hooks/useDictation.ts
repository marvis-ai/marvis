/**
 * Dictation-into-Ask controller — owns the `dictation:*` session state,
 * the tracked input slice live drafts rewrite, and the pending-stop
 * handshake that lets the user keep editing while `dictation_stop`
 * settles.
 *
 * While a dictation runs, `dictationRange` marks the dictated slice so
 * live drafts rewrite only their own text — a user edit inside it stops
 * the session and keeps the edit, and Enter stops for review instead of
 * sending (dictation never auto-submits). Bar keeps `text`, `textRef`,
 * and `inputRef`; everything dictation-specific lives here.
 */
import { useEffect, useRef, useState } from 'react';
import type { ChangeEvent, RefObject, SyntheticEvent } from 'react';
import {
  dictationStart,
  dictationStatus,
  dictationStop,
  raise,
} from '@/lib/commands';
import {
  EV_DICTATION_DRAFT,
  EV_DICTATION_ERROR,
  EV_DICTATION_STATE,
  useTauriEvent,
  type DictationDraftPayload,
  type DictationErrorPayload,
  type DictationStatePayload,
} from '@/lib/events';
import {
  applyDictationDraft,
  reconcileDictationEdit,
  selectionAfterDictationDraft,
  type ApplyDictationDraftResult,
  type DictationRange,
  type DictationSelection,
} from '@/lib/dictation';

/** An in-flight `dictation_stop`. While the invoke settles the tracked
 *  slice is evolved per user `onChange` — with THAT edit's caret —
 *  instead of diffing old/new text at resolve time, so the returned
 *  final draft lands at the right place or not at all once the user
 *  takes the text over. A second Enter attaches to `promise` to submit
 *  behind the apply decision. */
interface PendingDictationStop {
  /** The dictated slice in current-text coordinates — shifted by each
   *  edit outside it; `null` when the stop began without an anchor
   *  (nothing can be applied then). */
  range: DictationRange | null;
  /** Whether the returned final draft may land — the stop's
   *  `applyFinal` intent, downgraded to `false` by an edit inside the
   *  slice, a wholesale clear/replace, Escape, type-to-wake, or a
   *  discard stop. */
  applyDraft: boolean;
  /** Whether a live draft had already replaced the slice — an empty
   *  final draft deletes the slice only then; otherwise the slice
   *  still holds the user's own text. */
  draftLanded: boolean;
  /** The invoke promise queued Enter attaches to. */
  promise: Promise<void>;
}

export const useDictation = ({
  text,
  textRef,
  setText,
  inputRef,
  inputRendered,
}: {
  /** The committed input value — the queued-selection restore keys off
   *  its changes. */
  text: string;
  /** Live mirror of `text` for Tauri event handlers — they fire outside
   *  React's batching, so the state variable (and the DOM) can lag a
   *  queued `setText`; the ref always reads back the latest value. */
  textRef: RefObject<string>;
  setText: (value: string) => void;
  inputRef: RefObject<HTMLTextAreaElement | null>;
  /** Whether the Ask `<input>` is actually mounted: the permission
   *  card and the boot-error retry replace the whole row while the bar
   *  stays expanded, so dictation visibility keys off this — an input
   *  that isn't rendered can't receive drafts and must not keep the
   *  mic held. */
  inputRendered: boolean;
}) => {
  const [state, setState] = useState<DictationStatePayload['state']>('idle');
  /** The `[start, start + length)` slice of `text` owned by the live
   *  dictation session — `null` when no session is tracked. Cleared the
   *  moment a stop begins so a late live draft or another stop can
   *  never touch the input again. */
  const dictationRange = useRef<DictationRange | null>(null);
  /** Selection to restore after the next `text` commit — mapped across
   *  the draft replacement so user edits outside the dictated slice keep
   *  their position instead of jumping to its end. */
  const pendingSelection = useRef<DictationSelection | null>(null);
  /** A `dictation_start` in flight: a stale `dictation:state` snapshot
   *  emitted before it must not drop the freshly captured anchor. */
  const dictationStarting = useRef(false);
  /** Last `dictation:error` already surfaced — the `dictation_start`
   *  invoke rejects with the same message after the event, so the
   *  alert dedupes on it. */
  const lastDictationError = useRef<string | null>(null);
  /** Whether a live draft has already rewritten the tracked slice —
   *  until then the slice still holds the user's own (e.g. selected)
   *  text, so an empty final draft must leave it untouched. */
  const dictationDraftLanded = useRef(false);
  /** The input's last known selection — refreshed on `onSelect`, after
   *  each `onChange`, at programmatic caret restores, and initialized
   *  when a stop begins. While a stop is in flight it holds the
   *  PRE-edit selection for the next edit: one that covered the whole
   *  field marks a wholesale clear/replace the diff alone can't see
   *  (a boundary caret anchor never intersects it). */
  const inputSelection = useRef<DictationSelection | null>(null);
  /** The in-flight `dictation_stop` — evolved per edit while it
   *  settles; also the handle a second Enter queues its submit on. */
  const dictationStopPending = useRef<PendingDictationStop | null>(null);

  /** Snapshot the input's caret/selection as the dictation anchor — a
   *  selected range is replaced by the draft, a bare caret inserts.
   *  Falls back to the end of the text when the field isn't mounted. */
  const captureAnchor = () => {
    const input = inputRef.current;
    const start = input?.selectionStart ?? textRef.current.length;
    const end = input?.selectionEnd ?? start;
    dictationRange.current = { start, length: Math.max(0, end - start) };
    dictationDraftLanded.current = false;
    inputSelection.current = { start, end };
  };

  /** Commit a draft application to the input. The current selection is
   *  mapped across the replaced slice instead of always jumping to the
   *  draft end: a caret following dictation still follows it, while text
   *  the user selected or typed outside the slice keeps its position.
   *  When the value doesn't change React skips the commit, so restore
   *  immediately instead of leaving a stale queued selection. */
  const commitDictationText = (
    applied: ApplyDictationDraftResult,
    previousRange: DictationRange,
  ) => {
    const nextSelection = selectionAfterDictationDraft(
      inputSelection.current,
      previousRange,
      applied.range.length,
    );
    if (applied.value === textRef.current) {
      pendingSelection.current = null;
      inputRef.current?.setSelectionRange(
        nextSelection.start,
        nextSelection.end,
      );
      inputSelection.current = nextSelection;
      return;
    }
    pendingSelection.current = nextSelection;
    setText(applied.value);
  };

  /** Stop the live dictation session. The tracked range moves into a
   *  pending-stop object and `dictationRange` is cleared up front so
   *  a late live draft can never rewrite the input again. While the
   *  `dictation_stop` invoke is in flight it blocks server-side
   *  across the worker join, so the user can keep editing — each
   *  `onChange` reconciles against `pending.range` with THAT edit's
   *  caret: edits outside shift where the final draft lands, edits
   *  inside it or a wholesale clear/replace (Escape, type-to-wake,
   *  full-selection overwrite, emptying the field) downgrade
   *  `applyDraft` so the returned draft is discarded and the user's
   *  version wins. If no live draft ever landed (e.g. Enter during
   *  an in-flight start) the slice still holds the user's selected
   *  text — an empty final must not delete it. A second stop joins
   *  the pending one instead of re-invoking; a queued Enter attaches
   *  to `promise` to submit behind the apply decision. */
  const stop = (applyFinal: boolean): Promise<void> => {
    const existing = dictationStopPending.current;
    if (existing !== null) {
      // A stop is already in flight — a redundant invoke can't return
      // a different draft. A discard stop downgrades the pending
      // apply; an apply stop leaves the earlier decision alone. Clear
      // any newer anchor too: while this stop is settling there is no
      // safe way to target two sessions.
      dictationRange.current = null;
      if (!applyFinal) {
        existing.applyDraft = false;
      }
      return existing.promise;
    }
    const range = dictationRange.current;
    dictationRange.current = null;
    if (range === null && state !== 'listening') {
      return Promise.resolve();
    }
    // Seed the selection tracker so the first mid-flight edit sees
    // the selection the edit was made with.
    const input = inputRef.current;
    inputSelection.current = input
      ? { start: input.selectionStart ?? 0, end: input.selectionEnd ?? 0 }
      : null;
    const pending: PendingDictationStop = {
      range,
      applyDraft: applyFinal && range !== null,
      draftLanded: dictationDraftLanded.current,
      // Replaced below once the invoke chain exists — the field lets
      // the chain's callbacks self-reference `pending`.
      promise: Promise.resolve(),
    };
    const promise = dictationStop()
      .then((draft) => {
        const finalRange = pending.range;
        if (!pending.applyDraft || finalRange === null) {
          return;
        }
        if (draft.text === '' && !pending.draftLanded) {
          return;
        }
        commitDictationText(
          applyDictationDraft(textRef.current, finalRange, draft.text),
          finalRange,
        );
        if (inputRendered) {
          inputRef.current?.focus();
        }
      })
      .catch(() => raise('Stop failed'));
    pending.promise = promise;
    dictationStopPending.current = pending;
    void promise.finally(() => {
      if (dictationStopPending.current === pending) {
        dictationStopPending.current = null;
      }
    });
    return promise;
  };

  /** The live-session branch of the mic press — the anchor is captured
   *  before the invoke so a draft can never race ahead of it. The
   *  caller holds its `speechBusy` lock across the returned promise. */
  const start = (): Promise<void> => {
    captureAnchor();
    dictationStarting.current = true;
    return dictationStart()
      .then((next) => {
        setState(next.state);
        if (next.state !== 'listening') {
          dictationRange.current = null;
        } else if (dictationRange.current === null) {
          // A stop landed while the start was in flight —
          // leave no live session behind.
          void dictationStop().catch(() => {});
        }
      })
      .catch((e: unknown) => {
        // A deliberate stop landing mid-start rejects the
        // invoke without failing: the anchor is already gone,
        // or the backend aborted the commit with its internal
        // 'dictation start was interrupted' marker — both are
        // expected abandon/gate transitions, not alerts. Real
        // failures already emitted `dictation:error`, so
        // dedupe on it.
        const message = typeof e === 'string' ? e : 'Dictation failed';
        const abandoned =
          dictationRange.current === null ||
          message === 'dictation start was interrupted';
        dictationRange.current = null;
        if (!abandoned && message !== lastDictationError.current) {
          raise(message);
        }
      })
      .finally(() => {
        dictationStarting.current = false;
      });
  };

  /** The stop-first half of a mic press: a live session or an in-flight
   *  stop yields its settle promise; otherwise `null` and the press
   *  continues to a start. */
  const stopIfActive = (): Promise<void> | null =>
    state === 'listening' || dictationStopPending.current !== null
      ? stop(true)
      : null;

  /** Discard the session's claim on the text — Escape clears the field
   *  and type-to-wake replaces it, so a pending stop's returned draft
   *  must not land and a live anchor stops without applying. */
  const discard = () => {
    const pendingStop = dictationStopPending.current;
    if (pendingStop !== null) {
      pendingStop.applyDraft = false;
    }
    if (dictationRange.current !== null) {
      void stop(false);
    }
  };

  /** Enter's contract: a stop already settling queues the send behind
   *  the draft application (a session anchored or stopping meanwhile
   *  vetoes it); a live session ends for review instead — dictation
   *  never auto-submits; idle text sends. */
  const submit = (send: () => void) => {
    const pendingStop = dictationStopPending.current;
    if (pendingStop !== null) {
      void pendingStop.promise.then(() => {
        if (
          dictationRange.current === null &&
          dictationStopPending.current === null
        ) {
          send();
        }
      });
      return;
    }
    if (state === 'listening' || dictationRange.current !== null) {
      void stop(true);
      return;
    }
    send();
  };

  /** The textarea's `onChange` — a keystroke commits its own caret
   *  (a queued dictation-selection restore must not jump it to the
   *  dictated slice), then the edit is reconciled against the live
   *  anchor or the in-flight stop's tracked slice. */
  const handleChange = (e: ChangeEvent<HTMLTextAreaElement>) => {
    pendingSelection.current = null;
    const prev = textRef.current;
    const next = e.target.value;
    const caret = e.target.selectionEnd ?? next.length;
    const range = dictationRange.current;
    if (range !== null) {
      const edit = reconcileDictationEdit(prev, next, caret, range);
      if (edit.intersects) {
        // The edit touches dictated text — stop the session and
        // keep the user's version; the returning final draft is
        // discarded.
        void stop(false);
      } else {
        dictationRange.current = edit.range;
      }
    } else {
      const pendingStop = dictationStopPending.current;
      if (
        pendingStop !== null &&
        pendingStop.applyDraft &&
        pendingStop.range !== null
      ) {
        // A stop is in flight — evolve its tracked slice per
        // edit (with THIS edit's caret, not a resolve-time
        // read): outside edits shift where the final draft
        // lands; inside edits, a full-selection overwrite, or
        // clearing the field discard it.
        const sel = inputSelection.current;
        const wholesale =
          (prev !== '' && next === '') ||
          (prev !== '' &&
            sel !== null &&
            sel.start === 0 &&
            sel.end === prev.length);
        const edit = reconcileDictationEdit(
          prev,
          next,
          caret,
          pendingStop.range,
        );
        if (wholesale || edit.intersects) {
          pendingStop.applyDraft = false;
        } else {
          pendingStop.range = edit.range;
        }
      }
    }
    inputSelection.current = {
      start: e.target.selectionStart ?? caret,
      end: caret,
    };
    setText(next);
  };

  /** The textarea's `onSelect` — keeps the last-known-selection tracker
   *  current for draft-application mapping and wholesale-edit checks. */
  const handleSelect = (e: SyntheticEvent<HTMLTextAreaElement>) => {
    inputSelection.current = {
      start: e.currentTarget.selectionStart ?? 0,
      end: e.currentTarget.selectionEnd ?? 0,
    };
  };

  // Mount resync — a live session can outlive this webview.
  useEffect(() => {
    void dictationStatus()
      .then((next) => {
        setState(next.state);
        if (next.state === 'listening') {
          // The session outlived this webview, so no anchor was
          // captured at start — attach at the caret/end: later drafts
          // extend the text instead of rewriting unknown characters.
          captureAnchor();
        }
      })
      .catch(() => {});
  }, []);

  useTauriEvent<DictationStatePayload>(EV_DICTATION_STATE, (p) => {
    setState(p.state);
    if (p.state !== 'listening' && !dictationStarting.current) {
      // Unsolicited idle (e.g. `leave_main`) or error: the session is
      // gone and no final draft is coming — drop the anchor and keep
      // the text as it stands.
      dictationRange.current = null;
    }
  });
  useTauriEvent<DictationDraftPayload>(EV_DICTATION_DRAFT, (p) => {
    const range = dictationRange.current;
    // Only live snapshots apply: the authoritative draft arrives as the
    // `dictation_stop` return value, and once a stop begins the range
    // is already gone — a late event must not clobber the final text.
    if (p.final || range === null) {
      return;
    }
    const applied = applyDictationDraft(textRef.current, range, p.text);
    dictationRange.current = applied.range;
    dictationDraftLanded.current = true;
    commitDictationText(applied, range);
  });
  // `dictation:error` doubles as the `dictation_start` rejection
  // message — `lastDictationError` dedupes the alert.
  useTauriEvent<DictationErrorPayload>(EV_DICTATION_ERROR, (p) => {
    lastDictationError.current = p.message;
    raise(p.message);
  });

  // Dictation writes go through `setText` like any edit; once the value
  // commits, restore the selection mapped across the dictated slice.
  useEffect(() => {
    if (pendingSelection.current === null) {
      return;
    }
    const selection = pendingSelection.current;
    pendingSelection.current = null;
    inputRef.current?.setSelectionRange(selection.start, selection.end);
    inputSelection.current = selection;
  }, [text, inputRef]);
  // Dictation is bound to the visible Ask input — when the input leaves
  // the DOM (collapse, `ask_close`, permission/boot-error cards) stop
  // the session and keep the final draft for review. `state` is a dep
  // so a session resynced while the input is hidden is stopped too
  // instead of holding the mic invisibly.
  useEffect(() => {
    if (!inputRendered) {
      void stop(true);
    }
  }, [inputRendered, state]);

  return {
    state,
    start,
    stopIfActive,
    discard,
    submit,
    handleChange,
    handleSelect,
  };
};
