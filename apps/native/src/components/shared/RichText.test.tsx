/// <reference types="bun-types" />
import { describe, expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { RichText } from './RichText';

describe('RichText', () => {
  test('highlight mode keeps marks as dimmed spans', () => {
    const html = renderToStaticMarkup(<RichText text='**b**' />);
    expect(html).toContain('**');
    expect(html).toContain('b');
    expect(html).not.toContain('<strong');
  });

  test('interactive mode drops marks, emits semantic elements', () => {
    const html = renderToStaticMarkup(
      <RichText
        text='**b** _i_ `c`'
        interactive
      />,
    );
    expect(html).toContain('<strong>b</strong>');
    expect(html).toContain('<em>i</em>');
    expect(html).toContain('<code');
    expect(html).not.toContain('**');
  });

  test('interactive mode renders anchors with resolved hrefs', () => {
    const src = 'www.a.b and me@x.io and [docs](https://a.b)';
    const html = renderToStaticMarkup(
      <RichText
        text={src}
        interactive
      />,
    );
    expect(html).toContain('href="https://www.a.b"');
    expect(html).toContain('href="mailto:me@x.io"');
    expect(html).toContain('href="https://a.b"');
    expect(html).toContain('>docs</a>');
  });

  test('highlight mode keeps em/code as spans, never elements', () => {
    const html = renderToStaticMarkup(<RichText text='_i_ `c`' />);
    expect(html).not.toContain('<em');
    expect(html).not.toContain('<code');
    expect(html).toContain('<span');
  });

  // `inline-block` is atomic — a multi-word em must split at whitespace
  // so it can wrap where the textarea beneath does (caret alignment).
  test('highlight mode splits multi-word em, space stays raw text', () => {
    const html = renderToStaticMarkup(<RichText text='_a b_' />);
    expect(html).toContain('a</span> <span');
  });

  test('interactive mode renders ~~ as <s>', () => {
    const html = renderToStaticMarkup(
      <RichText
        text='~~s~~'
        interactive
      />,
    );
    expect(html).toContain('<s>s</s>');
  });

  test('highlight mode renders no anchors', () => {
    const html = renderToStaticMarkup(<RichText text='https://a.b' />);
    expect(html).not.toContain('<a');
  });
});
