import { useState } from 'react';
import { ChevronDownIcon, ChevronUpIcon } from '@marvis/ui';
import { CHIP, cn } from '@/lib/classes';
import type { ListenSummaryPayload } from '@/lib/events';

/** Pinned summary above the input row — the TLDR clamps to two lines;
 *  the chevron unfolds the bullets and follow-up chips in place. A
 *  follow-up chip sends itself as a question to the doc's own chat
 *  (`onFollowUp` — Bar binds it to this session's `listen_id`). */
export const SummaryStrip = ({
  summary,
  onFollowUp,
}: {
  summary: ListenSummaryPayload | null;
  onFollowUp: (question: string) => void;
}) => {
  const [open, setOpen] = useState(false);
  if (!summary) return null;
  return (
    <div className='flex-none border-t border-border px-3.5 py-2'>
      <button
        type='button'
        onClick={() => setOpen((o) => !o)}
        className='flex w-full items-center justify-between gap-2 text-left'>
        <p className='text-xs font-semibold'>
          TLDR{summary.topic ? ` · ${summary.topic}` : ''}
        </p>
        {open ? (
          <ChevronUpIcon className='size-3.5 text-muted-foreground' />
        ) : (
          <ChevronDownIcon className='size-3.5 text-muted-foreground' />
        )}
      </button>
      <p
        className={cn(
          'mt-0.5 text-[12.5px] leading-normal select-text',
          !open && 'line-clamp-2',
        )}>
        {summary.tldr}
      </p>
      {open && (
        <>
          {summary.bullets.length > 0 && (
            <ul className='mt-1 list-disc pl-4 text-[12.5px] leading-normal'>
              {summary.bullets.map((b) => (
                <li key={b}>{b}</li>
              ))}
            </ul>
          )}
          {summary.follow_ups.length > 0 && (
            <div className='mt-2 flex flex-wrap gap-1.5'>
              {summary.follow_ups.map((f) => (
                <button
                  key={f}
                  type='button'
                  onClick={() => onFollowUp(f)}
                  className={cn(
                    CHIP,
                    'cursor-pointer transition-colors hover:bg-fg-soft hover:text-foreground',
                  )}>
                  {f}
                </button>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
};
