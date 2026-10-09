/// <reference types="bun-types" />
import { afterEach, beforeEach, expect, mock, test } from 'bun:test';
import { GlobalWindow } from 'happy-dom';
import { act, useState } from 'react';
import type { Config, Preset } from '@/lib/commands';

const win = new GlobalWindow();
Object.assign(globalThis, {
  window: win,
  document: win.document,
  navigator: win.navigator,
  IS_REACT_ACT_ENVIRONMENT: true,
});
const { createRoot } = await import('react-dom/client');

const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
};
const listeners = new Set<() => void>();
let fetchList = () => Promise.resolve<Preset[]>([]);
const writes: {
  list: Preset[];
  result: ReturnType<typeof deferred<Config>>;
}[] = [];
mock.module('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => path,
  invoke: (command: string, args: { key: string; value: Preset[] }) => {
    if (command === 'presets_list') return fetchList();
    if (command === 'config_set' && args.key === 'prompts.custom') {
      const result = deferred<Config>();
      writes.push({ list: args.value, result });
      return result.promise;
    }
    throw new Error(`Unexpected command: ${command}`);
  },
}));
mock.module('@tauri-apps/api/event', () => ({
  listen: (_name: string, handler: (event: { payload: unknown }) => void) => {
    const emit = () => handler({ payload: {} });
    listeners.add(emit);
    return Promise.resolve(() => {
      listeners.delete(emit);
    });
  },
}));
const { usePresets } = await import('@/hooks/usePresets');
const { PresetsTab } = await import('@/components/prefs/PresetsTab');

const first: Preset = { id: 'u:first', name: ' First ', text: 'Be brief.' };
const second: Preset = { id: 'u:second', name: 'Second', text: 'Explain.' };
// Only the configuration fields consumed by this flow are needed.
const config = (custom: Preset[]) => ({ prompts: { custom } }) as Config;
const Settings = () => {
  const [current, setConfig] = useState(config([first, second]));
  return (
    <PresetsTab
      data={{
        config: current,
        setConfig,
        status: null,
        selected: null,
        setStatus: () => {},
        setSelected: () => {},
      }}
    />
  );
};
let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  fetchList = () => Promise.resolve([]);
  writes.length = 0;
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});
const click = async (label: string) => {
  const button = [...host.querySelectorAll('button')].find(
    (b) =>
      b.getAttribute('aria-label') === label || b.textContent?.trim() === label,
  );
  expect(button).toBeDefined();
  await act(async () => button!.click());
};
const confirm = async (index: number) => {
  await act(async () =>
    writes[index].result.resolve(config(writes[index].list)),
  );
};

test('a slower mount fetch cannot replace the latest config refresh', async () => {
  const mount = deferred<Preset[]>();
  const refresh = deferred<Preset[]>();
  let calls = 0;
  fetchList = () => (++calls === 1 ? mount.promise : refresh.promise);
  const Probe = () => (
    <div>
      {usePresets()
        .map((p) => p.name)
        .join(',')}
    </div>
  );
  await act(async () => root.render(<Probe />));
  await act(async () => {
    for (const emit of listeners) emit();
  });
  expect(calls).toBe(2);
  await act(async () => refresh.resolve([second]));
  expect(host.textContent).toBe('Second');
  await act(async () => mount.resolve([first]));
  expect(host.textContent).toBe('Second');
});

test('a deletion queued after a save preserves the confirmed edit', async () => {
  await act(async () => root.render(<Settings />));
  await click('Edit');
  await click('Save');
  await click('Delete Second');
  expect(writes).toHaveLength(1);
  await confirm(0);
  expect(writes).toHaveLength(2);
  expect(writes[1].list).toEqual([{ ...first, name: 'First' }]);
  await confirm(1);
  expect(host.querySelector('[aria-label="Delete Second"]')).toBeNull();
});

test('a save queued after a deletion does not restore the deleted preset', async () => {
  await act(async () => root.render(<Settings />));
  await click('Edit');
  await click('Delete Second');
  await click('Save');
  expect(writes).toHaveLength(1);
  await confirm(0);
  expect(writes[1].list).toEqual([{ ...first, name: 'First' }]);
  await confirm(1);
});

test('failed writes leave the confirmed list intact and the queue usable', async () => {
  await act(async () => root.render(<Settings />));
  await click('Edit');
  await click('Save');
  await click('Delete Second');
  await act(async () => writes[0].result.reject('Rejected'));
  expect(writes).toHaveLength(2);
  expect(writes[1].list).toEqual([first]);
  await confirm(1);
  expect(host.querySelector('input[aria-label="Preset name"]')).not.toBeNull();
  expect(host.textContent).toContain('Rejected');
});
