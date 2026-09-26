import { CHIP, NUM, cn } from '@/lib/classes';

/** Speaker filter row — `all` + one chip per distinct block identity,
 *  `{n} rows` pinned right. Display-only; the parent resets it on
 *  session/viewing change. */
export const SpeakerFilter = ({
  speakers,
  active,
  count,
  onPick,
}: {
  speakers: { key: string; name: string; color: string }[];
  active: string | null;
  count: number;
  onPick: (key: string | null) => void;
}) => (
  <div className='flex items-center gap-1.5 border-b border-border px-3 py-1.75'>
    {[{ key: null as string | null, name: 'all', color: '' }, ...speakers].map(
      (s) => (
        <button
          key={s.key ?? 'all'}
          type='button'
          onClick={() => onPick(s.key)}
          className={cn(
            CHIP,
            'cursor-pointer lowercase transition-colors',
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
      ),
    )}
    <span
      className={cn(NUM, 'ml-auto text-[10px] text-muted-foreground')}>
      {count} rows
    </span>
  </div>
);
