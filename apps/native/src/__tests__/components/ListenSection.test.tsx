/// <reference types="bun-types" />
import { expect, mock, test } from 'bun:test';
import { GlobalWindow } from 'happy-dom';
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import type { ListenStatus, Transcript } from '@/lib/commands';
import type { ListenViewing } from '@/components/listen/model';

/** Handlers `useTauriEvent` registered, keyed by event name — the test
 *  emits backend payloads through them. */
const listeners = new Map<string, (event: { payload: unknown }) => void>();
const emit = (name: string, payload: unknown) =>
  listeners.get(name)?.({ payload });

mock.module('@tauri-apps/api/event', () => ({
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(name, handler);
    return Promise.resolve(() => {
      if (listeners.get(name) === handler) listeners.delete(name);
    });
  },
}));

const IDLE: ListenStatus = {
  state: 'idle',
  provider: null,
  session_id: null,
  audio_file: null,
  turns: 0,
  mic: false,
  error: null,
  started_at: null,
  paused_secs: 0,
  paused_since: null,
};

let status: ListenStatus = IDLE;
const rows = new Map<number, Transcript[]>();

mock.module('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => path,
  invoke: (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case 'listen_status':
        return Promise.resolve(status);
      case 'transcripts_for':
        return Promise.resolve(rows.get(args?.id as number) ?? []);
      case 'summary_latest':
        return Promise.resolve(null);
      case 'listen_stop':
        // The backend emits the idle state inside `listen_stop` — it can
        // reach the webview BEFORE the invoke resolves, so `session_id`
        // flips null while the same session's turns are still on screen.
        emit('listen:state', { ...IDLE });
        status = { ...IDLE };
        return Promise.resolve();
      default:
        return Promise.reject(new Error(`unmocked command: ${cmd}`));
    }
  },
}));

const { ListenSection } = await import('@/components/ListenSection');

/** Drive a controlled React input: the native setter mutates `.value`,
 *  then an `input` event lets React's synthetic `onChange` see it. */
const typeInto = async (
  win: GlobalWindow,
  input: HTMLInputElement,
  value: string,
) => {
  Object.getOwnPropertyDescriptor(
    win.HTMLInputElement.prototype,
    'value',
  )!.set!.call(input, value);
  await act(async () =>
    input.dispatchEvent(
      new win.Event('input', { bubbles: true }) as unknown as Event,
    ),
  );
};

const turn = (sessionId: number, text: string): Transcript => ({
  audio_start_ms: null,
  id: sessionId * 10,
  session_id: sessionId,
  speaker: 'them',
  speaker_idx: 0,
  content: text,
  ts: 1001,
});

const doc = (id: number): ListenViewing => ({
  id,
  startedAt: 1000,
  endedAt: 1005,
  audioFile: null,
  stt: null,
});

test('a rename survives stopping into the same session document', async () => {
  const win = new GlobalWindow();
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  status = { ...IDLE, state: 'listening', session_id: 7, started_at: 1000 };
  rows.set(7, [turn(7, 'hi')]);
  rows.set(9, [turn(9, 'other')]);
  let ended: ListenViewing | null = null;
  const ui = (viewing: ListenViewing | null) => (
    <ListenSection
      viewing={viewing}
      onSessionEnded={(v) => {
        ended = v;
      }}
      onBack={() => {}}
      onFollowUp={() => {}}
    />
  );
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(ui(null)));
    expect(host.textContent).toContain('Speaker 1');

    await act(async () =>
      (
        host.querySelector(
          '[aria-label="Rename Speaker 1"]',
        ) as HTMLButtonElement
      ).click(),
    );
    const input = host.querySelector('input') as HTMLInputElement;
    await typeInto(win, input, 'Alice');
    await act(async () =>
      input.dispatchEvent(
        new win.KeyboardEvent('keydown', {
          key: 'Enter',
          bubbles: true,
          cancelable: true,
        }) as unknown as Event,
      ),
    );
    expect(host.textContent).toContain('Alice');

    // Stop: the idle `listen:state` lands first (session_id → null),
    // then `onSessionEnded` swaps in the same session's finished doc.
    await act(async () =>
      (
        host.querySelector('[aria-label="Stop recording"]') as HTMLButtonElement
      ).click(),
    );
    expect(ended).not.toBeNull();
    expect(ended!.id).toBe(7);
    await act(async () => root.render(ui(ended)));
    expect(host.textContent).toContain('Alice');
    expect(host.textContent).not.toContain('Speaker 1');

    // A different session's document still clears the names.
    await act(async () => root.render(ui(doc(9))));
    expect(host.textContent).toContain('Speaker 1');
    expect(host.textContent).not.toContain('Alice');
  } finally {
    await act(async () => root.unmount());
    host.remove();
    await win.happyDOM.close();
  }
});
