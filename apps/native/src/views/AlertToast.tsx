/**
 * `?view=alert` — the error toast (340×104, transparent, frameless,
 * non-resizable; `windows/mod.rs` centers it under the bar).
 *
 * Errors used to render inside the bar pill, but the bar window is a
 * fixed 353×47 frame: an extra row squeezed the pill's content and broke
 * the capsule⇄input morph. This window carries them instead, so no error
 * can change the bar's layout.
 *
 * The payload is read twice on purpose: `alert:show` is the live path,
 * and `alert_current` covers a show that raced this webview's listener
 * (the window is built hidden at startup, but the first emit can still
 * land mid-load). `alert_dismiss` clears the backend's copy so a later
 * mount doesn't resurrect a dead alert.
 *
 * `action: 'reset'` is the recoverable-keystore affordance (obsolete or
 * corrupt `keys.enc`, lost DEK): it deletes `keys.enc` + the Keychain
 * item(s), and `keystore:changed`/`app:state` put the bar back on its
 * first-run unlock card. Informational alerts self-dismiss; actionable
 * ones wait for the user.
 */
import { useCallback, useEffect, useState } from 'react';
import { ShieldAlert, X } from '@marvis/ui';
import {
  alertCurrent,
  alertDismiss,
  keystoreReset,
  type AlertPayload,
} from '../lib/commands';
import { EV_ALERT_SHOW, useTauriEvent } from '../lib/events';

/** How long an alert with no action stays up. */
const AUTO_DISMISS_MS = 6000;

const AlertToast = () => {
  const [alert, setAlert] = useState<AlertPayload | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void alertCurrent()
      .then(setAlert)
      .catch(() => {});
  }, []);

  useTauriEvent<AlertPayload | null>(EV_ALERT_SHOW, (p) => {
    setBusy(false);
    setAlert(p);
  });

  const dismiss = useCallback(() => {
    setAlert(null);
    setBusy(false);
    void alertDismiss().catch(() => {});
  }, []);

  useEffect(() => {
    if (!alert || alert.action) {
      return;
    }
    const timer = window.setTimeout(dismiss, AUTO_DISMISS_MS);
    return () => window.clearTimeout(timer);
  }, [alert, dismiss]);

  const reset = () => {
    if (busy) {
      return;
    }
    setBusy(true);
    void keystoreReset()
      .then(dismiss)
      .catch(() =>
        setAlert({ message: 'Reset failed — quit and relaunch Marvis.' }),
      );
  };

  if (!alert) {
    return null;
  }

  return (
    <div className='p-1'>
      <div className='mv-alert'>
        <header className='mv-alert-head'>
          <ShieldAlert className='mv-alert-ico' />
          <p className='mv-alert-title'>Marvis ran into a problem</p>
          <button
            type='button'
            className='mv-icon-btn shrink-0'
            title='Dismiss'
            aria-label='Dismiss'
            onClick={dismiss}>
            <X />
          </button>
        </header>
        <p
          className='mv-alert-msg'
          title={alert.message}>
          {alert.message}
        </p>
        {alert.action === 'reset' && (
          <div className='mv-alert-foot'>
            <span className='mv-alert-hint'>
              Stored API keys are deleted — you'll re-enter them.
            </span>
            <button
              type='button'
              className='mv-btn mv-btn-primary'
              onClick={reset}
              disabled={busy}>
              Reset keystore
            </button>
          </div>
        )}
      </div>
    </div>
  );
};

export default AlertToast;
