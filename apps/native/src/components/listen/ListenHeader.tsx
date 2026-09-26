import {
  CheckIcon,
  CopyIcon,
  PauseIcon,
  PlayIcon,
  SquareIcon,
} from '@marvis/ui';
import { CHIP, ICON_BTN, NUM, cn } from '@/lib/classes';
import { CardHeader } from '../bar/CardHeader';
import { elapsedLabel } from './model';

/** Document header: title/subtitle on the left; state pill, elapsed
 *  recording time, and pause/resume/stop/copy controls on the right.
 *  `live` stays in the contract (idle-live shows no badge, viewed docs
 *  show STOPPED) even though the control booleans carry the render. */
export const ListenHeader = ({
  title,
  subtitle,
  badge,
  elapsedSecs,
  listening,
  paused,
  copiedAll,
  onPause,
  onResume,
  onStop,
  onCopyAll,
}: {
  title: string;
  subtitle: string;
  badge: 'LISTENING' | 'PAUSED' | 'STOPPED' | null;
  elapsedSecs: number;
  live: boolean;
  listening: boolean;
  paused: boolean;
  copiedAll: boolean;
  onPause: () => void;
  onResume: () => void;
  onStop: () => void;
  onCopyAll: () => void;
}) => (
  <CardHeader
    title={title}
    subtitle={subtitle}>
    {badge && (
      <span
        className={cn(
          CHIP,
          badge === 'LISTENING' && 'border-accent/40 text-accent',
        )}>
        {badge === 'LISTENING' && (
          <i
            aria-hidden
            className='size-1.25 animate-capture-ping rounded-full bg-accent'
          />
        )}
        {badge}
      </span>
    )}
    {badge && (
      <span className={cn(NUM, 'text-xs text-foreground')}>
        {elapsedLabel(elapsedSecs)}
      </span>
    )}
    {listening && (
      <button
        type='button'
        className={ICON_BTN}
        title='Pause'
        aria-label='Pause recording'
        onClick={onPause}>
        <PauseIcon className='size-3.5' />
      </button>
    )}
    {paused && (
      <button
        type='button'
        className={cn(ICON_BTN, 'text-accent')}
        title='Resume'
        aria-label='Resume recording'
        onClick={onResume}>
        <PlayIcon className='size-3.5' />
      </button>
    )}
    {(listening || paused) && (
      <button
        type='button'
        className={ICON_BTN}
        title='Stop'
        aria-label='Stop recording'
        onClick={onStop}>
        <SquareIcon className='size-3.5' />
      </button>
    )}
    <button
      type='button'
      className={ICON_BTN}
      title='Copy transcript'
      aria-label='Copy transcript'
      onClick={onCopyAll}>
      {copiedAll ? (
        <CheckIcon className='size-3.5 text-accent' />
      ) : (
        <CopyIcon className='size-3.5' />
      )}
    </button>
  </CardHeader>
);
