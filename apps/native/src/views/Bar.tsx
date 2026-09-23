/**
 * The always-on-top bar (`?view=bar`) — the UNIFIED window. Two shapes:
 *
 *  - Pill modes (mini | input | permission): the 136⇄480×64 capsule⇄
 *    input morph — the capsule IS the window under liquid glass, so
 *    `expanded` reports to `window_set_bar_expanded` and Rust animates
 *    the width change. Unchanged mechanics.
 *  - Card modes (chat | listen): the same window grown to
 *    600×(64+content) — the bar row becomes the card's header, pinned
 *    to the anchored edge (`flex-col-reverse` when growing up puts the
 *    row at the bottom and it never visually jumps).
 *
 * `cardOpen` is read off the window itself: `innerHeight > BAR_H` means
 * Rust expanded us — `set_chat_open` emits nothing by design, so the
 * `resize` event is the single open/close signal for every path
 * (Cmd+/, ask send, `ask_close`, `window_set_chat_open`).
 *
 * `growDir` is detected at expand time: the anchored edge is fixed for
 * grow-down and rises for grow-up, so the first expanded y-read compared
 * to the last collapsed y gives the direction. While collapsed the
 * baseline refreshes on every `tauri://move`/`resize` tick.
 *
 * `data-tauri-drag-region` lives on the bar ROW only in card mode — a
 * stage-level region would intercept text selection in the scrollable
 * conversation. In pill mode it stays on the capsule chrome as before.
 *
 * The mic button routes by surface: with the Ask input visible it
 * dictates into the field (`dictation:*` — transient, nothing
 * persists); collapsed it opens the card into meeting Listen. While a
 * dictation runs, `dictationRange` marks the dictated slice so live
 * drafts rewrite only their own text — a user edit inside it stops the
 * session and keeps the edit, and Enter stops for review instead of
 * sending (dictation never auto-submits).
 *
 * Errors go to the `alert` window (`alertShow`) — the pill has no room.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { SubmitEvent } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  ArrowLeftIcon,
  CameraIcon,
  GripVerticalIcon,
  MicIcon,
  SettingsIcon,
  ShieldAlertIcon,
  ShineBorder,
  cn,
} from '@marvis/ui';
import {
  alertShow,
  askClose,
  askSend,
  askSendScreenOnly,
  dictationStart,
  dictationStatus,
  dictationStop,
  permissionsOpenPrefs,
  permissionsRequestScreen,
  permissionsStatus,
  listenStart,
  listenStop,
  listenStatus,
  windowAdjustHeight,
  windowSetBarExpanded,
  windowSetChatOpen,
  windowShowSettings,
  type AppStatePayload,
  type Gate,
} from '../lib/commands';
import {
  EV_ASK_STATE,
  EV_APP_STATE,
  EV_DICTATION_DRAFT,
  EV_DICTATION_ERROR,
  EV_DICTATION_STATE,
  EV_LISTEN_ERROR,
  EV_LISTEN_STATE,
  type DictationDraftPayload,
  type DictationErrorPayload,
  type DictationStatePayload,
  type ListenStatePayload,
  EV_CAPTURE_PERMISSION_NEEDED,
  useTauriEvent,
} from '../lib/events';
import {
  applyDictationDraft,
  reconcileDictationEdit,
  type ApplyDictationDraftResult,
  type DictationRange,
} from '../lib/dictation';
import { ChatSection } from '../components/ChatSection';
import { Iris } from '../components/Iris';
import { ListenSection } from '../components/ListenSection';
import { RetryCard } from '../components/RetryCard';
import {
  BTN_LINK,
  BTN_LINK_SM,
  BTN_PRIMARY,
  BTN_SM,
  ICON_BTN,
  PANEL,
} from '../lib/classes';

/** Mirrors `app_gate` in lib.rs so the first render doesn't wait on `app:state`. */
const gateFor = (screen: boolean): Gate =>
  screen ? 'main' : 'needs_permission';

/** Every bar error goes to the alert window — the pill has no room. */
const raise = (message: string) => void alertShow(message).catch(() => {});

