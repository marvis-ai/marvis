/// <reference types="bun-types" />
import { afterEach, beforeEach, expect, mock, test } from 'bun:test';
import { GlobalWindow } from 'happy-dom';
import { act } from 'react';
import type { Message } from '@/lib/commands';

const win = new GlobalWindow();
Object.assign(globalThis, {
  window: win,
  document: win.document,
  navigator: win.navigator,
  IS_REACT_ACT_ENVIRONMENT: true,
});
const { createRoot } = await import('react-dom/client');

/** `listen` registrations by event name — emitting delivers the
 *  payload to the hook's callback exactly once. */
const listeners = new Map<string, (event: { payload: unknown }) => void>();
let sessionRows: Message[] = [];
mock.module('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => `asset://localhost/${path}`,
  invoke: (command: string) => {
    switch (command) {
      case 'session_list':
        return Promise.resolve([
          {
            id: 7,
            kind: 'ask',
            title: null,
            audio_file: null,
            stt: null,
            started_at: 1,
            ended_at: null,
            last_active_at: 1,
          },
        ]);
      case 'session_get':
        return Promise.resolve(sessionRows);
      case 'ask_current':
        return Promise.resolve({
          state: 'idle',
          question: '',
          response: '',
          error: null,
          attachments: [],
        });
      case 'presets_list':
        return Promise.resolve([]);
      default:
        return Promise.reject(new Error(`Unexpected command: ${command}`));
    }
  },
}));
mock.module('@tauri-apps/api/event', () => ({
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(name, handler);
    return Promise.resolve(() => {
      listeners.delete(name);
    });
  },
}));
const { ChatSection } = await import('./ChatSection');

const attachment = {
  id: 1,
  message_id: 10,
  name: 'notes.png',
  path: '/root/.marvis/attachments/att-1.jpg',
  mime: 'image/jpeg',
  bytes: 4,
  position: 0,
};

const userRow = (over: Partial<Message> = {}): Message => ({
  id: 10,
  session_id: 7,
  role: 'user',
  content: 'what is this?',
  attachments: [attachment],
  provider: null,
  model: null,
  tokens_in: null,
  tokens_out: null,
  preset: null,
  ts: 1,
  ...over,
});

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  sessionRows = [];
  listeners.clear();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

test('persisted user attachments render as managed-path thumbnails', async () => {
  sessionRows = [
    userRow(),
    { ...userRow({ id: 11, role: 'assistant', content: 'a chart', attachments: [] }) },
  ];
  await act(async () => root.render(<ChatSection onBack={() => {}} />));

  const imgs = [...host.querySelectorAll('img')];
  expect(imgs).toHaveLength(1);
  expect(imgs[0].src).toBe(
    'asset://localhost//root/.marvis/attachments/att-1.jpg',
  );
  expect(imgs[0].alt).toBe('notes.png');
});

test('a loading payload with attachments renders them on the live user row', async () => {
  sessionRows = [];
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  const emit = listeners.get('ask:state');
  expect(emit).toBeDefined();
  await act(async () =>
    emit!({
      payload: {
        state: 'loading',
        question: 'look at this',
        attachments: [attachment],
      },
    }),
  );

  const imgs = [...host.querySelectorAll('img')];
  expect(imgs).toHaveLength(1);
  expect(imgs[0].alt).toBe('notes.png');
});
