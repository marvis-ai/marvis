import { expect, mock, test } from 'bun:test';

const calls: { command: string; args?: unknown }[] = [];
mock.module('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: unknown) => {
    calls.push({ command, args });
    return Promise.resolve({});
  },
  // `@tauri-apps/api/event` (pulled in via lib/events) imports this from
  // core — a stub export is enough; `listen` never runs in this test.
  transformCallback: () => 0,
}));

const { memoryList, memoryUpdate, memoryDelete } =
  await import('@/lib/commands');
const { EV_MEMORY_CHANGED } = await import('@/lib/events');

test('memory wrappers preserve the Rust command names and arguments', async () => {
  await memoryList();
  await memoryUpdate(7, 'The user prefers concise answers.');
  await memoryDelete(7);
  expect(calls).toEqual([
    { command: 'memory_list', args: undefined },
    {
      command: 'memory_update',
      args: { id: 7, value: 'The user prefers concise answers.' },
    },
    { command: 'memory_delete', args: { id: 7 } },
  ]);
  expect(EV_MEMORY_CHANGED).toBe('memory:changed');
});
