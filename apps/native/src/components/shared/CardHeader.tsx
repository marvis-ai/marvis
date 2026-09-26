import type { ReactNode } from 'react';
import { ArrowLeftIcon } from '@marvis/ui';
import { ICON_BTN, META, PANEL_HEAD, cn } from '@/lib/classes';

/** Shared card chrome: `PANEL_HEAD` shell with an optional back
 *  button (left), a title/subtitle block, and a right-side action
 *  slot (`children` — each section supplies its own controls).
 *  `data-tauri-drag-region='deep'` makes the whole header the card's
 *  drag surface — clickable children (buttons) still exclude
 *  themselves per Tauri's drag-region rules. */
export const CardHeader = ({
  title,
  subtitle,
  onBack,
  children,
}: {
  title: string;
  subtitle?: string;
  onBack?: () => void;
  children?: ReactNode;
}) => (
  <header
    className={PANEL_HEAD}
    data-tauri-drag-region='deep'>
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
    <div className='min-w-0 flex-1 select-none cursor-grab'>
      <p className='truncate text-xs leading-normal font-[550] select-none'>
        {title}
      </p>
      {subtitle && (
        <p className={cn(META, 'truncate select-none text-[10.5px]')}>
          {subtitle}
        </p>
      )}
    </div>
    {children}
  </header>
);
