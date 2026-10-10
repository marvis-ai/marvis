/// <reference types="bun-types" />
import { expect, test } from 'bun:test';
import { GlobalWindow } from 'happy-dom';
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { useSessionPlayer } from '@/hooks/useSessionPlayer';
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
  let player!: ReturnType<typeof useSessionPlayer>;
  const Harness = ({ file }: { file: string }) => {
    player = useSessionPlayer({
      file,
      ended: true,
      resetKey: null,
      blocks: [],
      startedAt: 0,
    });
    return <SessionPlayer player={player} />;
  };
  const render = async (file: string) => {
    await act(async () => root.render(<Harness file={file} />));
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
    const audio = host.querySelector('audio')!;
    expect(host.querySelector('input')).toBeNull();
    Object.defineProperty(audio, 'duration', {
      configurable: true,
      value: 125,
    });
    await fire(audio, 'loadedmetadata');
    expect(player.duration).toBe(125);
    expect(player.canSeek).toBe(true);
    const slider = host.querySelector('input')!;
    expect(slider.max).toBe('125');
    expect(slider.getAttribute('style')).toContain('--seek-fill: 0%');
    Object.defineProperty(audio, 'duration', {
      configurable: true,
      value: 130,
    });
    await fire(audio, 'durationchange');
    expect(slider.max).toBe('130');
    audio.currentTime = 20;
    await fire(audio, 'timeupdate');
    expect(slider.value).toBe('20');
    expect(player.position).toBe(20);
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
    expect(player.position).toBe(45);
    await fire(audio, 'play');
    expect(player.playing).toBe(true);
    await fire(audio, 'pause');
    expect(player.playing).toBe(false);
    audio.currentTime = 125;
    await fire(audio, 'ended');
    expect(slider.value).toBe('125');
    expect(player.playing).toBe(false);
    await render('second.wav');
    expect(host.querySelector('input')).toBeNull();
    expect(audio.currentTime).toBe(0);
    await fire(audio, 'error');
    expect(player.unavailable).toBe(true);
    // `visible` flips off on error — the element unmounts entirely.
    expect(host.querySelector('audio')).toBeNull();
  } finally {
    await act(async () => root.unmount());
    host.remove();
    await win.happyDOM.close();
  }
});
