# Rich Ask Input + Formatted User Bubbles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Ask input live-highlights links, emails, and inline
markdown marks while typing, and the sent user bubble renders the same
formatting with clickable links — via one shared tokenizer.

**Architecture:** A pure `tokenizeInline` in `src/lib/richtext.ts`
produces flat, non-overlapping `{start, end, role, href?}` tokens.
`RichText` (`src/components/shared/RichText.tsx`) renders tokens in two
modes: `highlight` (spans, for the input overlay) and `interactive`
(semantic elements + `openUrl` anchors, for user bubbles). `AskInput`
keeps its real `<textarea>` — text goes transparent over a mirrored
highlight `<div>` — so `useDictation`'s `selectionStart`-based caret
math and the CJK IME handling are untouched.

**Spec:** `docs/superpowers/specs/2026-10-07-rich-ask-input-design.md`

**Tech Stack:** React 19, Tailwind CSS 4, `bun test`,
`@tauri-apps/plugin-opener` (`openUrl`), no new dependencies.

## Global Constraints

- Package manager is **bun** (`bun test`, `bun run check-types` in
  `apps/native`).
- Components are arrow functions with named exports; imports inside
  `apps/native/src` use the `@/` alias, never `../../`.
- The textarea keeps a real editable surface — **do not** introduce
  `contenteditable` or an editor library.
- **Metric-safe overlay styles only:** any property that changes glyph
  advance (font-weight, font-family, font-size, letter-spacing, padding
  on inline spans) desyncs the overlay from the invisible caret. Bold
  is faked with `-webkit-text-stroke`, italic with `skewX`, decoration
  via color/underline/background only.
- Links in `interactive` mode open via `openUrl` from
  `@tauri-apps/plugin-opener` (the `Markdown.tsx` pattern) — never raw
  navigation.
- `bun test` picks up `*.test.ts(x)` next to sources; test files start
  with `/// <reference types="bun-types" />` and import from `bun:test`.

---

### Task 1: `tokenizeInline` tokenizer

**Files:**

- Create: `apps/native/src/lib/richtext.ts`
- Test: `apps/native/src/lib/richtext.test.ts`

**Interfaces:**

- Produces: `TokenRole` (`'text' | 'mark' | 'strong' | 'em' | 'strike'
  | 'code' | 'url' | 'email' | 'link'`), `Token` (`{start, end, role,
  href?}`), `tokenizeInline(text: string): Token[]` — consumed by
  `RichText` in Task 2.

- [ ] **Step 1: Write the failing test**

`apps/native/src/lib/richtext.test.ts`:

```ts
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

  test('www. autolinks get an https:// href but display verbatim', () => {
    const t = tokenizeInline('go www.a.b/c')[0];
    expect(t.role).toBe('url');
    expect(t.href).toBe('https://www.a.b/c');
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/native && bun test src/lib/richtext.test.ts`
Expected: FAIL — `Cannot find module './richtext'` (or
`tokenizeInline is not a function`).

- [ ] **Step 3: Implement `tokenizeInline`**

`apps/native/src/lib/richtext.ts`:

```ts
/**
 * Inline rich-text detection shared by the Ask input's highlight
 * overlay and the chat's user bubbles — links, emails, and the markdown
 * pair marks (`**`, `__`, `*`, `_`, `~~`, `` ` ``, `[label](url)`).
 * Emits flat, non-overlapping tokens: marks stay separate spans so the
 * overlay can dim them, and nothing nests — unmatched delimiters just
 * stay literal text.
 */
export type TokenRole =
  | 'text'
  | 'mark'
  | 'strong'
  | 'em'
  | 'strike'
  | 'code'
  | 'url'
  | 'email'
  | 'link';

export interface Token {
  start: number;
  end: number;
  role: TokenRole;
  /** url/email/link only — emails carry a `mailto:` href and `www.`
   *  autolinks an `https://`-prefixed one. */
  href?: string;
}

/* Alternative order is precedence at equal offsets: code masks inner
   syntax, `[label](url)` wins over a bare autolink, `**`/`__`/`~~` beat
   their single-char forms. `_`/`__` also need non-word boundaries so
   `foo_bar_baz` stays plain (`*`/`**` may sit intraword). */
