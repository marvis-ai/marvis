import { CheckIcon, CopyIcon, TimerIcon, CaptionsIcon } from '@marvis/ui';
import { CHIP, ICON_BTN, NUM, cn } from '@/lib/classes';

/** Speaker filter row — `all` + one chip per distinct block identity,
 *  with the document meta pinned right (`{n} lines`, the elapsed
 *  recording time, and transcript copy — moved off the header so live
 *  and viewed docs share them). Many speakers scroll the chip strip
 *  horizontally (scrollbar hidden) while the meta stays shrink-
 *  wrapped on the right. Display-only; the parent resets the filter
 *  on session/viewing change. */
export const SpeakerFilter = ({
  speakers,
  active,
  count,
  elapsed,
  copied,
  onPick,
  onCopy,
}: {
  speakers: { key: string; name: string; color: string }[];
  active: string | null;
  count: number;
  /** Formatted `m:ss` duration — null while no session is on screen. */
  elapsed: string | null;
  copied: boolean;
  onPick: (key: string | null) => void;
  onCopy: () => void;
}) => (
  <div className='flex items-center gap-1.5 border-b border-border px-3 py-1.75'>
    <div className='flex min-w-0 flex-1 items-center gap-1.5 overflow-x-auto scrollbar-none [&::-webkit-scrollbar]:hidden'>
      {[
        { key: null as string | null, name: 'all', color: '' },
        ...speakers,
      ].map((s) => (
        <button
          key={s.key ?? 'all'}
          type='button'
          onClick={() => onPick(s.key)}
          className={cn(
            CHIP,
            'shrink-0 cursor-pointer lowercase transition-colors',
            active === s.key
              ? 'border-foreground/60 bg-fg-soft text-foreground'
              : 'hover:text-foreground',
          )}>
          {s.key !== null && (
            <i
              aria-hidden
              className={cn('size-1.5 rounded-full bg-current', s.color)}
            />
          )}
          {s.name}
        </button>
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
  </div>
);
