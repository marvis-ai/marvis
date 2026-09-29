import { PauseIcon, PlayIcon, SquareIcon } from '@marvis/ui';
import { BTN_OUTLINE, BTN_SM, CHIP, ICON_BTN, cn } from '@/lib/classes';
import { CardHeader } from '../shared/CardHeader';

/** Document header: title/subtitle on the left; the live state pill
 *  (LISTENING/PAUSED) and pause/resume/stop controls on the right —
 *  live-only, so a viewed doc instead carries a 'Start new' action.
 *  Elapsed time and copy moved down to the SpeakerFilter row, shared
 *  by live and viewed docs alike. */
export const ListenHeader = ({
  title,
  subtitle,
  badge,
  listening,
  paused,
  onPause,
  onResume,
  onStop,
  onStartNew,
  onBack,
}: {
  title: string;
  subtitle: string;
  badge: 'LISTENING' | 'PAUSED' | null;
  listening: boolean;
  paused: boolean;
  onPause: () => void;
  onResume: () => void;
  onStop: () => void;
  /** Present only on a viewed doc — mints a fresh session. */
  onStartNew?: () => void;
  onBack: () => void;
}) => (
  <CardHeader
    onBack={onBack}
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
    {onStartNew && (
      <button
        type='button'
        className={cn(BTN_SM, BTN_OUTLINE)}
        onClick={onStartNew}>
        Start new
      </button>
    )}
  </CardHeader>
);
