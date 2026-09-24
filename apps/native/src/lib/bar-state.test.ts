import { describe, expect, test } from 'bun:test';
import { barControls, hasActiveWork } from './bar-state';

describe('hasActiveWork', () => {
  test('is false when the bar is idle', () => {
    expect(
      hasActiveWork({
        ask: 'idle',
        listen: 'idle',
        dictation: 'idle',
      }),
    ).toBe(false);
  });

  test('is true for each active foreground state', () => {
    for (const state of [
      { ask: 'loading', listen: 'idle', dictation: 'idle' },
      { ask: 'streaming', listen: 'idle', dictation: 'idle' },
      { ask: 'idle', listen: 'listening', dictation: 'idle' },
      { ask: 'idle', listen: 'idle', dictation: 'listening' },
    ] as const) {
      expect(hasActiveWork(state)).toBe(true);
    }
  });
});

describe('barControls', () => {
  test('shows Listen only while collapsed and dictation only while expanded', () => {
    expect(barControls(false)).toEqual(['iris', 'capture', 'listen']);
    expect(barControls(true)).toEqual(['iris', 'dictation', 'settings']);
  });
});
