import { describe, expect, test } from 'bun:test';
import type { Preset } from './commands';
import {
  expandTemplate,
  langName,
  matchPreset,
  resolveSlash,
  slashToken,
} from './presets';

const P = (id: string, name: string, kind: Preset['kind']): Preset => ({
  id,
  name,
  kind,
  text: '',
});

const presets = [
  P('b:sum', 'Sum', 'instruct'),
  P('b:summarize', 'Summarize', 'instruct'),
  P('u:xx', 'Reply', 'template'),
];

describe('slashToken', () => {
  test('splits the caret-0 token from the rest', () => {
    expect(slashToken('hello')).toBeNull();
    expect(slashToken('/')).toEqual({ token: '', rest: '' });
    expect(slashToken('/sum')).toEqual({ token: 'sum', rest: '' });
    expect(slashToken('/sum hello world')).toEqual({
      token: 'sum',
      rest: ' hello world',
    });
    expect(slashToken('/sum.x')).toEqual({ token: 'sum', rest: '.x' });
    expect(slashToken('mid /sum')).toBeNull();
  });
});

describe('matchPreset', () => {
  test('matches id suffix, full id, or name — first in list order', () => {
    expect(matchPreset('sum', presets)?.id).toBe('b:sum');
    expect(matchPreset('SUM', presets)?.id).toBe('b:sum');
    expect(matchPreset('summarize', presets)?.id).toBe('b:summarize');
    expect(matchPreset('reply', presets)?.id).toBe('u:xx');
    expect(matchPreset('u:xx', presets)?.id).toBe('u:xx');
    expect(matchPreset('nope', presets)).toBeNull();
    expect(matchPreset('', presets)).toBeNull();
  });
});

describe('resolveSlash', () => {
  test('eager pass needs a whitespace terminator', () => {
    expect(resolveSlash('/sum ', presets, false)?.preset.id).toBe('b:sum');
    expect(resolveSlash('/sum', presets, false)).toBeNull();
    expect(resolveSlash('/sum', presets, true)?.preset.id).toBe('b:sum');
    expect(resolveSlash('/sumx ', presets, true)).toBeNull();
    expect(resolveSlash('no slash', presets, true)).toBeNull();
  });

  test('rest strips exactly one terminator space', () => {
    expect(resolveSlash('/sum  two', presets, false)?.rest).toBe(' two');
    expect(resolveSlash('/sum two', presets, false)?.rest).toBe('two');
  });
});

describe('expandTemplate', () => {
  test('substitutes {input} and {lang}', () => {
    expect(expandTemplate('T {lang}: {input}', 'hello', 'English')).toBe(
      'T English: hello',
    );
  });

  test('appends input when {input} is absent', () => {
    expect(expandTemplate('Do this', 'x', 'en')).toBe('Do this\n\nx');
    expect(expandTemplate('Do this', '', 'en')).toBe('Do this');
  });

  test('empty input clears the placeholder', () => {
    expect(expandTemplate('T: {input}', '', 'en')).toBe('T: ');
  });
});

test('langName mirrors prompts.rs', () => {
  expect(langName('zh')).toBe('Chinese');
  expect(langName('bogus')).toBe('English');
});
