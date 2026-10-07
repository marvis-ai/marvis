/// <reference types="bun-types" />
import { describe, expect, test } from 'bun:test';
import { tokenizeInline } from './richtext';

/** Flatten to [role, text] pairs so assertions read like the input. */
const parts = (src: string) =>
  tokenizeInline(src).map((t) => [t.role, src.slice(t.start, t.end)]);

describe('tokenizeInline', () => {
  test('plain text is a single text token', () => {
    expect(parts('just words')).toEqual([['text', 'just words']]);
  });

  test('detects a bare url with href', () => {
    expect(parts('see https://a.b/c now')).toEqual([
      ['text', 'see '],
      ['url', 'https://a.b/c'],
      ['text', ' now'],
    ]);
    const url = tokenizeInline('https://a.b/c')[0];
    expect(url.href).toBe('https://a.b/c');
  });

  test('strips trailing punctuation off autolinks', () => {
    expect(parts('check https://a.b, ok')).toEqual([
      ['text', 'check '],
      ['url', 'https://a.b'],
      ['text', ', ok'],
    ]);
  });

  test('keeps a balanced trailing paren inside the url', () => {
    expect(parts('https://en.wiki/x_(y)')).toEqual([
      ['url', 'https://en.wiki/x_(y)'],
    ]);
  });

  test('re-strips punctuation stranded behind a stripped closer', () => {
    expect(parts('see https://a.b,) now')).toEqual([
      ['text', 'see '],
      ['url', 'https://a.b'],
      ['text', ',) now'],
    ]);
    expect(parts('https://a.b.)')).toEqual([
      ['url', 'https://a.b'],
      ['text', '.)'],
    ]);
  });

  test('www. autolinks get an https:// href but display verbatim', () => {
    const t = tokenizeInline('go www.a.b/c')[1];
    expect(t.role).toBe('url');
    expect(t.href).toBe('https://www.a.b/c');
    expect('go www.a.b/c'.slice(t.start, t.end)).toBe('www.a.b/c');
  });

  test('detects emails with a mailto: href', () => {
    const t = tokenizeInline('mail me@x.io please')[1];
    expect(t.role).toBe('email');
    expect(t.href).toBe('mailto:me@x.io');
  });

  test('[label](url) emits mark/link/mark, never double-linkifies', () => {
    const toks = tokenizeInline('see [docs](https://a.b) now');
    expect(parts('see [docs](https://a.b) now')).toEqual([
      ['text', 'see '],
      ['mark', '['],
      ['link', 'docs'],
      ['mark', '](https://a.b)'],
      ['text', ' now'],
    ]);
    expect(toks[2].href).toBe('https://a.b');
  });

  test('** strong ** emits dimmable marks around strong content', () => {
    expect(parts('a **bold** b')).toEqual([
      ['text', 'a '],
      ['mark', '**'],
      ['strong', 'bold'],
      ['mark', '**'],
      ['text', ' b'],
    ]);
  });

  test('* and _ italicize; __ also makes strong', () => {
    expect(parts('*it* _em_ __st__')).toEqual([
      ['mark', '*'],
      ['em', 'it'],
      ['mark', '*'],
      ['text', ' '],
      ['mark', '_'],
      ['em', 'em'],
      ['mark', '_'],
      ['text', ' '],
      ['mark', '__'],
      ['strong', 'st'],
      ['mark', '__'],
    ]);
  });

  test('intraword underscores stay literal (foo_bar_baz)', () => {
    expect(parts('foo_bar_baz')).toEqual([['text', 'foo_bar_baz']]);
  });

  test('~~ strike ~~ works', () => {
    expect(parts('~~gone~~')).toEqual([
      ['mark', '~~'],
      ['strike', 'gone'],
      ['mark', '~~'],
    ]);
  });

  test('code spans mask inner marks and urls', () => {
    expect(parts('`**x** https://a.b`')).toEqual([
      ['mark', '`'],
      ['code', '**x** https://a.b'],
      ['mark', '`'],
    ]);
  });

  test('unmatched delimiters stay literal', () => {
    expect(parts('an **open pair')).toEqual([['text', 'an **open pair']]);
  });

  test('no nesting: **a *b* c** is one strong run', () => {
    expect(parts('**a *b* c**')).toEqual([
      ['mark', '**'],
      ['strong', 'a *b* c'],
      ['mark', '**'],
    ]);
  });
});
