import { describe, expect, test } from 'bun:test';
import type { Preset } from '@/lib/commands';
import {
  expandTemplate,
  hasLangParam,
  isTemplate,
  langName,
  matchPreset,
  presetToken,
  resolveSlash,
  slashQuery,
  slashToken,
  stripSlashToken,
} from '../../lib/presets';

const P = (id: string, name: string, text = ''): Preset => ({
  id,
  name,
  text,
});

const presets = [
  P('b:sum', 'Sum'),
  P('b:summarize', 'Summarize'),
  P('u:xx', 'Reply'),
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
    expect(slashToken('/u:xx  hello\nworld')).toEqual({
      token: 'u:xx',
      rest: '  hello\nworld',
    });
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
    const custom = [P('u:a1', 'Reply nicely'), P('u:a2', "Devil's advocate")];
    expect(matchPreset('reply-nicely', custom)?.id).toBe('u:a1');
    expect(matchPreset('devil-s-advocate', custom)?.id).toBe('u:a2');
  });

  test('a slug collision resolves first-in-list', () => {
    const dups = [P('u:a1', 'Reply nicely'), P('u:a2', 'Reply, nicely!')];
    expect(matchPreset('reply-nicely', dups)?.id).toBe('u:a1');
  });
});

describe('resolveSlash', () => {
  test('full preset IDs resolve eagerly and at send time', () => {
    expect(resolveSlash('/u:xx hello', presets, false)).toEqual({
      preset: presets[2],
      rest: 'hello',
    });
    expect(resolveSlash('/B:SUM', presets, true)?.preset.id).toBe('b:sum');
    expect(resolveSlash('/u:xx', presets, false)).toBeNull();
    expect(stripSlashToken('/u:xx  hello')).toBe(' hello');
  });

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
    const custom = [P('u:a1', 'Reply nicely')];
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

describe('isTemplate / hasLangParam', () => {
  test('{input} alone decides expansion — mirrors presets::is_template', () => {
    expect(isTemplate(P('u:t', 'T', 'Summarize:\n\n{input}'))).toBe(true);
    // `{lang}` alone stays a silent instruction.
    expect(isTemplate(P('u:l', 'L', 'Answer in {lang}'))).toBe(false);
    expect(isTemplate(P('u:p', 'P', 'Be terse.'))).toBe(false);
  });

  test('{lang} flags the editable param badge', () => {
    expect(hasLangParam(P('u:t', 'T', 'into {lang}:\n\n{input}'))).toBe(true);
    expect(hasLangParam(P('u:p', 'P', 'Be terse.'))).toBe(false);
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

describe('slashQuery / stripSlashToken', () => {
  test('slashQuery seeds the palette filter from the token', () => {
    expect(slashQuery('/trans hello')).toBe('trans');
    expect(slashQuery('/')).toBe('');
    expect(slashQuery('plain text')).toBeNull();
  });

  test('stripSlashToken drops the token and ONE space', () => {
    expect(stripSlashToken('/trans hello')).toBe('hello');
    expect(stripSlashToken('/')).toBe('');
    expect(stripSlashToken('/sum')).toBe('');
    // A second `/` ends the token — the rest survives untouched.
    expect(stripSlashToken('/x/y')).toBe('/y');
    // Non-slash text passes through untouched.
    expect(stripSlashToken('hello')).toBe('hello');
    // Extra spaces after the token belong to the user's text.
    expect(stripSlashToken('/sum  indented')).toBe(' indented');
  });
});
