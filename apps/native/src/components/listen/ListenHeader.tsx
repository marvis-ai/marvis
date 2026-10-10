import type { ReactNode } from 'react';
import {
  PauseIcon,
  PlayIcon,
  RotateCcwIcon,
  RotateCwIcon,
  SquareIcon,
} from '@marvis/ui';
import { CHIP, ICON_BTN, cn } from '@/lib/classes';
import { CardHeader } from '@/components/shared/CardHeader';

/** Document header: title/subtitle on the left; the live state pill
 *  (LISTENING/PAUSED) and pause/resume/stop controls on the right —
 *  live-only, while viewed docs carry ended-session audio controls.
 *  Elapsed time and copy moved down to the SpeakerFilter row, shared
 *  by live and viewed docs alike. `player` is the seek-bar slot — it
 *  overlays the header's bottom border line. */
export const ListenHeader = ({
  title,
  subtitle,
  listening,
  paused,
  hasSession,
  audioReady,
  audioPlaying,
  onAudioSkip,
  onAudioToggle,
  onPause,
  onResume,
  onStop,
  onBack,
  player,
}: {
  title: string;
  subtitle: string;
  listening: boolean;
  paused: boolean;
  /** A session is actually open (live-only) — gates the LISTENING
   *  badge, which is derived here so it can't disagree with the
   *  controls. */
  hasSession: boolean;
  /** Ended-session audio controls — absent for live docs. */
  audioReady: boolean;
  audioPlaying: boolean;
  onAudioSkip: (seconds: number) => void;
  onAudioToggle: () => void;
  onPause: () => void;
  onResume: () => void;
  onStop: () => void;
  onBack: () => void;
  /** Ended-session seek bar — rendered under the header, its 1px track
   *  centered on the header's bottom border. */
  player?: ReactNode;
}) => {
  const badge = paused
    ? ('PAUSED' as const)
    : hasSession && listening
      ? ('LISTENING' as const)
      : null;
  return (
    <div className='relative'>
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
        {audioReady && (
          <div className={cn(CHIP, 'gap-0.5 px-0.5')}>
            <button
              type='button'
              className={ICON_BTN}
              title='Back 15 seconds'
              aria-label='Back 15 seconds'
              onClick={() => onAudioSkip(-15)}>
              <RotateCcwIcon className='size-3.25' />
            </button>
            <button
              type='button'
              className={cn(ICON_BTN, 'text-foreground')}
              title={audioPlaying ? 'Pause audio' : 'Play audio'}
              aria-label={audioPlaying ? 'Pause audio' : 'Play audio'}
              onClick={onAudioToggle}>
              {audioPlaying ? (
                <PauseIcon className='size-3.5' />
              ) : (
                <PlayIcon className='size-3.5' />
              )}
            </button>
            <button
              type='button'
              className={ICON_BTN}
              title='Forward 15 seconds'
              aria-label='Forward 15 seconds'
              onClick={() => onAudioSkip(15)}>
              <RotateCwIcon className='size-3.25' />
            </button>
          </div>
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
      </CardHeader>
      {player}
    </div>
  );
};
