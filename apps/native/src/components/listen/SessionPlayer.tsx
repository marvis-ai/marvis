import type { CSSProperties } from 'react';
import { elapsedLabel } from './model';
import type { SessionPlayerHandle } from '@/hooks/useSessionPlayer';

/** Ended-session audio view — the hidden `<audio>` element plus the
 *  1px seek line that rides `ListenHeader`'s bottom border. All the
 *  element plumbing and playback state come from `useSessionPlayer`;
 *  this component is markup only. */
export const SessionPlayer = ({ player }: { player: SessionPlayerHandle }) => {
  if (!player.visible) return null;
  return (
    <>
      <audio
        preload='metadata'
        className='hidden'
        aria-hidden
        {...player.audio}
      />
      {player.duration > 0 && (
        <input
          type='range'
          aria-label='Playback position'
          aria-valuetext={`${elapsedLabel(player.position)} of ${elapsedLabel(player.duration)}`}
          min={0}
          max={player.duration}
          step={0.1}
          value={Math.min(player.position, player.duration)}
          style={
            {
              '--seek-fill': `${(Math.min(player.position, player.duration) / player.duration) * 100}%`,
            } as CSSProperties
          }
          className='absolute inset-x-0 bottom-[-4.5px] m-0 h-2.5 cursor-pointer appearance-none [-webkit-appearance:none] bg-transparent outline-none [&::-webkit-slider-runnable-track]:h-px [&::-webkit-slider-runnable-track]:bg-[linear-gradient(to_right,var(--accent)_var(--seek-fill,0%),transparent_var(--seek-fill,0%))] [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:[-webkit-appearance:none] [&::-webkit-slider-thumb]:mt-[-4.5px] [&::-webkit-slider-thumb]:size-2.5 [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-accent'
          onInput={(event) => player.seekTo(Number(event.currentTarget.value))}
        />
      )}
    </>
  );
};
