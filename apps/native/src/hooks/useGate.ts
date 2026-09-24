/**
 * The permission gate + boot path for the bar. `gate` mirrors `app_gate`
 * in lib.rs: `needs_permission` swaps the row for the grant card, `main`
 * is the capsule. `bootError` covers a rejected mount-time status read
 * (a transparent frameless window must never render blank). `busy`
 * serializes the grant click — and the capture toggle shares the flag.
 */
import { useCallback, useEffect, useState } from 'react';
import {
  permissionsRequestScreen,
  permissionsStatus,
  raise,
  windowSetChatOpen,
  type AppStatePayload,
  type Gate,
} from '@/lib/commands';
import {
  EV_APP_STATE,
  EV_CAPTURE_PERMISSION_NEEDED,
  useTauriEvent,
} from '@/lib/events';

/** Mirrors `app_gate` in lib.rs so the first render doesn't wait on `app:state`. */
const gateFor = (screen: boolean): Gate =>
  screen ? 'main' : 'needs_permission';

export const useGate = () => {
  const [gate, setGate] = useState<Gate | null>(null);
  const [bootError, setBootError] = useState(false);
  const [busy, setBusy] = useState(false);

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
  }, [bootstrap]);

  useTauriEvent<AppStatePayload>(EV_APP_STATE, (p) => setGate(p.gate));
  // Mid-session screen-permission revocation (ask.rs detects it when a
  // stale frame would have shipped): collapse the card — NOT `askClose`,
  // which would cancel the text-only fallback — and show the
  // permission card.
  useTauriEvent<{ permission: string }>(EV_CAPTURE_PERMISSION_NEEDED, () => {
    void windowSetChatOpen(false).catch(() => {});
    setGate('needs_permission');
  });

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

  return { gate, bootError, busy, setBusy, bootstrap, grantScreen };
};