/** The bar row's height — the pill's window height in pill modes and
 *  the card header's height in card modes (spec: 64). */
const BAR_H = 64;
/** Card-open read: any window taller than the pill is a card. */
const OPEN_EPS = 2;
/** Card's max-height so the reported height never exceeds Rust's 900
 *  cap — frost keeps the stage's `p-1` (8 px of chrome). */
const CARD_MAX = 900 - 8;
/** Reported-height deadband + invoke throttle (was AskPanel's). */
const HEIGHT_EPS = 4;
const HEIGHT_MS = 150;

/** Bar controls step up from the 26px overlay default (`ICON_BTN`) —
 *  the capsule is 64px tall, so buttons/icons scale ~1.3×; `fg-2` reads
 *  better than `muted` on glass. */
const BAR_BTN = cn(ICON_BTN, 'size-8.5 text-fg-2');

const grip = (
  <span
    className='-mx-0.75 grid w-4 max-w-0 flex-none cursor-grab place-items-center overflow-hidden text-muted-foreground opacity-0 transition-[max-width_var(--motion-base)_var(--ease),opacity_var(--motion-fast)_var(--ease),margin-inline_var(--motion-base)_var(--ease)] group-hover/bar:mx-0 group-hover/bar:max-w-4 group-hover/bar:opacity-100 active:cursor-grabbing motion-reduce:transition-none'
    data-tauri-drag-region='deep'
    title='Drag'
    aria-hidden='true'>
    <GripVerticalIcon className='size-3.75' />
  </span>
);

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

