import { useEffect, useState, type RefObject } from 'react';
import { PauseIcon, PlayIcon } from '@marvis/ui';
import { convertFileSrc } from '@tauri-apps/api/core';
import { ICON_BTN, NUM, cn } from '@/lib/classes';
import { elapsedLabel, type TurnBlock } from './model';

/** The ended-session audio controls. The audio element stays mounted while
 * metadata is loading so header-only recordings can degrade without a row. */
export const SessionPlayer = ({
  audioFile,
  audioRef,
  blocks,
  activeBlock,
  audioReady,
  duration,
  currentTime,
  onTime,
  onSeek,
  onReady,
  onError,
}: {
  audioFile: string;
  audioRef: RefObject<HTMLAudioElement | null>;
  blocks: TurnBlock[];
  activeBlock: string | null;
  audioReady: boolean;
  duration: number;
  currentTime: number;
  onTime: (seconds: number) => void;
  onSeek: (seconds: number) => void;
  onReady: (duration: number) => void;
  onError: () => void;
}) => {
  const [playing, setPlaying] = useState(false);
  const assetSrc = convertFileSrc(audioFile);
  const activeName = activeBlock
    ? blocks.find((block) => `${block.key}-${block.ts}` === activeBlock)?.name
    : null;
  const displayTime = Math.min(Math.max(currentTime, 0), duration);

  useEffect(() => {
    const audio = audioRef.current;
    setPlaying(false);
    if (!audio) return;

    audio.pause();
    audio.currentTime = 0;
    audio.load();
    return () => {
      audio.pause();
      audio.currentTime = 0;
    };
  }, [audioFile, audioRef]);

  const togglePlayback = () => {
    const audio = audioRef.current;
    if (!audio) return;
    if (audio.paused) {
      void audio.play().catch(() => setPlaying(false));
    } else {
      audio.pause();
    }
  };

  const seek = (seconds: number) => {
    const audio = audioRef.current;
    if (audio) audio.currentTime = seconds;
    onSeek(seconds);
  };

  return (
    <>
      <audio
        ref={audioRef}
        src={assetSrc}
        preload='metadata'
        className='hidden'
        aria-hidden
        onLoadedMetadata={(event) => onReady(event.currentTarget.duration)}
        onTimeUpdate={(event) => onTime(event.currentTarget.currentTime)}
        onSeeking={(event) => onTime(event.currentTarget.currentTime)}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={(event) => {
          setPlaying(false);
          onTime(event.currentTarget.currentTime);
        }}
        onError={onError}
      />
      {audioReady && duration > 0 && (
        <div className='flex items-center gap-2 border-b border-border px-3 py-1.5'>
          <button
            type='button'
            className={cn(ICON_BTN, 'size-6 text-foreground')}
            aria-label={playing ? 'Pause session audio' : 'Play session audio'}
            title={playing ? 'Pause' : 'Play'}
            onClick={togglePlayback}>
            {playing ? (
              <PauseIcon className='size-3.5' />
            ) : (
              <PlayIcon className='size-3.5' />
            )}
          </button>
          <input
            type='range'
            min={0}
            max={duration}
            step={0.01}
            value={displayTime}
            aria-label={
              activeName
                ? `Seek ${activeName} at ${elapsedLabel(displayTime)}`
                : 'Seek session audio'
            }
            className='min-w-0 flex-1'
            onChange={(event) => seek(Number(event.currentTarget.value))}
          />
          <span
            className={cn(NUM, 'flex-none text-[10px] text-muted-foreground')}>
            {elapsedLabel(displayTime)} / {elapsedLabel(duration)}
          </span>
        </div>
      )}
    </>
  );
};
