import { describe, expect, test } from 'bun:test';
import type { Preset } from './commands';
import {
  expandTemplate,
  langName,
  matchPreset,
  presetToken,
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

describe('presetToken', () => {
  test('slugifies a name into the token charset', () => {
    expect(presetToken('Reply nicely')).toBe('reply-nicely');
    expect(presetToken("Devil's advocate")).toBe('devil-s-advocate');
    expect(presetToken('  Pad -- me!!  ')).toBe('pad-me');
    expect(presetToken('Reply')).toBe('reply');
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

  test('a spaced/punctuated name matches its slugged form', () => {
    // Customs have no memorable id suffix — the slug is their only
    // typeable shorthand.
    const custom = [
      P('u:a1', 'Reply nicely', 'template'),
      P('u:a2', "Devil's advocate", 'instruct'),
    ];
    expect(matchPreset('reply-nicely', custom)?.id).toBe('u:a1');
    expect(matchPreset('devil-s-advocate', custom)?.id).toBe('u:a2');
  });

  test('a slug collision resolves first-in-list', () => {
    const dups = [
      P('u:a1', 'Reply nicely', 'template'),
      P('u:a2', 'Reply, nicely!', 'template'),
    ];
    expect(matchPreset('reply-nicely', dups)?.id).toBe('u:a1');
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

  test('tab and newline also terminate the token', () => {
    expect(resolveSlash('/sum\tx', presets, false)?.rest).toBe('x');
    expect(resolveSlash('/sum\nx', presets, false)?.rest).toBe('x');
  });

  test('a custom name resolves via its slug', () => {
    const custom = [P('u:a1', 'Reply nicely', 'template')];
    expect(resolveSlash('/reply-nicely ', custom, false)?.preset.id).toBe(
      'u:a1',
    );
  });

  test('end-to-end: the longer name wins through the resolver', () => {
    // `/sum ` hits b:sum; `/summarize ` must reach b:summarize — a
    // prefix match must not swallow the full token.
    expect(resolveSlash('/summarize ', presets, false)?.preset.id).toBe(
      'b:summarize',
    );
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
  // Prototype keys are not languages — a plain-object lookup would leak
  // `Object.prototype.toString` here.
  expect(langName('toString')).toBe('English');
});
