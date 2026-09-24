/**
 * `?view=alert` — the error toast (340×100, transparent, frameless,
 * non-resizable; `windows/mod.rs` centers it under the bar).
 *
 * Errors used to render inside the bar pill, but the bar window is a
 * fixed 600×64 frame: an extra row squeezed the pill and broke its
 * icon-row⇄input-row content swap. This window carries them instead,
 * so no error can change the bar's layout.
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
import { ICON_BTN, cn } from '../lib/classes';

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
    <div className='glass-stage h-full p-1'>
      <div className='glass-surface flex min-h-full flex-col justify-center gap-1.5 rounded-[14px] border border-[color-mix(in_oklch,var(--destructive)_28%,var(--border))] bg-[color-mix(in_oklch,var(--surface)_92%,transparent)] px-2.75 pt-2.25 pb-2.5 shadow-[0_18px_40px_-16px_color-mix(in_oklch,var(--fg)_34%,transparent)] backdrop-blur-lg'>
        <header className='flex items-center gap-1.75'>
          <ShieldAlertIcon className='size-3.75 flex-none text-destructive' />
          <p className='min-w-0 flex-1 text-[12.5px] font-semibold'>
            Marvis ran into a problem
          </p>
          <button
            type='button'
            className={cn(ICON_BTN, 'shrink-0')}
            title='Dismiss'
            aria-label='Dismiss'
            onClick={dismiss}>
            <XIcon className='size-4' />
          </button>
        </header>
        <p
          className='line-clamp-2 text-xs leading-[1.45] wrap-break-word text-muted-foreground select-text'
          title={alert.message}>
          {alert.message}
        </p>
      </div>
    </div>
  );
};

export default AlertToast;
