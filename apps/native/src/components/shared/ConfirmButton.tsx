import { useRef, useState, type MouseEvent, type ReactNode } from 'react';
import { BTN_DANGER, BTN_OUTLINE, BTN_SM, cn } from '@/lib/classes';

/** Arm window for the second press. */
const CONFIRM_MS = 4000;

/** Two-step confirm — the first press swaps `children` for the `label`
 *  chip, the second within CONFIRM_MS fires `onConfirm` (the delete is
 *  never the first click). Same contract as prefs' Clear history.
 *  Clicks stop at the button: a clickable host row must not see the
 *  arm OR the confirm press. The caller owns the idle trigger's
 *  styling via `className`. */
export const ConfirmButton = ({
  onConfirm,
  label = 'Delete?',
  className,
  'aria-label': ariaLabel,
  children,
}: {
  onConfirm: () => void;
  /** Armed-chip text. */
  label?: string;
  /** Idle-trigger className. */
  className?: string;
  'aria-label'?: string;
  children: ReactNode;
}) => {
  const [armed, setArmed] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  const press = (e: MouseEvent<HTMLButtonElement>) => {
    e.stopPropagation();
    if (!armed) {
      setArmed(true);
      timer.current = window.setTimeout(() => setArmed(false), CONFIRM_MS);
      return;
    }
    window.clearTimeout(timer.current);
    setArmed(false);
    onConfirm();
  };

  return armed ? (
    <button
      type='button'
      onClick={press}
      className={cn(BTN_SM, BTN_OUTLINE, BTN_DANGER, 'h-5 px-2 text-[10px]')}>
      {label}
    </button>
  ) : (
    <button
      type='button'
      aria-label={ariaLabel}
      onClick={press}
      className={className}>
      {children}
    </button>
  );
};
