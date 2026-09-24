import type { ReactNode } from 'react';
import { ICON_BTN, cn } from '@/lib/classes';

/** Bar controls step up from the 26px overlay default (`ICON_BTN`) —
 *  the capsule is 64px tall, so buttons/icons scale ~1.3×; `fg-2` reads
 *  better than `muted` on glass. */
export const BAR_BTN = cn(ICON_BTN, 'size-8.5 text-fg-2');

/** One of the bar row's round icon buttons — shared chrome for the
 *  capture toggle, both mic controls, and settings. */
export const BarButton = ({
  label,
  title = label,
  pressed,
  disabled,
  className,
  onPress,
  children,
}: {
  label: string;
  /** Tooltip override — defaults to `label` (e.g. Settings adds ⌘,). */
  title?: string;
  pressed?: boolean;
  disabled?: boolean;
  className?: string;
  onPress: () => void;
  children: ReactNode;
}) => (
  <button
    type='button'
    className={cn(BAR_BTN, className)}
    aria-label={label}
    title={title}
    aria-pressed={pressed}
    disabled={disabled}
    onClick={onPress}>
    {children}
  </button>
);
