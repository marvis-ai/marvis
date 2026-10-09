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
let currentQuestion = '';
let currentAttachments: Message['attachments'] = [];
let endActiveCalls = 0;
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
      case 'session_end_active':
        endActiveCalls += 1;
        return Promise.resolve(true);
      case 'ask_current':
        return Promise.resolve({
          state: 'idle',
          question: currentQuestion,
          response: '',
          error: null,
          attachments: currentAttachments,
          run: 1,
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
const { ChatSection } = await import('@/components/ChatSection');

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
  currentQuestion = '';
  currentAttachments = [];
  endActiveCalls = 0;
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
    {
      ...userRow({
        id: 11,
        role: 'assistant',
        content: 'a chart',
        attachments: [],
      }),
    },
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

test('a loading emit that beat the listener heals from the service fold on streaming', async () => {
  // The card opened mid-capture: the `loading` emit was missed and the
  // mount-resync raced the persist, so the user row painted bare. The
  // service-side `current_attachments` fold still has the metadata —
  // the `streaming` heal merges it onto the matching row.
  sessionRows = [userRow({ attachments: [] })];
  currentQuestion = 'what is this?';
  currentAttachments = [attachment];
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  expect(host.querySelectorAll('img')).toHaveLength(0);
  const emit = listeners.get('ask:state');
  expect(emit).toBeDefined();
  await act(async () => emit!({ payload: { state: 'streaming' } }));

  const imgs = [...host.querySelectorAll('img')];
  expect(imgs).toHaveLength(1);
  expect(imgs[0].alt).toBe('notes.png');
});

test('an empty service fold never strips a row of its attachments', async () => {
  // `ask_current` answers `[]` before the run's emit lands — that's
  // "not yet known", not "none": the painted row keeps its own
  // attachments.
  sessionRows = [userRow()];
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  const emit = listeners.get('ask:state');
  await act(async () => emit!({ payload: { state: 'streaming' } }));

  const imgs = [...host.querySelectorAll('img')];
  expect(imgs).toHaveLength(1);
  expect(imgs[0].alt).toBe('notes.png');
});

const newChatButton = () =>
  [...host.querySelectorAll('button')].find(
    (b) => b.textContent === 'New chat',
  )!;

test("New chat mid-stream drops the ended run's in-flight packets", async () => {
  // The stream emits continuously — packets already in the IPC pipe
  // when `session_end_active` aborts the run must not re-paint the
  // cleared list (the reported bug).
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  const emitState = listeners.get('ask:state')!;
  const emitChunk = listeners.get('ask:chunk')!;
  const emitDone = listeners.get('ask:done')!;
  await act(async () =>
    emitState({ payload: { state: 'loading', question: 'q', run: 1 } }),
  );
  await act(async () => emitChunk({ payload: { text: 'hel', run: 1 } }));
  expect(host.textContent).toContain('hel');

  await act(async () => newChatButton().click());
  expect(endActiveCalls).toBe(1);

  // Late run-1 packets — emitted before the abort landed — all drop.
  await act(async () => emitChunk({ payload: { text: 'lo world', run: 1 } }));
  await act(async () => emitDone({ payload: { full: 'hello world', run: 1 } }));
  await act(async () => emitState({ payload: { state: 'idle', run: 1 } }));
  expect(host.textContent).not.toContain('hello world');
  expect(host.textContent).toContain('Ask Marvis');

  // A NEW run is unaffected — suppression is scoped to the dead one.
  await act(async () =>
    emitState({ payload: { state: 'loading', question: 'next', run: 2 } }),
  );
  await act(async () => emitChunk({ payload: { text: 'fresh', run: 2 } }));
  expect(host.textContent).toContain('fresh');
});

test('New chat during a loading refetch drops the dead fold', async () => {
  // The `loading` fold is async (session refetch): clicking New chat
  // while it is in flight must not let it commit the dead run's pair.
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  const emitState = listeners.get('ask:state')!;
  // Sync act: the fold starts and suspends on the session read.
  act(() =>
    emitState({ payload: { state: 'loading', question: 'q', run: 1 } }),
  );
  await act(async () => newChatButton().click());
  // The fold resumed during the click's flush — dead run → dropped.
  expect(host.textContent).not.toContain('q');
  expect(host.textContent).toContain('Ask Marvis');
});

test('a same-text re-send appends a second pair instead of hiding the first', async () => {
  // The reported bug: re-asking the identical question hit the
  // failover fold (same `loading` shape) and silently dropped the
  // previous reply — while the DB, and the history view on reopen,
  // kept both turns. `attempt` now marks the boundary: attempt 0 of
  // a NEW run appends like any fresh turn.
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  const emitState = listeners.get('ask:state')!;
  const emitChunk = listeners.get('ask:chunk')!;
  const emitDone = listeners.get('ask:done')!;

  await act(async () =>
    emitState({
      payload: { state: 'loading', question: 'same?', run: 1, attempt: 0 },
    }),
  );
  await act(async () => emitChunk({ payload: { text: 'first', run: 1 } }));
  await act(async () => emitDone({ payload: { full: 'first', run: 1 } }));
  await act(async () => emitState({ payload: { state: 'idle', run: 1 } }));

  await act(async () =>
    emitState({
      payload: { state: 'loading', question: 'same?', run: 2, attempt: 0 },
    }),
  );
  await act(async () => emitChunk({ payload: { text: 'second', run: 2 } }));
  await act(async () => emitDone({ payload: { full: 'second', run: 2 } }));

  const text = host.textContent ?? '';
  expect(text).toContain('first');
  expect(text).toContain('second');
  // Two user bubbles — the second turn is its own row, like history.
  expect(host.querySelectorAll('.rounded-br-sm')).toHaveLength(2);
});

test('a failover retry (attempt>0) drops the dead attempt\u2019s partial', async () => {
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  const emitState = listeners.get('ask:state')!;
  const emitChunk = listeners.get('ask:chunk')!;
  const emitDone = listeners.get('ask:done')!;

  await act(async () =>
    emitState({
      payload: { state: 'loading', question: 'q', run: 1, attempt: 0 },
    }),
  );
  await act(async () => emitChunk({ payload: { text: 'par', run: 1 } }));
  expect(host.textContent).toContain('par');

  // Provider one died mid-stream — the chain re-announces `loading`
  // with attempt 1: the partial resets before the next stream.
  await act(async () =>
    emitState({
      payload: { state: 'loading', question: 'q', run: 1, attempt: 1 },
    }),
  );
  await act(async () => emitChunk({ payload: { text: 'full', run: 1 } }));
  await act(async () => emitDone({ payload: { full: 'full', run: 1 } }));

  const text = host.textContent ?? '';
  expect(text).toContain('full');
  expect(text).not.toContain('par');
});

test('a regenerate folds in place — one pair, rejected reply dropped', async () => {
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  const emitState = listeners.get('ask:state')!;
  const emitDone = listeners.get('ask:done')!;

  await act(async () =>
    emitState({
      payload: { state: 'loading', question: 'q', run: 1, attempt: 0 },
    }),
  );
  await act(async () => emitDone({ payload: { full: 'a1', run: 1 } }));
  await act(async () => emitState({ payload: { state: 'idle', run: 1 } }));

  // `ask_retry` — a new run flagged `regenerate`: the rejected reply's
  // row is already deleted server-side, so the fold resets the tail.
  await act(async () =>
    emitState({
      payload: {
        state: 'loading',
        question: 'q',
        run: 2,
        attempt: 0,
        regenerate: true,
      },
    }),
  );
  await act(async () => emitDone({ payload: { full: 'a2', run: 2 } }));

  const text = host.textContent ?? '';
  expect(text).toContain('a2');
  expect(text).not.toContain('a1');
  expect(host.querySelectorAll('.rounded-br-sm')).toHaveLength(1);
});
