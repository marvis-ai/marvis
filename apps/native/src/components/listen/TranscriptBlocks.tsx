import { useState } from 'react';
import { CheckIcon, CopyIcon } from '@marvis/ui';
import { ICON_BTN, NUM, cn } from '@/lib/classes';
import {
  blockCopyText,
  elapsedLabel,
  timeLabel,
  type TurnBlock,
} from './model';

const ACTIVE_BLOCK_CLASSES: Record<string, string> = {
  'text-accent': 'bg-accent/10 ring-1 ring-accent/25',
  'text-fg-2': 'bg-fg-2/10 ring-1 ring-fg-2/25',
  'text-speaker-1': 'bg-speaker-1/10 ring-1 ring-speaker-1/25',
  'text-speaker-2': 'bg-speaker-2/10 ring-1 ring-speaker-2/25',
  'text-speaker-3': 'bg-speaker-3/10 ring-1 ring-speaker-3/25',
  'text-speaker-4': 'bg-speaker-4/10 ring-1 ring-speaker-4/25',
};

/** The block list: `m:ss` · NAME header per block (relative to the
 *  session start, wall-clock fallback), paragraph of finals + dimmed
 *  interim caret, and a hover copy icon per block. */
export const TranscriptBlocks = ({
  blocks,
  startedAt,
  activeBlock = null,
  onSeekBlock,
}: {
  blocks: TurnBlock[];
  startedAt: number | null;
  activeBlock?: string | null;
  onSeekBlock?: (block: TurnBlock) => void;
}) => {
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const copyBlock = (b: TurnBlock) => {
    // Same-speaker blocks share `b.key` — key the flash by the block's
    // React key (`key-ts`) so only the copied block shows the check.
    const key = `${b.key}-${b.ts}`;
    void navigator.clipboard
      .writeText(blockCopyText(b))
      .then(() => {
        setCopiedKey(key);
        window.setTimeout(
          () => setCopiedKey((k) => (k === key ? null : k)),
          1500,
        );
      })
      .catch(() => {});
  };
  return (
    <>
      {blocks.map((block) => {
        const blockId = `${block.key}-${block.ts}`;
        const stamp =
          startedAt != null
            ? elapsedLabel(block.ts - startedAt)
            : timeLabel(block.ts);
        const timestamp = (
          <span
            className={cn(
              NUM,
              'w-8 flex-none text-[10px] text-muted-foreground',
            )}>
            {stamp}
          </span>
        );
        return (
          <div
            key={blockId}
            className={cn(
              'group/row relative mb-3.5 rounded-lg',
              activeBlock === blockId && ACTIVE_BLOCK_CLASSES[block.color],
            )}>
            <div className={cn('flex items-center gap-1.5', block.color)}>
              {onSeekBlock ? (
                <button
                  type='button'
                  onClick={() => onSeekBlock(block)}
                  aria-label={`Seek to ${stamp}`}
                  className='flex w-8 flex-none cursor-pointer border-0 bg-transparent p-0 text-left focus-visible:outline-2 focus-visible:outline-accent'>
                  {timestamp}
                </button>
              ) : (
                timestamp
              )}
              <span
                aria-hidden
                className='size-1.75 flex-none rounded-full bg-current'
              />
              <span className='text-[12px] font-[650] tracking-[-0.005em]'>
                {block.name}
              </span>
              <button
                type='button'
                onClick={() => copyBlock(block)}
                aria-label={`Copy ${block.name}'s turn`}
                className={cn(
                  ICON_BTN,
                  'ml-auto size-5 opacity-0 transition-opacity group-hover/row:opacity-100 focus-visible:opacity-100',
                )}>
                {copiedKey === blockId ? (
                  <CheckIcon className='size-3 text-accent' />
                ) : (
                  <CopyIcon className='size-3' />
                )}
              </button>
            </div>
            <p className='mt-1 wrap-break-word whitespace-pre-wrap select-text pl-8'>
              {block.finals.map((turn) => turn.text).join(' ')}
              {block.interim && (
                <span className='text-muted-foreground'>
                  {block.finals.length > 0 ? ' ' : ''}
                  {block.interim.text}
                  <span
                    aria-hidden
                    className='animate-caret ml-0.5 inline-block h-[0.95em] w-[1.5px] translate-y-[0.15em] bg-current'
                  />
                </span>
              )}
            </p>
          </div>
        );
      })}
    </>
  );
};
