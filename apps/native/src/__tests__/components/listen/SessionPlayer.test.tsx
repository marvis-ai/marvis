/// <reference types="bun-types" />
import { expect, test } from 'bun:test';
import { GlobalWindow } from 'happy-dom';
import { act, createRef } from 'react';
import { createRoot } from 'react-dom/client';
import { SessionPlayer } from '@/components/listen/SessionPlayer';

test('playback controls follow metadata, playback, external seeks, slider input, and source changes', async () => {
  const win = new GlobalWindow();
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  Object.assign(win, {
    __TAURI_INTERNALS__: { convertFileSrc: (path: string) => path },
  });
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const audioRef = createRef<HTMLAudioElement>();
  const times: number[] = [];
  const durations: number[] = [];
  const playing: boolean[] = [];
  let errors = 0;
  const render = async (audioFile: string) => {
    await act(async () =>
      root.render(
        <SessionPlayer
          audioFile={audioFile}
          audioRef={audioRef}
          onTime={(time) => times.push(time)}
          onReady={(duration) => durations.push(duration)}
          onPlayingChange={(value) => playing.push(value)}
          onError={() => errors++}
        />,
      ),
    );
  };
  const fire = async (target: Element, type: string) => {
    await act(async () =>
      target.dispatchEvent(
        new win.Event(type, { bubbles: true }) as unknown as Event,
      ),
    );
  };
  try {
    await render('first.wav');
    const audio = audioRef.current!;
    expect(host.querySelector('input')).toBeNull();
    Object.defineProperty(audio, 'duration', {
      configurable: true,
      value: 125,
    });
    await fire(audio, 'loadedmetadata');
    expect(durations[durations.length - 1]).toBe(125);
    const slider = host.querySelector('input')!;
    expect(slider.max).toBe('125');
    expect(host.textContent).toContain('2:05');
    Object.defineProperty(audio, 'duration', {
      configurable: true,
      value: 130,
    });
    await fire(audio, 'durationchange');
    expect(slider.max).toBe('130');
    audio.currentTime = 20;
    await fire(audio, 'timeupdate');
    expect(slider.value).toBe('20');
    expect(times[times.length - 1]).toBe(20);
    audio.currentTime = 30;
    await fire(audio, 'seeking');
    expect(slider.value).toBe('30');
    // Use the native setter so React sees an actual user value change.
    Object.getOwnPropertyDescriptor(
      win.HTMLInputElement.prototype,
      'value',
    )!.set!.call(slider, '45');
    await fire(slider, 'input');
    expect(audio.currentTime).toBe(45);
    expect(times[times.length - 1]).toBe(45);
    await fire(audio, 'play');
    await fire(audio, 'pause');
    expect(playing.slice(-2)).toEqual([true, false]);
    audio.currentTime = 125;
    await fire(audio, 'ended');
    expect(slider.value).toBe('125');
    expect(playing[playing.length - 1]).toBe(false);
    await render('second.wav');
    expect(host.querySelector('input')).toBeNull();
    expect(audio.currentTime).toBe(0);
    await fire(audio, 'error');
    expect(errors).toBe(1);
  } finally {
    await act(async () => root.unmount());
    host.remove();
    await win.happyDOM.close();
  }
});
