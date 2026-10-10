/// <reference types="bun-types" />
import { afterEach, beforeEach, expect, mock, test } from 'bun:test';
import { GlobalWindow } from 'happy-dom';
import { act } from 'react';
import type { Config, Memory, ModelSelection } from '@/lib/commands';

const win = new GlobalWindow();
Object.assign(globalThis, {
  window: win,
  document: win.document,
  navigator: win.navigator,
  IS_REACT_ACT_ENVIRONMENT: true,
});
const { createRoot } = await import('react-dom/client');

let facts: Memory[] = [];
const configWrites: { key: string; value: unknown }[] = [];
const commands: (string | number | boolean | null)[][] = [];
const listeners = new Set<() => void>();
mock.module('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => path,
  invoke: (command: string, args?: Record<string, unknown>) => {
    if (command === 'config_set') {
      configWrites.push({ key: args!.key as string, value: args!.value });
      return Promise.resolve(config);
    }
    if (command === 'memory_list') return Promise.resolve(facts);
    if (command === 'memory_update') {
      commands.push([
        'memory_update',
        args!.id as number,
        args!.value as string,
      ]);
      const row = facts.find((f) => f.id === args!.id);
      return Promise.resolve({ ...row!, value: args!.value });
    }
    if (command === 'memory_delete') {
      commands.push(['memory_delete', args!.id as number]);
      return Promise.resolve();
    }
    if (command === 'model_list_available')
      return Promise.resolve(['gpt-4o', 'gpt-4o-mini']);
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

const { MemoryTab } = await import('@/components/prefs/MemoryTab');

const config = {
  memory: { enabled: false, provider: '', model: '' },
  providers: { order: ['openai'], disabled: [], models: { openai: 'gpt-4o' } },
  compat: { name: '', base_url: '' },
} as unknown as Config;
const fact: Memory = {
  id: 7,
  category: 'preference',
  attribute: 'response_style',
  value: 'The user prefers concise answers.',
  confidence: 0.86,
  basis: 'inferred',
  source: 'automatic',
  source_session_id: 4,
  source_message_id: 9,
  created_at: 1,
  updated_at: 2,
};
const prefsData = (c: Config, selected: ModelSelection | null) => ({
  config: c,
  selected,
  status: null,
  setStatus: () => {},
  setConfig: () => {},
  setSelected: () => {},
});

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  facts = [];
  configWrites.length = 0;
  commands.length = 0;
  listeners.clear();
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
// Native setter + happy-dom Event — the patched React setter would
// update the input's value tracker and swallow the synthetic change.
const typeIn = async (input: HTMLInputElement, value: string) => {
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

test('renders the suggested current Ask model without enabling memory', async () => {
  await act(async () =>
    root.render(<MemoryTab data={prefsData(config, null)} />),
  );
  expect(host.textContent).toContain('Memory is off');
  expect(host.textContent).toContain('gpt-4o');
  expect(host.querySelector('[aria-label="Enable memory"]')).not.toBeNull();
});

test('confirms before enabling and disabling', async () => {
  await act(async () =>
    root.render(<MemoryTab data={prefsData(config, null)} />),
  );
  await click('Enable memory');
  expect(host.textContent).toContain('Click to confirm');
  expect(configWrites).toHaveLength(0);
  await click('Click to confirm');
  expect(configWrites).toEqual([
    { key: 'memory.provider', value: 'openai' },
    { key: 'memory.model', value: 'gpt-4o' },
    { key: 'memory.enabled', value: true },
  ]);

  // Enabled — the first click only arms, the second writes.
  const on = {
    ...config,
    memory: { enabled: true, provider: 'openai', model: 'gpt-4o' },
  } as unknown as Config;
  await act(async () => root.render(<MemoryTab data={prefsData(on, null)} />));
  configWrites.length = 0;
  await click('Disable memory');
  expect(host.textContent).toContain('Click to confirm');
  expect(configWrites).toHaveLength(0);
  await click('Click to confirm');
  expect(configWrites).toContainEqual({ key: 'memory.enabled', value: false });
});

test('edits, deletes, and refreshes profile facts', async () => {
  facts = [fact];
  await act(async () =>
    root.render(<MemoryTab data={prefsData(config, null)} />),
  );
  expect(host.textContent).toContain('The user prefers concise answers.');
  await click('Edit response_style');
  const input = host.querySelector(
    'input[aria-label="Memory value"]',
  ) as HTMLInputElement;
  await typeIn(input, 'The user prefers short answers.');
  await click('Save memory');
  await click('Delete response_style');
  await click('Click to confirm');
  for (const emit of listeners) emit();
  await act(async () => Promise.resolve());
  expect(commands).toContainEqual([
    'memory_update',
    7,
    'The user prefers short answers.',
  ]);
  expect(commands).toContainEqual(['memory_delete', 7]);
});
