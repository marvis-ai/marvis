/**
 * The bar's background-activity mirror — Listen, screen capture, and
 * the Ask tail each keep a local state synced from their `*:state`
 * events plus a mount-time status read (an emit that raced this
 * webview's listener would otherwise leave the pulse stale). Dictation
 * is separate: `useDictation` owns it.
 */
import { useEffect, useState } from 'react';
import { askRuns, captureStatus, listenStatus } from '@/lib/commands';
import {
  EV_ASK_STATE,
  EV_CAPTURE_STATE,
  EV_LISTEN_ERROR,
  EV_LISTEN_STATE,
  useTauriEvent,
  type CaptureStatePayload,
  type ListenStatePayload,
} from '@/lib/events';
import type { AskActivity } from '@/lib/bar-state';

export const useBarActivity = () => {
  const [listenWanted, setListenWanted] = useState(false);
  const [listenState, setListenState] =
    useState<ListenStatePayload['state']>('idle');
  /** Continuous screen capture — starts by default in the main gate,
   *  toggled by the collapsed MonitorDot control. */
  const [captureRunning, setCaptureRunning] = useState(false);
  /** The picker-selected capture scope (`capture:state`'s `target`) —
   *  the record button's tooltip while running; `null` on the auto
   *  primary-display path. */
  const [captureTarget, setCaptureTarget] =
    useState<CaptureStatePayload['target']>(null);
  /** Per-session mirror of `ask:state` — runs are concurrent, so a
   *  backgrounded session's stream must not mark the composer of the
   *  conversation in front of the user busy. `askRuns` is the per-
   *  session map; `askLive` is the any-run-live pulse feed. */
  const [runs, setRuns] = useState<Record<number, AskActivity>>({});

  useEffect(() => {
    void listenStatus()
      .then((next) => {
        setListenState(next.state);
        if (
          next.state === 'listening' ||
          next.state === 'paused' ||
          next.state === 'error'
        ) {
          setListenWanted(true);
        }
      })
      .catch(() => {});
    // A status read isn't user-actionable — sync the toggle silently
    // and let `capture:state` correct it if the read raced a stop.
    void captureStatus()
      .then((next) => {
        setCaptureRunning(next.running);
        setCaptureTarget(next.target);
      })
      .catch(() => {});
    // Same resync for the ask runs — an `ask:state` emit that raced
    // this webview's listener would otherwise leave the map stale.
    void askRuns()
      .then((next) => {
        setRuns(
          Object.fromEntries(
            next
              .filter((r) => r.session_id != null)
              .map((r) => [r.session_id as number, r.state]),
          ),
        );
      })
      .catch(() => {});
  }, []);

  useTauriEvent<ListenStatePayload>(EV_LISTEN_STATE, (p) => {
    setListenState(p.state);
    if (
      p.state === 'listening' ||
      p.state === 'paused' ||
      p.state === 'error'
    ) {
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
  // Capture lifecycle snapshots — emitted after every `capture_*`
  // command and every automatic gate transition, so the MonitorDot
  // toggle tracks the recorder live.
  useTauriEvent<CaptureStatePayload>(EV_CAPTURE_STATE, (p) => {
    setCaptureRunning(p.running);
    setCaptureTarget(p.target);
  });
  // A send while in listen mode reasserts chat (`loading` = a run);
  // every session's snapshot feeds both its map entry and the pulse.
  useTauriEvent<{ state: AskActivity; session_id?: number | null }>(
    EV_ASK_STATE,
    (p) => {
      if (p.session_id != null) {
        const sid = p.session_id;
        setRuns((prev) => ({ ...prev, [sid]: p.state }));
      }
      if (p.state === 'loading') {
        setListenWanted(false);
      }
    },
  );

  return {
    listenWanted,
    setListenWanted,
    listenState,
    setListenState,
    captureRunning,
    setCaptureRunning,
    captureTarget,
    setCaptureTarget,
    /** Session → live state for every run the mirror has seen. */
    askRuns: runs,
    /** Whether any session has a live run — the pulse's ask side. */
    askLive: Object.values(runs).some((s) => s !== 'idle')
      ? ('streaming' as AskActivity)
      : ('idle' as AskActivity),
  };
};
