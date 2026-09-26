import {
  HistoryIcon,
  MessageSquareTextIcon,
  MicAudioLinesIcon,
} from '@marvis/ui';
import { CHIP, cn } from '@/lib/classes';

/** The open card's section switcher — the card stacks
 *  `flex-col-reverse`, so this LAST DOM child renders on top. The
 *  Listen tab pings while a session is live so a background capture
 *  stays visible from chat/history. */
export const CardTabs = ({
  section,
  listenLive,
  onPick,
}: {
  section: 'chat' | 'listen' | 'history';
  listenLive: boolean;
  onPick: (s: 'chat' | 'listen' | 'history') => void;
}) => {
  const tabs = [
    { id: 'chat' as const, label: 'Chat', icon: MessageSquareTextIcon },
    { id: 'listen' as const, label: 'Listen', icon: MicAudioLinesIcon },
    { id: 'history' as const, label: 'History', icon: HistoryIcon },
  ];
  return (
    <div className='flex flex-none items-center gap-1 border-b border-border px-3 py-1.75'>
      {tabs.map(({ id, label, icon: Icon }) => (
        <button
          key={id}
          type='button'
          onClick={() => onPick(id)}
          className={cn(
            CHIP,
            'cursor-pointer gap-1 text-[10px] transition-colors',
            section === id
              ? 'border-foreground/60 bg-fg-soft text-foreground'
              : 'hover:text-foreground',
          )}>
          <Icon className='size-3' />
          {label}
          {id === 'listen' && listenLive && (
            <i
              aria-hidden
              className='size-1.25 animate-capture-ping rounded-full bg-accent'
            />
          )}
        </button>
      ))}
    </div>
  );
};
