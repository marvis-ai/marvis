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
 * mount doesn't resurrect a dead alert. Alerts are informational only
 * and self-dismiss after `AUTO_DISMISS_MS`.
 */
import { useCallback, useEffect, useState } from 'react';
import { ShieldAlertIcon, XIcon } from '@marvis/ui';
import { alertCurrent, alertDismiss, type AlertPayload } from '../lib/commands';
import { EV_ALERT_SHOW, useTauriEvent } from '../lib/events';

/** How long an alert stays up. */
const AUTO_DISMISS_MS = 6000;

const AlertToast = () => {
  const [alert, setAlert] = useState<AlertPayload | null>(null);

  useEffect(() => {
    void alertCurrent()
      .then(setAlert)
      .catch(() => {});
  }, []);

  useTauriEvent<AlertPayload | null>(EV_ALERT_SHOW, setAlert);

  const dismiss = useCallback(() => {
    setAlert(null);
    void alertDismiss().catch(() => {});
  }, []);

  useEffect(() => {
    if (!alert) {
      return;
    }
    const timer = window.setTimeout(dismiss, AUTO_DISMISS_MS);
    return () => window.clearTimeout(timer);
  }, [alert, dismiss]);

  if (!alert) {
    return null;
  }

  return (
    <div className='p-1'>
      <div className='mv-alert'>
        <header className='mv-alert-head'>
          <ShieldAlertIcon className='mv-alert-ico' />
          <p className='mv-alert-title'>Marvis ran into a problem</p>
          <button
            type='button'
            className='mv-icon-btn shrink-0'
            title='Dismiss'
            aria-label='Dismiss'
            onClick={dismiss}>
            <XIcon />
          </button>
        </header>
        <p
          className='mv-alert-msg'
          title={alert.message}>
          {alert.message}
        </p>
      </div>
    </div>
  );
};

export default AlertToast;
