/// <reference types="bun-types" />
import { beforeAll, describe, expect, mock, test } from 'bun:test';
import { GlobalWindow } from 'happy-dom';
import { StrictMode, act } from 'react';
import { createRoot, type Root } from 'react-dom/client';

/** Handlers Tauri keeps alive — index 0 is the zombie left by the
 *  StrictMode cleanup racing the registration eval (tauri#15799): its
 *  unlisten throws, so the handler is NEVER removed from this set. */
const live = new Set<(event: { payload: unknown }) => void>();
let raced = true;
const emitToAll = (payload: unknown) => {
  for (const h of live) h({ payload });
};

mock.module('@tauri-apps/api/event', () => ({
  listen: (_name: string, handler: (event: { payload: unknown }) => void) => {
    live.add(handler);
    const zombie = raced;
    raced = false;
    return Promise.resolve(async () => {
      if (zombie) {
        // The racy unlisten throws before `plugin:event|unlisten` runs —
        // the backend listener leaks and `handler` stays in `live`.
        throw new TypeError(
          "undefined is not an object (evaluating 'listeners[eventId].handlerId')",
        );
      }
      live.delete(handler);
    });
  },
}));

const { useTauriEvent } = await import('../../lib/events');

let root: Root;
let host: HTMLElement;

const Probe = ({ cb }: { cb: (p: { text: string }) => void }) => {
  useTauriEvent<{ text: string }>('ask:chunk', cb);
  return null;
};

const mount = async (cb: (p: { text: string }) => void, strict = true) => {
  const ui = strict ? (
    <StrictMode>
      <Probe cb={cb} />
    </StrictMode>
  ) : (
    <Probe cb={cb} />
  );
  await act(async () => root.render(ui));
};

beforeAll(() => {
  const win = new GlobalWindow() as unknown as Window & typeof globalThis;
  const g = globalThis as Record<string, unknown>;
  g.window = win;
  g.document = win.document;
  g.navigator = win.navigator;
  g.IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

describe('useTauriEvent', () => {
  test('delivers each event once under StrictMode (zombie listener dropped)', async () => {
    // StrictMode mounts → cleans up → remounts. The cleanup's unlisten
    // throws in the registration gap, leaking the first handler: `live`
    // holds the zombie AND the real listener for the same event.
    const seen: string[] = [];
    await mount((p) => seen.push(p.text));
    expect(live.size).toBe(2);

    emitToAll({ text: 'A' });
    expect(seen).toEqual(['A']);
  });

  test('stops delivering after unmount', async () => {
    const seen: string[] = [];
    await mount((p) => seen.push(p.text), false);
    await act(async () => root.render(null));

    emitToAll({ text: 'B' });
    // Only the StrictMode zombie from the first test may still fire —
    // and it no-ops through its dead `active` flag.
    expect(seen).toEqual([]);
  });
});
