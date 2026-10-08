import { useEffect, useState, type RefObject } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { elapsedLabel } from './model';

/** Metadata loads before showing controls so empty recordings can degrade. */
export const SessionPlayer = ({
  audioFile,
  audioRef,
  onTime,
  onReady,
  onError,
  onPlayingChange,
}: {
  audioFile: string;
  audioRef: RefObject<HTMLAudioElement | null>;
  onTime: (seconds: number) => void;
  onReady: (duration: number) => void;
  onError: () => void;
  onPlayingChange: (playing: boolean) => void;
}) => {
  const assetSrc = convertFileSrc(audioFile);
  const [position, setPosition] = useState(0);
  const [duration, setDuration] = useState(0);
  const updateTime = (audio: HTMLAudioElement) => {
    const seconds = Number.isFinite(audio.currentTime) ? audio.currentTime : 0;
    setPosition(seconds);
    onTime(seconds);
  };
  const updateMetadata = (audio: HTMLAudioElement) => {
    const seconds = audio.duration;
    setDuration(Number.isFinite(seconds) && seconds > 0 ? seconds : 0);
    onReady(seconds);
    updateTime(audio);
  };

  useEffect(() => {
    setPosition(0);
    setDuration(0);
    const audio = audioRef.current;
    if (!audio) return;

    audio.pause();
    audio.currentTime = 0;
    audio.load();
    return () => {
      audio.pause();
      audio.currentTime = 0;
    };
  }, [audioFile, audioRef]);

  return (
    <>
      <audio
        ref={audioRef}
        src={assetSrc}
        preload='metadata'
        className='hidden'
        aria-hidden
        onLoadedMetadata={(event) => updateMetadata(event.currentTarget)}
        onDurationChange={(event) => {
          if (Number.isFinite(event.currentTarget.duration) && event.currentTarget.duration > 0) {
            updateMetadata(event.currentTarget);
          }
        }}
        onTimeUpdate={(event) => updateTime(event.currentTarget)}
        onSeeking={(event) => updateTime(event.currentTarget)}
        onSeeked={(event) => updateTime(event.currentTarget)}
        onPlay={() => onPlayingChange(true)}
        onPause={() => onPlayingChange(false)}
        onEnded={(event) => {
          onPlayingChange(false);
          updateTime(event.currentTarget);
        }}
        onError={() => {
          setDuration(0);
          setPosition(0);
          onError();
        }}
      />
      {duration > 0 && (
        <div className='flex items-center gap-3 border-b border-border px-3 py-2 text-xs text-muted-foreground tabular-nums'>
          <span>{elapsedLabel(position)}</span>
          <input
            type='range'
            aria-label='Playback position'
            aria-valuetext={`${elapsedLabel(position)} of ${elapsedLabel(duration)}`}
            min={0}
            max={duration}
            step={0.1}
            value={Math.min(position, duration)}
            className='min-w-0 flex-1 accent-accent'
            onInput={(event) => {
              const audio = audioRef.current;
              if (!audio) return;
              audio.currentTime = Number(event.currentTarget.value);
              updateTime(audio);
            }}
          />
          <span>{elapsedLabel(duration)}</span>
        </div>
      )}
    </>
  );
};
