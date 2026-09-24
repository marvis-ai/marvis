/**
 * The bar's background-activity mirror — Listen, screen capture, and
 * the Ask tail each keep a local state synced from their `*:state`
 * events plus a mount-time status read (an emit that raced this
 * webview's listener would otherwise leave the pulse stale). Dictation
 * is separate: `useDictation` owns it.
 */
import { useEffect, useState } from 'react';
import { askCurrent, captureStatus, listenStatus } from '@/lib/commands';
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
  /** Local mirror of `ask:state` so loading/streaming count as active
   *  work for the bar pulse. */
  const [askState, setAskState] = useState<AskActivity>('idle');

  useEffect(() => {
    void listenStatus()
      .then((next) => {
        setListenState(next.state);
        if (next.state === 'listening' || next.state === 'error') {
          setListenWanted(true);
        }
      })
      .catch(() => {});
    // A status read isn't user-actionable — sync the toggle silently
    // and let `capture:state` correct it if the read raced a stop.
    void captureStatus()
      .then((next) => setCaptureRunning(next.running))
      .catch(() => {});
    // Same resync for the ask tail — an `ask:state` emit that raced
    // this webview's listener would otherwise leave the pulse stale.
    void askCurrent()
      .then((next) => setAskState(next.state))
      .catch(() => {});
  }, []);

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
  // Capture lifecycle snapshots — emitted after every `capture_*`
  // command and every automatic gate transition, so the MonitorDot
  // toggle tracks the recorder live.
  useTauriEvent<CaptureStatePayload>(EV_CAPTURE_STATE, (p) => {
    setCaptureRunning(p.running);
  });
  // A send while in listen mode reasserts chat (`loading` = a run);
  // every snapshot feeds the active-work pulse.
  useTauriEvent<{ state: AskActivity }>(EV_ASK_STATE, (p) => {
    setAskState(p.state);
    if (p.state === 'loading') {
      setListenWanted(false);
    }
  });

  return {
    listenWanted,
    setListenWanted,
    listenState,
    setListenState,
    captureRunning,
    setCaptureRunning,
    askState,
  };
};
