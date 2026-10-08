import { useState } from 'react';
import {
  CheckIcon,
  CopyIcon,
  TimerIcon,
  CaptionsIcon,
  ShareIcon,
} from '@marvis/ui';
import { CHIP, ICON_BTN, NUM, cn } from '@/lib/classes';
import { SpeakerNameEditor } from './SpeakerNameEditor';

/** Speaker filter row — `all` + one chip per distinct block identity,
 *  with the document meta pinned right (`{n} lines`, the elapsed
 *  recording time, and transcript copy + export — moved off the header
 *  so live and viewed docs share them). Many speakers scroll the chip
 *  strip horizontally (scrollbar hidden) while the meta stays shrink-
 *  wrapped on the right. Display-only; the parent resets the filter
 *  on session/viewing change. */
export const SpeakerFilter = ({
  speakers,
  active,
  count,
  elapsed,
  copied,
  exported,
  onPick,
  onRename,
  onCopy,
  onCopyMarkdown,
  onSaveMarkdown,
  onSaveAudio,
  canSaveAudio,
}: {
  speakers: {
    key: string;
    name: string;
    color: string;
    canRename: boolean;
  }[];
  active: string | null;
  count: number;
  /** Formatted `m:ss` duration — null while no session is on screen. */
  elapsed: string | null;
  copied: boolean;
  exported: boolean;
  onPick: (key: string | null) => void;
  /** Session-local rename — `''` restores the default label. */
  onRename: (key: string, label: string) => void;
  onCopy: () => void;
  onCopyMarkdown: () => void;
  onSaveMarkdown: () => void;
  onSaveAudio: () => void;
  canSaveAudio: boolean;
}) => {
  const [exportOpen, setExportOpen] = useState(false);
  return (
    <div className='flex items-center gap-1.5 border-b border-border px-3 py-1.75'>
      <div className='flex min-w-0 flex-1 items-center gap-1.5 overflow-x-auto scrollbar-none [&::-webkit-scrollbar]:hidden'>
        {/* `all` stays a plain button; speaker chips split into two
            controls — the colored dot filters (`onPick`), the name edits
            in place (`onRename`). */}
        <button
          type='button'
          onClick={() => onPick(null)}
          className={cn(
            CHIP,
            'shrink-0 cursor-pointer lowercase transition-colors',
            active === null
              ? 'border-foreground/60 bg-fg-soft text-foreground'
              : 'hover:text-foreground',
          )}>
          all
        </button>
        {speakers.map((s) => (
          <span
            key={s.key}
            className={cn(
              CHIP,
              'shrink-0 lowercase transition-colors',
              active === s.key
                ? 'border-foreground/60 bg-fg-soft text-foreground'
                : 'hover:text-foreground',
            )}>
            <button
              type='button'
              onClick={() => onPick(s.key)}
              aria-label={`Filter by ${s.name}`}
              title={`Filter by ${s.name}`}
              className={cn(
                '-my-0.5 -ml-0.5 grid cursor-pointer place-items-center',
                'rounded-full border-0 bg-transparent p-1 text-inherit',
                'focus-visible:outline-2 focus-visible:outline-accent',
              )}>
              <i
                aria-hidden
                className={cn('size-1.5 rounded-full bg-current', s.color)}
              />
            </button>
            <SpeakerNameEditor
              label={s.name}
              editable={s.canRename}
              onCommit={(label) => onRename(s.key, label)}
            />
          </span>
        ))}
      </div>
      <span
        className={cn(
          NUM,
          'inline-flex flex-none items-center gap-1 text-[10px] text-muted-foreground',
        )}>
        <CaptionsIcon
          aria-hidden
          className='size-3 text-muted-foreground'
        />
        {count} lines
      </span>
      {elapsed !== null && (
        <span
          className={cn(
            NUM,
            'inline-flex flex-none items-center gap-1 text-[10px] text-muted-foreground',
          )}>
          <TimerIcon
            aria-hidden
            className='size-3 text-muted-foreground'
          />
          {elapsed}
        </span>
      )}
      <button
        type='button'
        className={cn(ICON_BTN, 'size-5 flex-none')}
        title='Copy transcript'
        aria-label='Copy transcript'
        onClick={onCopy}>
        {copied ? (
          <CheckIcon className='size-3 text-accent' />
        ) : (
          <CopyIcon className='size-3' />
        )}
      </button>
      <div className='relative flex-none'>
        <button
          type='button'
          className={cn(ICON_BTN, 'size-5')}
          title='Export transcript'
          aria-label='Export transcript'
          aria-expanded={exportOpen}
          onClick={() => setExportOpen((o) => !o)}>
          {exported ? (
            <CheckIcon className='size-3 text-accent' />
          ) : (
            <ShareIcon className='size-3' />
          )}
        </button>
        {exportOpen && (
          <>
            <button
              type='button'
              aria-hidden
              tabIndex={-1}
              onClick={() => setExportOpen(false)}
              className={cn(
                'fixed inset-0 z-10 cursor-default',
                'border-0 bg-transparent',
              )}
            />
            <div
              className={cn(
                'absolute right-0 top-full z-20 mt-1 min-w-36 rounded-xl',
                'border border-border',
                'bg-[color-mix(in_oklch,var(--surface)_88%,transparent)]',
                'py-1 text-[12px] text-foreground shadow-md backdrop-blur-lg',
              )}>
              <button
                type='button'
                className={cn(
                  'flex w-full cursor-pointer items-center border-0',
                  'bg-transparent px-2.5 py-1.5 text-left text-foreground',
                  'enabled:hover:bg-fg-soft',
                )}
                onClick={() => {
                  setExportOpen(false);
                  onCopyMarkdown();
                }}>
                Copy markdown
              </button>
              <button
                type='button'
                className={cn(
                  'flex w-full cursor-pointer items-center border-0',
                  'bg-transparent px-2.5 py-1.5 text-left text-foreground',
                  'enabled:hover:bg-fg-soft',
                )}
                onClick={() => {
                  setExportOpen(false);
                  onSaveMarkdown();
                }}>
                Save .md…
              </button>
              {canSaveAudio && (
                <button
                  type='button'
                  className={cn(
                    'flex w-full cursor-pointer items-center border-0',
                    'bg-transparent px-2.5 py-1.5 text-left text-foreground',
                    'enabled:hover:bg-fg-soft disabled:opacity-40',
                  )}
                  onClick={() => {
                    setExportOpen(false);
                    onSaveAudio();
                  }}>
                  Save audio…
                </button>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
};
