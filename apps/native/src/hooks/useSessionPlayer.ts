/**
 * Ended-session audio for the Listen doc — owns the hidden `<audio>`
 * element, its playback state (position/duration/playing/readiness),
 * and the transcript coupling (`activeBlock` follows `currentTime`; a
 * block seek jumps + plays). `SessionPlayer` is the matching view: it
 * spreads `audio` onto the element and renders the seek line.
 */
import { useEffect, useRef, useState } from 'react';
import type { SyntheticEvent } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import {
  activeBlockAt,
  audioOffset,
  type TurnBlock,
} from '@/components/listen/model';

export const useSessionPlayer = ({
  file,
  ended,
  resetKey,
  blocks,
  startedAt,
}: {
  /** The recorded file for the session on screen — null while live. */
  file: string | null;
  /** Audio exists only for an ended capture (`status.state === 'idle'`
   *  live, `endedAt != null` viewed). */
  ended: boolean;
  /** Identity of the document on screen (`viewing?.id`) — a switch
   *  pauses playback and clears every audio state. */
  resetKey: number | null;
  /** Transcript blocks — `activeBlock` resolution only. */
  blocks: TurnBlock[];
  /** Capture start for `audioOffset` mapping — null disables the
   *  player outright. */
  startedAt: number | null;
}) => {
  const audioRef = useRef<HTMLAudioElement>(null);
  const [position, setPosition] = useState(0);
  const [duration, setDuration] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [ready, setReady] = useState(false);
  const [unavailable, setUnavailable] = useState(false);
  const [activeBlock, setActiveBlock] = useState<string | null>(null);

  /** The shared `currentTime` read for timeupdate/seeking/seeked/ended
   *  and the seek line's input: position mirrors it and `activeBlock`
   *  follows it, unless the element has ended. */
  const syncTime = (audio: HTMLAudioElement) => {
    const seconds = Number.isFinite(audio.currentTime) ? audio.currentTime : 0;
    setPosition(seconds);
    setActiveBlock(
      audio.ended ? null : activeBlockAt(blocks, startedAt, seconds),
    );
  };

  /** Metadata gates the bar — an empty or corrupt recording degrades
   *  to `unavailable` instead of showing a dead seek line. */
  const updateMetadata = (audio: HTMLAudioElement) => {
    const seconds = audio.duration;
    const valid = Number.isFinite(seconds) && seconds > 0;
    setDuration(valid ? seconds : 0);
    setReady(valid);
    setUnavailable(!valid);
    if (!valid) setActiveBlock(null);
    syncTime(audio);
  };

  /** The seek line's input — move `currentTime` and mirror it. */
  const seekTo = (seconds: number) => {
    const audio = audioRef.current;
    if (!audio) return;
    audio.currentTime = seconds;
    syncTime(audio);
  };

  /** ±15s header controls — clamped to [0, duration]. */
  const skip = (seconds: number) => {
    const audio = audioRef.current;
    if (!audio || duration <= 0) return;
    const next = Math.min(Math.max(audio.currentTime + seconds, 0), duration);
    audio.currentTime = next;
    setActiveBlock(activeBlockAt(blocks, startedAt, next));
  };

  const toggle = () => {
    const audio = audioRef.current;
    if (!audio) return;
    if (audio.paused) {
      void audio.play().catch(() => setPlaying(false));
    } else {
      audio.pause();
    }
  };

  /** Transcript click-to-seek — jump to the block's capture offset and
   *  play; a failed play clears the highlight it just set. */
  const seekBlock = (block: TurnBlock) => {
    const audio = audioRef.current;
    if (!audio || startedAt == null) return;
    const seconds = audioOffset(block, startedAt);
    audio.currentTime = seconds;
    setActiveBlock(activeBlockAt(blocks, startedAt, seconds));
    void audio.play().catch(() => setActiveBlock(null));
  };

  // Playback belongs to the viewed document. Stop and clear it before
  // a document switch (file/ended/resetKey), and again on unmount in
  // case the element is still mounted.
  useEffect(() => {
    const audio = audioRef.current;
    audio?.pause();
    if (audio) {
      audio.currentTime = 0;
      audio.load();
    }
    setPosition(0);
    setDuration(0);
    setReady(false);
    setPlaying(false);
    setActiveBlock(null);
    setUnavailable(false);
    return () => {
      audio?.pause();
      if (audio) audio.currentTime = 0;
    };
  }, [file, ended, resetKey]);

  return {
    /** Spread onto the hidden `<audio>` — ref, resolved `src`, and all
     *  the DOM-event plumbing. */
    audio: {
      ref: audioRef,
      src: file === null ? undefined : convertFileSrc(file),
      onLoadedMetadata: (event: SyntheticEvent<HTMLAudioElement>) =>
        updateMetadata(event.currentTarget),
      onDurationChange: (event: SyntheticEvent<HTMLAudioElement>) => {
        if (
          Number.isFinite(event.currentTarget.duration) &&
          event.currentTarget.duration > 0
        ) {
          updateMetadata(event.currentTarget);
        }
      },
      onTimeUpdate: (event: SyntheticEvent<HTMLAudioElement>) =>
        syncTime(event.currentTarget),
      onSeeking: (event: SyntheticEvent<HTMLAudioElement>) =>
        syncTime(event.currentTarget),
      onSeeked: (event: SyntheticEvent<HTMLAudioElement>) =>
        syncTime(event.currentTarget),
      onPlay: () => setPlaying(true),
      onPause: () => setPlaying(false),
      onEnded: (event: SyntheticEvent<HTMLAudioElement>) => {
        setPlaying(false);
        syncTime(event.currentTarget);
      },
      onError: () => {
        setDuration(0);
        setPosition(0);
        setReady(false);
        setPlaying(false);
        setActiveBlock(null);
        setUnavailable(true);
      },
    },
    /** Whether the view should render at all — file + ended +
     *  loadable + a clock to offset against. */
    visible: file !== null && ended && !unavailable && startedAt !== null,
    /** Controls and transcript seeking gate — needs valid metadata. */
    canSeek: file !== null && ended && !unavailable && ready && duration > 0,
    /** WAV export gate. */
    canSave: file !== null && ended && ready,
    playing,
    position,
    duration,
    /** The recording failed or decoded empty — distinct from "no
     *  file": the session exists but its audio can't play. */
    unavailable,
    activeBlock,
    seekTo,
    skip,
    toggle,
    seekBlock,
  };
};

/** The hook's return — `SessionPlayer` takes it whole. */
export type SessionPlayerHandle = ReturnType<typeof useSessionPlayer>;