const INLINE = new RegExp(
  [
    '`(?<code>[^`]+)`',
    '\\[(?<label>[^\\]\\n]+)\\]' +
      '\\((?<target>(?:https?://|mailto:|www\\.)[^\\s)]+)\\)',
    '(?<url>https?://[^\\s<>\'")\\]]+|www\\.[^\\s<>\'")\\]]+)',
    '(?<mail>[\\w.+-]+@[\\w-]+(?:\\.[\\w-]+)+)',
    '(?<strong>\\*\\*(?=\\S)[\\s\\S]*?\\S\\*\\*' +
      '|(?<![\\w])__(?=\\S)[\\s\\S]*?\\S__(?![\\w]))',
    '(?<strike>~~(?=\\S)[\\s\\S]*?\\S~~)',
    '(?<em>\\*(?=\\S)[\\s\\S]*?\\S\\*' +
      '|(?<![\\w])_(?=\\S)[\\s\\S]*?\\S_(?![\\w]))',
  ].join('|'),
  'g',
);

/** Wrapping punctuation the reader didn't mean as part of the url. */
const TRAILING = /[.,!?;:'"]+$/;

/** Strip sentence-tail punctuation, plus a trailing `)`/`]` only when
 *  it has no opener inside the match — `x_(y)` keeps its paren. */
const trimUrlTail = (raw: string): string => {
  let s = raw.replace(TRAILING, '');
  while (s.endsWith(')') || s.endsWith(']')) {
    const open = s.endsWith(')') ? '(' : '[';
    const close = s.endsWith(')') ? ')' : ']';
    if (
      s.split(close).length - 1 <=
      s.split(open).length - 1
    ) {
      break;
    }
    s = s.slice(0, -1);
  }
  return s;
};

const push = (
  out: Token[],
  start: number,
  end: number,
  role: TokenRole,
  href?: string,
) => {
  if (end > start) out.push({ start, end, role, href });
};

export const tokenizeInline = (text: string): Token[] => {
  const out: Token[] = [];
  let cursor = 0;
  for (const m of text.matchAll(INLINE)) {
    const at = m.index!;
    const raw = m[0];
    const g = m.groups!;
    if (at > cursor) push(out, cursor, at, 'text');
    if (g.code !== undefined) {
      // `x` — backticks dim, content is code.
      push(out, at, at + 1, 'mark');
      push(out, at + 1, at + raw.length - 1, 'code');
      push(out, at + raw.length - 1, at + raw.length, 'mark');
    } else if (g.label !== undefined) {
      // [label](target) — the label is the link face; `](`, target and
      // `)` collapse into one dimmed mark so the overlay keeps the
      // caret-aligned raw characters.
      const href = g.target.startsWith('www.')
        ? `https://${g.target}`
        : g.target;
      push(out, at, at + 1, 'mark');
      push(out, at + 1, at + 1 + g.label.length, 'link', href);
      push(out, at + 1 + g.label.length, at + raw.length, 'mark');
    } else if (g.url !== undefined) {
      const url = trimUrlTail(raw);
      push(
        out,
        at,
        at + url.length,
        'url',
        url.startsWith('www.') ? `https://${url}` : url,
      );
      if (url.length < raw.length) {
        push(out, at + url.length, at + raw.length, 'text');
      }
    } else if (g.mail !== undefined) {
      push(out, at, at + raw.length, 'email', `mailto:${raw}`);
    } else {
      const wide = g.strong !== undefined || g.strike !== undefined;
      const role: TokenRole =
        g.strong !== undefined
          ? 'strong'
          : g.strike !== undefined
            ? 'strike'
            : 'em';
      const d = wide ? 2 : 1;
      push(out, at, at + d, 'mark');
      push(out, at + d, at + raw.length - d, role);
      push(out, at + raw.length - d, at + raw.length, 'mark');
    }
    cursor = at + raw.length;
  }
  if (cursor < text.length) push(out, cursor, text.length, 'text');
  return out;
};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd apps/native && bun test src/lib/richtext.test.ts`
Expected: PASS — all 14 tests.

- [ ] **Step 5: Commit**

```bash
cd apps/native && git add src/lib/richtext.ts src/lib/richtext.test.ts
git commit -m "feat(native): inline richtext tokenizer"
```

---

### Task 2: `RichText` token renderer

**Files:**

- Create: `apps/native/src/components/shared/RichText.tsx`
- Test: `apps/native/src/components/shared/RichText.test.tsx`

**Interfaces:**

- Consumes: `tokenizeInline`, `Token`, `TokenRole` from
  `@/lib/richtext` (Task 1).
- Produces: `RichText({ text, interactive }: { text: string;
  interactive?: boolean })` — Task 3 uses `<RichText text={value} />`
  (highlight mode), Task 4 uses `<RichText text={m.content}
  interactive />`.

No `cn` import — the render is a flat role→class lookup, and keeping
`@marvis/ui` out of the file keeps the bun test hermetic.

- [ ] **Step 1: Write the failing test**

`apps/native/src/components/shared/RichText.test.tsx` (server-render
assertions only — no DOM needed):

```tsx
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
      <RichText text='**b** _i_ `c`' interactive />,
    );
    expect(html).toContain('<strong>b</strong>');
    expect(html).toContain('<em>i</em>');
    expect(html).toContain('<code');
    expect(html).not.toContain('**');
  });

  test('interactive mode renders anchors with resolved hrefs', () => {
    const src = 'www.a.b and me@x.io and [docs](https://a.b)';
    const html = renderToStaticMarkup(<RichText text={src} interactive />);
    expect(html).toContain('href="https://www.a.b"');
    expect(html).toContain('href="mailto:me@x.io"');
    expect(html).toContain('href="https://a.b"');
    expect(html).toContain('>docs</a>');
  });

  test('highlight mode renders no anchors', () => {
    const html = renderToStaticMarkup(<RichText text='https://a.b' />);
    expect(html).not.toContain('<a');
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/native && bun test src/components/shared/RichText.test.tsx`
Expected: FAIL — `Cannot find module './RichText'`.

- [ ] **Step 3: Implement `RichText`**

`apps/native/src/components/shared/RichText.tsx`:

```tsx
import { openUrl } from '@tauri-apps/plugin-opener';
import { useMemo, type ReactNode } from 'react';
import {
  tokenizeInline,
  type Token,
  type TokenRole,
} from '@/lib/richtext';

/**
 * Renders `tokenizeInline` output in two modes:
 *
 * - `highlight` (default) — the Ask input's overlay. Every token is a
 *   span and `mark` delimiters just dim: the characters stay 1:1 with
 *   the real textarea text, so the invisible caret never drifts. Every
 *   class here is metric-safe — no font-weight/family/size changes
 *   (bold is a `text-stroke`, italic a `skewX`), or the overlay would
 *   land at a different x than the caret.
 * - `interactive` — the sent user bubble. Marks drop out entirely and
 *   the roles become semantic elements with real `<a>`s that open via
 *   the opener plugin (the `Markdown.tsx` pattern).
 */

const HL: Record<Exclude<TokenRole, 'text'>, string> = {
  mark: 'text-muted-foreground/60',
  strong: 'font-normal [-webkit-text-stroke:0.45px]',
  em: 'inline-block -skew-x-[8deg]',
  strike: 'line-through decoration-muted-foreground/70',
  code: 'rounded-[3px] bg-fg-soft',
  url: 'text-accent underline decoration-accent/60 underline-offset-2',
  email: 'text-accent underline decoration-accent/60 underline-offset-2',
  link: 'text-accent underline decoration-accent/60 underline-offset-2',
};

const CODE_CHIP =
  'rounded-[3px] bg-fg-soft px-0.5 font-mono text-[0.92em]';

export const RichText = ({
  text,
  interactive = false,
}: {
  text: string;
  /** Sent-bubble mode: marks hidden, links clickable. Default is the
   *  input overlay's highlight mode (marks dimmed, no interactivity). */
  interactive?: boolean;
}) => {
  const nodes = useMemo<ReactNode[]>(
    () =>
      tokenizeInline(text).map((t: Token, i: number) => {
        const slice = text.slice(t.start, t.end);
        if (!interactive) {
          return t.role === 'text' ? (
            slice
          ) : (
            <span
              key={i}
              className={HL[t.role as Exclude<TokenRole, 'text'>]}>
              {slice}
            </span>
          );
        }
        switch (t.role) {
          case 'mark':
            return null;
          case 'strong':
            return <strong key={i}>{slice}</strong>;
          case 'em':
            return <em key={i}>{slice}</em>;
          case 'strike':
            return <s key={i}>{slice}</s>;
          case 'code':
            return (
              <code
                key={i}
                className={CODE_CHIP}>
                {slice}
              </code>
            );
          case 'url':
          case 'email':
          case 'link':
            return (
              <a
                key={i}
                href={t.href}
                onClick={(e) => {
                  e.preventDefault();
                  if (t.href) void openUrl(t.href);
                }}
                className='underline underline-offset-2'>
                {slice}
              </a>
            );
          default:
            return slice;
        }
      }),
    [text, interactive],
  );
  return <>{nodes}</>;
};
```

- [ ] **Step 4: Run tests + typecheck**

Run: `cd apps/native && bun test && bun run check-types`
Expected: all PASS; typecheck clean.

- [ ] **Step 5: Commit**

```bash
cd apps/native && git add src/components/shared/
git commit -m "feat(native): RichText token renderer"
```

---

### Task 3: `AskInput` mirrored highlight overlay

**Files:**

- Modify: `apps/native/src/components/bar/AskInput.tsx`

**Interfaces:**

- Consumes: `RichText` from `@/components/shared/RichText` (Task 2).
- Produces: unchanged public props — `Bar.tsx` needs no edits.

Mechanics: the textarea goes `text-transparent` over an `absolute
inset-0` overlay that renders the styled tokens with identical
metrics; `scrollTop` is synced on `onScroll`; during an IME
composition (`onCompositionStart`/`End`) the textarea briefly shows
real `text-foreground` and the overlay hides, because
`color: transparent` would also hide the CJK marked-text preview.

- [ ] **Step 1: Replace the file**

`apps/native/src/components/bar/AskInput.tsx`:

```tsx
import {
  useRef,
  useState,
  type ChangeEvent,
  type RefObject,
  type SyntheticEvent,
} from 'react';
import { cn } from '@/lib/classes';
import { RichText } from '@/components/shared/RichText';

/** Typography shared verbatim by the textarea and its highlight
 *  overlay — font, size, leading, and padding must stay identical or
 *  the overlay drifts off the (invisible) real caret. */
const METRICS = 'text-[14px] leading-5';

/** The Ask field — a growing textarea shared by the input pill and the
 *  card header, with a mirrored highlight overlay: the textarea's text
 *  is transparent (`caret-accent` still draws the caret) while an
 *  `aria-hidden` div behind it renders link/email/markdown highlighting
 *  from `RichText`. Keeping a real textarea preserves the dictation
 *  tracker's `selectionStart` caret math.
 *
 *  Enter submits; Cmd/Ctrl+Enter submits with a forced screen read
 *  (plain Enter submits normally); Shift+Enter is the textarea's
 *  default newline. An Enter that commits an IME (CJK) composition
 *  never submits — WKWebView dispatches `compositionend` before that
 *  keydown, so `isComposing` is already false and `keyCode === 229` is
 *  the reliable signal (WebKit bug 165004). `onChange`/`onSelect` feed
 *  the dictation tracker. */
export const AskInput = ({
  ref,
  value,
  cardOpen,
  visible,
  onChange,
  onSelect,
  onFocus,
  onSubmit,
}: {
  ref: RefObject<HTMLTextAreaElement | null>;
  value: string;
  /** Card mode raises the line cap — its ResizeObserver reports growth. */
  cardOpen: boolean;
  /** The icon-row ⇄ input-row swap — hidden collapses to `max-w-0` so
   *  the capsule's controls take the width. */
  visible: boolean;
  onChange: (e: ChangeEvent<HTMLTextAreaElement>) => void;
  onSelect: (e: SyntheticEvent<HTMLTextAreaElement>) => void;
  onFocus: () => void;
  onSubmit: (withScreen: boolean) => void;
}) => {
  const overlayRef = useRef<HTMLDivElement>(null);
  /* `color: transparent` would hide the CJK marked-text preview too,
   * so composition briefly restores the real color and hides the
   * overlay — a transient unstyled flash, but the preview stays
   * readable. */
  const [composing, setComposing] = useState(false);
  return (
    <div
      className={cn(
        'relative min-w-0 flex-1 self-center',
        'transition-[max-width_var(--motion-base)_var(--ease),' +
          'opacity_var(--motion-fast)_var(--ease),' +
          'margin-inline_var(--motion-base)_var(--ease)]',
        'motion-reduce:transition-none',
        visible
          ? 'max-w-full'
          : 'pointer-events-none -mx-0.75 max-w-0 opacity-0',
      )}>
      <div
        ref={overlayRef}
        aria-hidden
        className={cn(
          'pointer-events-none absolute inset-0 select-none',
          'overflow-hidden whitespace-pre-wrap wrap-break-word',
          METRICS,
          cardOpen && 'pl-2',
          composing && 'opacity-0',
        )}>
        <RichText text={value} />
      </div>
      <textarea
        ref={ref}
        value={value}
        rows={1}
        onKeyDown={(e) => {
          if (
            e.key === 'Enter' &&
            !e.shiftKey &&
            !e.nativeEvent.isComposing &&
            e.keyCode !== 229
          ) {
            e.preventDefault();
            onSubmit(e.metaKey || e.ctrlKey);
          }
        }}
        onChange={onChange}
        onSelect={onSelect}
        onFocus={onFocus}
        onScroll={(e) => {
          const o = overlayRef.current;
          if (o) o.scrollTop = e.currentTarget.scrollTop;
        }}
        onCompositionStart={() => setComposing(true)}
        onCompositionEnd={() => setComposing(false)}
        placeholder='Ask Marvis…'
        aria-label='Ask Marvis'
        className={cn(
          'field-sizing-content relative w-full resize-none self-center',
          'overflow-y-auto border-0 bg-transparent caret-accent',
          'outline-none select-text placeholder:text-muted-foreground',
          'focus-visible:shadow-none',
          METRICS,
          composing ? 'text-foreground' : 'text-transparent',
          // Line cap: 2 inside the fixed-height pill (scrolls past),
          // ~6 in the card — its ResizeObserver reports growth up.
          cardOpen ? 'max-h-30 pl-2' : 'max-h-10',
        )}
      />
    </div>
  );
};
```

Note the `relative` on the textarea is load-bearing: positioned
elements paint above the earlier `absolute` overlay, keeping the caret
and selection tint on top.

- [ ] **Step 2: Typecheck**

Run: `cd apps/native && bun run check-types`
Expected: clean.

- [ ] **Step 3: Manual smoke — `bun run build:dev`**

Type into the collapsed pill: `check https://marvis.ai and me@x.io`,
then `**bold** _it_`code`~~gone~~`, then a `[docs](https://a.b)`.
Expect: urls/emails accent+underlined, `**`/`_`/`` ` ``/`~~` dimmed
with content stroked-bold / skewed / chipped / struck. Verify: caret
sits correctly inside styled words, placeholder still shows when
empty, Enter still submits, Shift+Enter newline, and — if a CJK IME is
handy — the marked-text preview stays visible while composing. Type
>2 lines and scroll: overlay tracks the textarea's scroll.

- [ ] **Step 4: Commit**

```bash
cd apps/native && git add src/components/bar/AskInput.tsx
git commit -m "feat(native): live highlight in the Ask input"
```

---

### Task 4: Formatted user bubble in `ChatSection`

**Files:**

- Modify: `apps/native/src/components/ChatSection.tsx` (user `<p>`
  around line 352)

**Interfaces:**

- Consumes: `RichText` from `@/components/shared/RichText` (Task 2),
  interactive mode.

- [ ] **Step 1: Import `RichText`**

Add to the import block (component imports are grouped after the
`lib/classes` import):

```tsx
import { ChatMsgMenu, type ChatMsgMeta } from '@/components/ChatMsgMenu';
import { RichText } from '@/components/shared/RichText';
```

- [ ] **Step 2: Render tokens in the user bubble**

Replace the `<p>`'s `{m.content}` child:

```tsx
<p
  className={cn(
    'max-w-[85%] rounded-2xl rounded-br-sm bg-accent/10 px-4 py-2',
    'text-[14px] leading-normal wrap-break-word whitespace-pre-wrap',
    'select-text text-accent',
  )}>
  <RichText text={m.content} interactive />
</p>
```

- [ ] **Step 3: Typecheck + tests**

Run: `cd apps/native && bun run check-types && bun test`
Expected: clean; all tests pass.

- [ ] **Step 4: Manual smoke — `bun run build:dev`**

Send a message containing `https://marvis.ai`, `me@x.io`, `**bold**`,
`_it_`, `` `code` ``, `~~gone~~`, `[docs](https://a.b)`. Expect the
sent bubble to show clickable underlined links (opening in the system
browser), real bold/italic/code/strike, and no visible `**`/`_` marks.
Copy button still copies raw text.

- [ ] **Step 5: Commit**

```bash
cd apps/native && git add src/components/ChatSection.tsx
git commit -m "feat(native): formatted user chat bubbles"
```

---

### Task 5: Full verification pass

**Files:** none — verification only.

- [ ] **Step 1: Full suite + typecheck**

Run: `cd apps/native && bun test && bun run check-types`
Expected: all pass, clean.

- [ ] **Step 2: Pill ↔ card transitions**

Run `bun run build:dev`. Focus the input to open the card, collapse
back to the pill — the `max-w-0` collapse animation still runs on the
wrapper and the overlay doesn't ghost outside it. Resize between
states with styled text present.

- [ ] **Step 3: Dictation regression check**

Start dictation (mic button), dictate a phrase containing "at gmail
dot com" or similar — dictated drafts still insert at the caret and
pick up highlighting. The `useDictation` selection tracker was
untouched, but confirm visually.

- [ ] **Step 4: Done**

Report results; no commit (nothing changed).
