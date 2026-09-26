import type { ReactNode } from 'react';
import { ArrowLeftIcon } from '@marvis/ui';
import { ICON_BTN, META, PANEL_HEAD, cn } from '@/lib/classes';

/** Shared card chrome: `PANEL_HEAD` shell with an optional back
 *  button (left), a title/subtitle block, and a right-side action
 *  slot (`children` — each section supplies its own controls).
 *  `draggable` makes the header the card's drag region for surfaces
 *  that render no bottom input row (e.g. standalone history). */
export const CardHeader = ({
  title,
  subtitle,
  onBack,
  draggable,
  children,
}: {
  title: string;
  subtitle?: string;
  onBack?: () => void;
  draggable?: boolean;
  children?: ReactNode;
}) => (
  <header className={PANEL_HEAD} data-tauri-drag-region={draggable || undefined}>
    {onBack && (
      <button
        type='button'
        className={cn(ICON_BTN, '-mt-0.5 shrink-0')}
        title='Back'
        aria-label='Back'
        onClick={onBack}>
        <ArrowLeftIcon className='size-4' />
      </button>
    )}
    <div className='min-w-0 flex-1'>
      <p className='truncate text-xs leading-normal font-[550] select-text'>
        {title}
      </p>
      {subtitle && (
        <p className={cn(META, 'truncate text-[10.5px]')}>{subtitle}</p>
      )}
    </div>
    {children}
  </header>
);
