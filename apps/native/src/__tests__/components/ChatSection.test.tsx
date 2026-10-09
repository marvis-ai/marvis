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
let currentResponse = '';
let currentAttachments: Message['attachments'] = [];
let currentState: 'idle' | 'loading' | 'streaming' = 'idle';
let currentSessionId: number | null = 7;
let endActiveCalls = 0;
/** The session `session_list` reports as open — `session_end_active`
 *  clears it, and a test that "sends" a fresh turn sets it to the
 *  minted session's id. */
const ACTIVE_SESSION = {
  id: 7,
  kind: 'ask',
  title: null,
  audio_file: null,
  stt: null,
  started_at: 1,
  ended_at: null,
  last_active_at: 1,
};
let activeSession: typeof ACTIVE_SESSION | null = ACTIVE_SESSION;
mock.module('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => `asset://localhost/${path}`,
  invoke: (command: string) => {
    switch (command) {
      case 'session_list':
        // Deferred read — a `loading` fold's refetch must observe
        // the session state at flush time (a `session_end_active`
        // may have landed mid-flight).
        return Promise.resolve().then(() =>
          activeSession ? [activeSession] : [],
        );
      case 'session_get':
        return Promise.resolve(sessionRows);
      case 'session_end_active':
        endActiveCalls += 1;
        activeSession = null;
        return Promise.resolve(true);
      case 'ask_current':
        return Promise.resolve({
          state: currentState,
          question: currentQuestion,
          response: currentResponse,
          error: null,
          attachments: currentAttachments,
          run: 1,
          session_id: currentSessionId,
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
  currentResponse = '';
  currentAttachments = [];
  currentState = 'idle';
  currentSessionId = 7;
  endActiveCalls = 0;
  activeSession = ACTIVE_SESSION;
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

test('New chat mid-stream drops the detached run\u2019s packets — it keeps streaming, just not here', async () => {
  // New Chat does NOT kill the run — it keeps streaming into the
  // ended session, every packet tagged `session_id: 7`. Packets
  // already in the IPC pipe or still arriving must never paint into
  // the cleared view (the reported bug).
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  const emitState = listeners.get('ask:state')!;
  const emitChunk = listeners.get('ask:chunk')!;
  const emitDone = listeners.get('ask:done')!;
  await act(async () =>
    emitState({
      payload: { state: 'loading', question: 'q', run: 1, session_id: 7 },
    }),
  );
  await act(async () =>
    emitChunk({ payload: { text: 'hel', run: 1, session_id: 7 } }),
  );
  expect(host.textContent).toContain('hel');

  await act(async () => newChatButton().click());
  expect(endActiveCalls).toBe(1);

  // The detached run keeps emitting — a failover `loading` re-emit,
  // chunks, `done`, its terminal `idle`: all drop on the session gate.
  await act(async () =>
    emitState({
      payload: {
        state: 'loading',
        question: 'q',
        run: 1,
        session_id: 7,
        attempt: 1,
      },
    }),
  );
  await act(async () =>
    emitChunk({ payload: { text: 'lo world', run: 1, session_id: 7 } }),
  );
  await act(async () =>
    emitDone({ payload: { full: 'hello world', run: 1, session_id: 7 } }),
  );
  await act(async () =>
    emitState({ payload: { state: 'idle', run: 1, session_id: 7 } }),
  );
  expect(host.textContent).not.toContain('hello world');
  expect(host.textContent).toContain('Ask Marvis');

  // A new send mints a fresh session — its run renders normally.
  activeSession = { ...ACTIVE_SESSION, id: 8 };
  await act(async () =>
    emitState({
      payload: { state: 'loading', question: 'next', run: 2, session_id: 8 },
    }),
  );
  await act(async () =>
    emitChunk({ payload: { text: 'fresh', run: 2, session_id: 8 } }),
  );
  expect(host.textContent).toContain('fresh');
});

test('New chat during a loading refetch drops the detached fold', async () => {
  // The `loading` fold is async (session refetch): clicking New chat
  // while it is in flight must not let it commit the detached run's
  // pair — by the time the fold resumes, the emit's session no longer
  // matches the view.
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  const emitState = listeners.get('ask:state')!;
  // Sync act: the fold starts and suspends on the session read.
  act(() =>
    emitState({
      payload: { state: 'loading', question: 'q', run: 1, session_id: 7 },
    }),
  );
  await act(async () => newChatButton().click());
  // The fold resumed during the click's flush — session 7 ended, so
  // the emit's `session_id` fails the gate.
  expect(host.textContent).not.toContain('q');
  expect(host.textContent).toContain('Ask Marvis');
});

test("a stopped run's strays drop once its tagged idle retires it", async () => {
  // The composer's stop (`ask_stop` → `abort`) cancels mid-stream and
  // emits `idle` tagged with the killed run — packets the task already
  // emitted must not paint post-mortem, in the SAME session view.
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  const emitState = listeners.get('ask:state')!;
  const emitChunk = listeners.get('ask:chunk')!;
  const emitDone = listeners.get('ask:done')!;
  await act(async () =>
    emitState({
      payload: { state: 'loading', question: 'q', run: 5, session_id: 7 },
    }),
  );
  await act(async () =>
    emitChunk({ payload: { text: 'he', run: 5, session_id: 7 } }),
  );
  expect(host.textContent).toContain('he');

  // `abort`'s own `idle` emit — tagged with the killed run.
  await act(async () =>
    emitState({ payload: { state: 'idle', run: 5, session_id: 7 } }),
  );
  // Strays the task emitted before the cancel landed.
  await act(async () =>
    emitChunk({ payload: { text: 'llo', run: 5, session_id: 7 } }),
  );
  await act(async () =>
    emitDone({ payload: { full: 'hello', run: 5, session_id: 7 } }),
  );
  const text = host.textContent ?? '';
  expect(text).toContain('he');
  expect(text).not.toContain('llo');
  expect(text).not.toContain('hello');
});

test("mount resync re-attaches a live run's tail on its own session", async () => {
  // Leaving and resuming the session a run is streaming into re-shows
  // the live tail — the stream was never killed.
  sessionRows = [userRow()];
  currentState = 'streaming';
  currentSessionId = 7;
  currentQuestion = 'what is this?';
  currentResponse = 'partial';
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  expect(host.textContent).toContain('partial');

  // And its live packets keep flowing — the session still matches.
  const emitChunk = listeners.get('ask:chunk')!;
  await act(async () =>
    emitChunk({ payload: { text: ' more', run: 1, session_id: 7 } }),
  );
  expect(host.textContent).toContain('partial more');
});

test("mount resync does NOT fold a live run's tail onto another chat", async () => {
  // The run writes to session 99 while this view shows 7 — neither
  // the resync tail nor its live packets may paint here.
  sessionRows = [userRow()];
  currentState = 'streaming';
  currentSessionId = 99;
  currentQuestion = 'foreign q';
  currentResponse = 'foreign';
  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  expect(host.textContent).toContain('what is this?');
  expect(host.textContent).not.toContain('foreign');

  const emitChunk = listeners.get('ask:chunk')!;
  await act(async () =>
    emitChunk({ payload: { text: 'foreign tail', run: 4, session_id: 99 } }),
  );
  expect(host.textContent).not.toContain('foreign tail');
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
