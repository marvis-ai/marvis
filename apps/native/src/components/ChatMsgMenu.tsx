import { Fragment } from 'react';
import { EllipsisIcon, RotateCcwIcon } from '@marvis/ui';
import { ICON_BTN, NUM, cn } from '@/lib/classes';

/** Persisted answer metadata shown in the ⋯ menu. */
export interface ChatMsgMeta {
  provider?: string | null;
  model?: string | null;
  tokensIn?: number | null;
  tokensOut?: number | null;
}

/** The assistant row's ⋯ menu — Retry on the last reply, plus the
 *  message's persisted model/provider/token metadata. Renders null when
 *  neither applies (e.g. a pre-metadata row mid-history while busy). */
export const ChatMsgMenu = ({
  open,
  onOpenChange,
  canRetry,
  onRetry,
  meta,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  canRetry: boolean;
  onRetry: () => void;
  meta: ChatMsgMeta;
}) => {
  const rows: [string, string][] = [];
  if (meta.model != null) rows.push(['Model', meta.model]);
  if (meta.provider != null) rows.push(['Provider', meta.provider]);
  if (meta.tokensIn != null)
    rows.push(['Tokens in', meta.tokensIn.toLocaleString()]);
  if (meta.tokensOut != null)
    rows.push(['Tokens out', meta.tokensOut.toLocaleString()]);
  if (!canRetry && rows.length === 0) return null;
  return (
    <div className='relative'>
      <button
        type='button'
        aria-label='More actions'
        aria-expanded={open}
        onClick={() => onOpenChange(!open)}
        className={cn(ICON_BTN, 'size-5')}>
        <EllipsisIcon className='size-3.5' />
      </button>
      {open && (
        <>
          <button
            type='button'
            aria-hidden
            tabIndex={-1}
            onClick={() => onOpenChange(false)}
            className='fixed inset-0 z-10 cursor-default border-0 bg-transparent'
          />
          <div className='absolute bottom-full left-0 z-20 mb-1 min-w-52 rounded-xl border border-border bg-[color-mix(in_oklch,var(--surface)_88%,transparent)] py-1 text-[12px] text-foreground shadow-md backdrop-blur-lg'>
            {canRetry && (
              <button
                type='button'
                onClick={onRetry}
                className='flex w-full cursor-pointer items-center gap-1.5 border-0 bg-transparent px-2.5 py-1.5 text-left text-foreground enabled:hover:bg-fg-soft'>
                <RotateCcwIcon className='size-3.5' />
                Retry
              </button>
            )}
            {rows.length > 0 && (
              <div
                className={cn(
                  NUM,
                  'grid grid-cols-[auto_1fr] gap-x-3 px-2.5 py-1.5 text-[10.5px] leading-normal',
                  canRetry && 'mt-0.5 border-t border-border',
                )}>
                {rows.map(([label, value]) => (
                  <Fragment key={label}>
                    <span className='text-muted-foreground'>{label}</span>
                    <span className='text-[8px] text-right wrap-break-word'>
                      {value}
                    </span>
                  </Fragment>
                ))}
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
};
