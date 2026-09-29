import { windowShowSettings } from '@/lib/commands';
import { BTN_OUTLINE, BTN_SM, cn } from '@/lib/classes';

/** The destructive error strip shared by the card sections — the
 *  failure's message plus an 'Open settings' shortcut when it is a
 *  setup gap (`needs_setup`). */
export const ErrorBanner = ({
  message,
  needsSetup,
}: {
  message: string;
  needsSetup: boolean;
}) => (
  <div className='flex items-center gap-2 border-b border-border bg-[color-mix(in_oklch,var(--destructive)_9%,transparent)] px-3 py-2 text-xs text-destructive'>
    <span className='min-w-0 flex-1 wrap-break-word'>{message}</span>
    {needsSetup && (
      <button
        type='button'
        className={cn(BTN_SM, BTN_OUTLINE)}
        onClick={() => void windowShowSettings().catch(() => {})}>
        Open settings
      </button>
    )}
  </div>
);
