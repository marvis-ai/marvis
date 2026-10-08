import { useEffect, type RefObject } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';

/** The ended-session audio element. It stays mounted while metadata is
 * loading so header-only recordings can degrade without a controls row. */
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

  useEffect(() => {
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
    <audio
      ref={audioRef}
      src={assetSrc}
      preload='metadata'
      className='hidden'
      aria-hidden
      onLoadedMetadata={(event) => onReady(event.currentTarget.duration)}
      onTimeUpdate={(event) => onTime(event.currentTarget.currentTime)}
      onSeeking={(event) => onTime(event.currentTarget.currentTime)}
      onPlay={() => onPlayingChange(true)}
      onPause={() => onPlayingChange(false)}
      onEnded={(event) => {
        onPlayingChange(false);
        onTime(event.currentTarget.currentTime);
      }}
      onError={onError}
    />
  );
};