const Bar = () => {
  const [gate, setGate] = useState<Gate | null>(null);
  const [bootError, setBootError] = useState(false);
  const [busy, setBusy] = useState(false);
  const [text, setTextState] = useState('');
  /** Live mirror of `text` for Tauri event handlers — they fire outside
   *  React's batching, so the state variable (and the DOM) can lag a
   *  queued `setText`; the ref is written with every update and always
   *  reads back the latest value. */
  const textRef = useRef('');
  const setText = (value: string) => {
    textRef.current = value;
    setTextState(value);
  };
  const [open, setOpen] = useState(false);
  const [cardOpen, setCardOpen] = useState(
    () => window.innerHeight > BAR_H + OPEN_EPS,
  );
  const [growDir, setGrowDir] = useState<'up' | 'down'>('down');
  const [listenWanted, setListenWanted] = useState(false);
  const [listenState, setListenState] =
    useState<ListenStatePayload['state']>('idle');
  const [dictationState, setDictationState] =
    useState<DictationStatePayload['state']>('idle');
  const inputRef = useRef<HTMLInputElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const cardRef = useRef<HTMLDivElement>(null);
  /** Last collapsed-mode outer y — the baseline the expand direction
   *  is detected against. */
  const collapsedY = useRef<number | null>(null);
  /** Serializes mic-button start/stop across both speech modes — a new
   *  start must never race an in-flight stop (dictation and Listen are
   *  mutually exclusive server-side). */
  const speechBusy = useRef(false);
  /** The `[start, start + length)` slice of `text` owned by the live
   *  dictation session — `null` when no session is tracked. Cleared the
   *  moment a stop begins so a late live draft or another stop can
   *  never touch the input again. */
  const dictationRange = useRef<DictationRange | null>(null);
  /** Caret to restore after the next `text` commit — dictation keeps
   *  it right after the dictated slice. */
  const pendingCaret = useRef<number | null>(null);
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
  const inputSelection = useRef<{ start: number; end: number } | null>(null);
  /** The in-flight `dictation_stop` — evolved per edit while it
   *  settles; also the handle a second Enter queues its submit on. */
  const dictationStopPending = useRef<PendingDictationStop | null>(null);

  // Icon-row ⇄ input-row swap: the gate card and boot errors count as
  // expanded; `main` rests as the icon row until the iris opens it or
  // the user starts typing on the focused window.
  const expanded = bootError
    ? true
    : gate === 'main'
      ? open || text.length > 0
      : gate !== null;
  /** The row shows the input row in card modes regardless of `open` —
   *  it's the card header and the follow-up field. */
  const showInputRow = expanded || cardOpen;
  /** Whether the Ask `<input>` is actually mounted: the permission
   *  card and the boot-error retry replace the whole row while
   *  `showInputRow` stays true, so dictation visibility keys off
   *  this — an input that isn't rendered can't receive drafts and
   *  must not keep the mic held. */
  const inputRendered =
    showInputRow && !bootError && gate !== 'needs_permission';
  const section: 'chat' | 'listen' | null = !cardOpen
    ? null
    : listenWanted
      ? 'listen'
      : 'chat';

  /** Snapshot the input's caret/selection as the dictation anchor — a
   *  selected range is replaced by the draft, a bare caret inserts.
   *  Falls back to the end of the text when the field isn't mounted. */
  const captureDictationAnchor = () => {
    const input = inputRef.current;
    const start = input?.selectionStart ?? textRef.current.length;
    const end = input?.selectionEnd ?? start;
    dictationRange.current = { start, length: Math.max(0, end - start) };
    dictationDraftLanded.current = false;
    inputSelection.current = { start, end };
  };

  /** Commit a draft application to the input. When the value actually
   *  changes the caret is queued for the `[text]`-keyed restore
   *  effect; when it doesn't, React skips the commit and a queued
   *  caret would go stale — restore the selection immediately
   *  instead. */
  const commitDictationText = (applied: ApplyDictationDraftResult) => {
    if (applied.value === textRef.current) {
      pendingCaret.current = null;
      inputRef.current?.setSelectionRange(applied.caret, applied.caret);
      inputSelection.current = { start: applied.caret, end: applied.caret };
      return;
    }
    pendingCaret.current = applied.caret;
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
  const stopDictation = (applyFinal: boolean) => {
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
    if (range === null && dictationState !== 'listening') {
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

  const bootstrap = useCallback(async () => {
    try {
      const perms = await permissionsStatus();
      setGate(gateFor(perms.screen));
      setBootError(false);
    } catch {
      setBootError(true);
    }
  }, []);

  useEffect(() => {
    void bootstrap();
    void listenStatus()
      .then((next) => {
        setListenState(next.state);
        if (next.state === 'listening' || next.state === 'error') {
          setListenWanted(true);
        }
      })
      .catch(() => {});
    void dictationStatus()
      .then((next) => {
        setDictationState(next.state);
        if (next.state === 'listening') {
          // The session outlived this webview, so no anchor was
          // captured at start — attach at the caret/end: later drafts
          // extend the text instead of rewriting unknown characters.
          captureDictationAnchor();
        }
      })
      .catch(() => {});
  }, [bootstrap]);

  useTauriEvent<AppStatePayload>(EV_APP_STATE, (p) => setGate(p.gate));
  useTauriEvent<ListenStatePayload>(EV_LISTEN_STATE, (p) => {
    setListenState(p.state);
    if (p.state === 'listening' || p.state === 'error') {
      setListenWanted(true);
    } else if (p.state === 'idle') {
      setListenWanted(false);
    }
  });
  // Setup failures are emitted separately before the durable state snapshot;
  // switch to Listen immediately and let the snapshot/cold-open status resync
  // preserve the error after this webview mounts or reopens.
  useTauriEvent(EV_LISTEN_ERROR, () => {
    setListenState('error');
    setListenWanted(true);
  });
  useTauriEvent<DictationStatePayload>(EV_DICTATION_STATE, (p) => {
    setDictationState(p.state);
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
    commitDictationText(applied);
  });
  // `dictation:error` doubles as the `dictation_start` rejection
  // message — `lastDictationError` dedupes the alert.
  useTauriEvent<DictationErrorPayload>(EV_DICTATION_ERROR, (p) => {
    lastDictationError.current = p.message;
    raise(p.message);
  });
  // Mid-session screen-permission revocation (ask.rs detects it when a
  // stale frame would have shipped): collapse the card — NOT `askClose`,
  // which would cancel the text-only fallback — and show the
  // permission card.
  useTauriEvent<{ permission: string }>(EV_CAPTURE_PERMISSION_NEEDED, () => {
    void windowSetChatOpen(false).catch(() => {});
    setGate('needs_permission');
  });
  // A send while in listen mode reasserts chat (`loading` = a run).
  useTauriEvent<{ state: string }>(EV_ASK_STATE, (p) => {
    if (p.state === 'loading') {
      setListenWanted(false);
    }
  });

  // Card open/close is learned from the window itself; grow direction
  // from the y-delta at expand time. While collapsed the baseline y
  // refreshes on move + resize ticks (a drag moves without resizing).
  useEffect(() => {
    const win = getCurrentWindow();
    let alive = true;
    const unMove = win.onMoved((e) => {
      if (window.innerHeight <= BAR_H + OPEN_EPS) {
        collapsedY.current = e.payload.y;
      }
    });
    const read = () => {
      const openNow = window.innerHeight > BAR_H + OPEN_EPS;
      void win
        .outerPosition()
        .then((p) => {
          if (!alive) {
            return;
          }
          setCardOpen((was) => {
            if (openNow && !was && collapsedY.current !== null) {
              setGrowDir(p.y < collapsedY.current - 0.5 ? 'up' : 'down');
            }
            return openNow;
          });
          if (!openNow) {
            collapsedY.current = p.y;
          }
        })
        .catch(() => setCardOpen(openNow));
    };
    window.addEventListener('resize', read);
    read();
    return () => {
      alive = false;
      window.removeEventListener('resize', read);
      void unMove.then((u) => u());
    };
  }, []);

  // Listen is an independent session: collapsing the card must not change
  // which active section is shown when it is reopened.

  // The capsule IS the window under liquid glass — the pill⇄input morph
  // resizes it (idle 136 ⇄ 480). While the card is open the morph is
  // dormant: the width report is skipped so `bar_rect` (the canonical
  // pill) restores verbatim on collapse.
  useEffect(() => {
    if (!cardOpen) {
      void windowSetBarExpanded(expanded).catch(() => {});
    }
  }, [expanded, cardOpen]);

  // Report the card's desired TOTAL window height: leading + trailing
  // throttle, only on a real (>EPS) change — `adjust_height` animates
  // per call. The observer watches the CARD element (content-sized,
  // capped at CARD_MAX) — NOT the window-fixed `h-full` stage, whose
  // box only changes on real window resizes, so streaming content
  // growth/shrink actually triggers reports. `scrollHeight` reads the
  // uncapped content height (overflow counts); the backend clamps to
  // min(900, free). The stage's vertical padding is added back (frost
  // keeps `p-1`, glass strips it) so the report is total window height
  // under both materials.
  useEffect(() => {
    const el = cardRef.current;
    const stage = stageRef.current;
    if (!el || !cardOpen) {
      return;
    }
    let lastValue = -1;
    let lastSentAt = 0;
    let timer: number | undefined;
    const report = () => {
      const cs = stage ? getComputedStyle(stage) : null;
      const padY = cs
        ? parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom)
        : 0;
      const h = Math.min(
        Math.ceil(el.scrollHeight + (Number.isFinite(padY) ? padY : 0)),
        900,
      );
      if (Math.abs(h - lastValue) <= HEIGHT_EPS) {
        return;
      }
      const wait = HEIGHT_MS - (Date.now() - lastSentAt);
      if (wait <= 0) {
        lastValue = h;
        lastSentAt = Date.now();
        void windowAdjustHeight(h).catch(() => {});
      } else if (timer === undefined) {
        timer = window.setTimeout(() => {
          timer = undefined;
          report();
        }, wait);
      }
    };
    const observer = new ResizeObserver(report);
    observer.observe(el);
    report();
    return () => {
      observer.disconnect();
      window.clearTimeout(timer);
    };
  }, [cardOpen]);

  // Focus the field whenever the input row shows.
  useEffect(() => {
    if (showInputRow && gate === 'main') {
      inputRef.current?.focus();
    }
  }, [showInputRow, gate]);

  // Dictation writes go through `setText` like any edit; once the value
  // commits, restore the caret to just after the dictated slice.
  useEffect(() => {
    if (pendingCaret.current === null) {
      return;
    }
    const caret = pendingCaret.current;
    pendingCaret.current = null;
    inputRef.current?.setSelectionRange(caret, caret);
    inputSelection.current = { start: caret, end: caret };
  }, [text]);

  // Dictation is bound to the visible Ask input — when the input leaves
  // the DOM (collapse, `ask_close`, permission/boot-error cards) stop
  // the session and keep the final draft for review. `dictationState`
  // is a dep so a session resynced while the input is hidden is
  // stopped too instead of holding the mic invisibly.
  useEffect(() => {
    if (!inputRendered) {
      void stopDictation(true);
    }
  }, [inputRendered, dictationState]);

  // Type-to-wake on the collapsed pill; Esc collapses input → capsule,
  // and collapses the card via `ask_close` (cancel + `set_chat_open`).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (cardOpen) {
          void askClose().catch(() => {});
          return;
        }
        // Esc discards the field — stop without applying the final
        // draft so the cleared text stays cleared, and discard a stop
        // already in flight: its returned draft must not land in the
        // cleared field.
        const pendingStop = dictationStopPending.current;
        if (pendingStop !== null) {
          pendingStop.applyDraft = false;
        }
        if (dictationRange.current !== null) {
          void stopDictation(false);
        }
        setText('');
        setOpen(false);
        inputRef.current?.blur();
        return;
      }
      if (
        gate !== 'main' ||
        open ||
        cardOpen ||
        e.metaKey ||
        e.ctrlKey ||
        e.altKey
      ) {
        return;
      }
      if (e.key.length === 1) {
        // Type-to-wake replaces the input — drop any tracked anchor so
        // the wake character isn't treated as an edit inside it, and
        // discard a pending stop's draft so it can't splice into the
        // replacement text either.
        const pendingStop = dictationStopPending.current;
        if (pendingStop !== null) {
          pendingStop.applyDraft = false;
        }
        if (dictationRange.current !== null) {
          void stopDictation(false);
        }
        setText(e.key);
        setOpen(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [gate, open, cardOpen]);

  const collapse = () => {
    setOpen(false);
    inputRef.current?.blur();
  };

  const grantScreen = async () => {
    if (busy) {
      return;
    }
    setBusy(true);
    try {
      await permissionsRequestScreen();
      await bootstrap();
    } catch {
      raise('Permission request failed');
    } finally {
      setBusy(false);
    }
  };

  /** Send the field's current text — read off `textRef` so the Enter
   *  queued behind a settling stop sees the applied final draft, not
   *  the render-time `text`. */
  const sendAsk = () => {
    const t = textRef.current.trim();
    if (!t) {
      return;
    }
    setText('');
    void askSend(t).catch(() => raise('Send failed'));
  };

  // Submit = ask (a follow-up while the card is open). The backend
  // expands the window itself — no local collapse needed either way.
  const submitAsk = (e: SubmitEvent<HTMLFormElement>) => {
    e.preventDefault();
    const pendingStop = dictationStopPending.current;
    if (pendingStop !== null) {
      // A stop is already settling — Enter's contract is "submit the
      // final text", so queue it behind the draft application rather
      // than re-stop (the still-`listening` state would swallow it).
      // A session anchored or stopping meanwhile vetoes the send.
      void pendingStop.promise.then(() => {
        if (
          dictationRange.current === null &&
          dictationStopPending.current === null
        ) {
          sendAsk();
        }
      });
      return;
    }
    // Enter ends dictation for review but never sends — the next Enter
    // submits the reviewed text.
    if (dictationState === 'listening' || dictationRange.current !== null) {
      void stopDictation(true);
      return;
    }
    sendAsk();
  };

  const rowCls = cn(
    'flex min-h-0 w-full flex-none items-center gap-1.5',
    cardOpen
      ? cn(
          'h-16 border-border px-2.75',
          // The divider sits between the row and the section — which
          // side depends on the grow direction (the row is bottom-
          // pinned under `flex-col-reverse` when growing up).
          growDir === 'up' ? 'border-t' : 'border-b',
        )
      : showInputRow
        ? 'px-2.75'
        : 'justify-center px-1.75',
  );

  /** The mic button's next action: stop the live mode first, dictate
   *  into the visible Ask input, or start meeting Listen from the
   *  collapsed capsule. */
  const micLabel =
    dictationState === 'listening'
      ? 'Stop dictation'
      : listenState === 'listening'
        ? 'Stop listening'
        : showInputRow
          ? 'Dictate'
          : 'Listen';

  const row = () => {
    if (bootError) {
      return (
        <div
          className={cn(rowCls, 'justify-center')}
          data-tauri-drag-region>
          {grip}
          <RetryCard onRetry={() => void bootstrap()} />
        </div>
      );
    }
    if (gate === 'needs_permission') {
      return (
        <div
          className={rowCls}
          data-tauri-drag-region>
          {grip}
          <ShieldAlertIcon
            className='size-4.5 flex-none text-muted-foreground'
            data-tauri-drag-region
          />
          <span
            className='min-w-0 flex-1 truncate text-xs text-muted-foreground'
            title='Marvis needs screen recording to see your screen'
            data-tauri-drag-region>
            Screen recording needed
          </span>
          <button
            type='button'
            className={cn(BTN_SM, BTN_PRIMARY)}
            onClick={() => void grantScreen()}
            disabled={busy}>
            Grant
          </button>
          <button
            type='button'
            className={cn(BTN_LINK_SM, BTN_LINK)}
            onClick={() => void permissionsOpenPrefs('Privacy_ScreenCapture')}>
            Open settings
          </button>
        </div>
      );
    }
    // `main` (and the null boot frame — the same capsule, inert until
    // the gate resolves).
    return (
      <form
        onSubmit={submitAsk}
        className={rowCls}
        data-tauri-drag-region>
        {grip}
        <button
          type='button'
          className={cn(
            BAR_BTN,
            'relative',
            (listenState === 'listening' || dictationState === 'listening') &&
              'listen-active',
          )}
          aria-label={
            cardOpen ? 'Close chat' : open ? 'Back to capsule' : 'Ask Marvis'
          }
          onClick={() =>
            cardOpen
              ? void askClose().catch(() => {})
              : open
                ? collapse()
                : setOpen(true)
          }
          disabled={gate !== 'main'}>
          <Iris />
          <span className='pointer-events-none absolute inset-0 grid -rotate-90 scale-[0.4] place-items-center opacity-0 transition-[rotate_var(--motion-base)_var(--ease)_55ms,scale_var(--motion-base)_var(--ease)_55ms,opacity_var(--motion-fast)_var(--ease)_55ms] group-data-expanded/bar:rotate-none group-data-expanded/bar:scale-100 group-data-expanded/bar:opacity-100 motion-reduce:transition-none'>
            <ArrowLeftIcon className='size-5.5' />
          </span>
        </button>
        <input
          ref={inputRef}
          value={text}
          onChange={(e) => {
            // A keystroke commits its own caret — don't let a queued
            // dictation-caret restore jump it to the dictated slice.
            pendingCaret.current = null;
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
                void stopDictation(false);
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
          }}
          onSelect={(e) => {
            inputSelection.current = {
              start: e.currentTarget.selectionStart ?? 0,
              end: e.currentTarget.selectionEnd ?? 0,
            };
          }}
          onFocus={() => gate === 'main' && setOpen(true)}
          placeholder='Ask Marvis…'
          aria-label='Ask Marvis'
          className={cn(
            'min-w-0 flex-1 self-stretch border-0 bg-transparent text-[13.5px] text-foreground caret-accent outline-none select-text placeholder:text-muted-foreground focus-visible:shadow-none transition-[max-width_var(--motion-base)_var(--ease),opacity_var(--motion-fast)_var(--ease),margin-inline_var(--motion-base)_var(--ease)] motion-reduce:transition-none',
            showInputRow
              ? 'max-w-80'
              : 'pointer-events-none -mx-1.5 max-w-0 opacity-0',
          )}
        />
        <button
          type='button'
          className={BAR_BTN}
          aria-label='Ask about the screen'
          title='Ask about the screen'
          disabled={gate !== 'main'}
          onClick={() =>
            void askSendScreenOnly().catch(() => raise('Send failed'))
          }>
          <CameraIcon className='size-5' />
        </button>
        <button
          type='button'
          className={BAR_BTN}
          aria-label={micLabel}
          title={micLabel}
          disabled={gate !== 'main'}
          onClick={() => {
            if (speechBusy.current) return;
            speechBusy.current = true;
            // An active session stops first — `speechBusy` stays held
            // until it settles so a follow-up press can't start the
            // other mode mid-teardown.
            if (
              dictationState === 'listening' ||
              dictationStopPending.current !== null
            ) {
              void stopDictation(true).finally(() => {
                speechBusy.current = false;
              });
              return;
            }
            if (listenState === 'listening') {
              void listenStop()
                .catch(() => raise('Stop failed'))
                .finally(() => {
                  speechBusy.current = false;
                });
              return;
            }
            if (showInputRow) {
              // The anchor is captured before the invoke so a draft
              // can never race ahead of it.
              captureDictationAnchor();
              dictationStarting.current = true;
              void dictationStart()
                .then((next) => {
                  setDictationState(next.state);
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
                  const message =
                    typeof e === 'string' ? e : 'Dictation failed';
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
                  speechBusy.current = false;
                });
              return;
            }
            setListenWanted(true);
            void windowSetChatOpen(true).catch(() => {});
            void listenStart()
              .then((next) => setListenState(next.state))
              .catch(() => raise('Listen failed'))
              .finally(() => {
                speechBusy.current = false;
              });
          }}>
          <MicIcon className='size-5' />
        </button>
        {/* Only rendered in the input row — the idle capsule has no
            room for a fourth control (tray menu + Cmd+, reach it
            anyway). */}
        {showInputRow && (
          <button
            type='button'
            className={BAR_BTN}
            aria-label='Settings'
            title='Settings (⌘,)'
            disabled={gate !== 'main'}
            onClick={() => void windowShowSettings().catch(() => {})}>
            <SettingsIcon className='size-5' />
          </button>
        )}
      </form>
    );
  };

  return (
    <div
      ref={stageRef}
      className={cn(
        'group/stage glass-stage flex h-full flex-col p-1',
        growDir === 'up' ? 'justify-end' : 'justify-start',
      )}
      data-pos={growDir === 'up' ? 'bottom' : 'top'}
      data-dir={growDir}>
      <div
        ref={cardRef}
        className={cn(
          'group/bar glass-surface relative flex w-full flex-none select-none',
          cardOpen
            ? cn(PANEL, growDir === 'up' ? 'flex-col-reverse' : 'flex-col')
            : 'h-full flex-col justify-center rounded-full bg-[color-mix(in_oklch,var(--surface)_80%,transparent)] backdrop-blur-[14px] transition-[border-color,box-shadow] duration-(--motion-base) ease-(--ease) motion-reduce:transition-none',
        )}
        style={cardOpen ? { maxHeight: CARD_MAX } : undefined}
        data-expanded={showInputRow || undefined}
        data-tauri-drag-region={cardOpen ? undefined : 'deep'}>
        {row()}
        {section === 'chat' && <ChatSection />}
        {section === 'listen' && <ListenSection />}
        {/* Capsule shimmer — accent duotone follows light/dark via the
            tokens; masked to the border ring, pointer-events-none. */}
        <ShineBorder
          shineColor={['#A07CFE', '#FE8FB5', '#FFBE7B', 'var(--accent)']}
          borderWidth={1.8}
        />
      </div>
    </div>
  );
};

export default Bar;
