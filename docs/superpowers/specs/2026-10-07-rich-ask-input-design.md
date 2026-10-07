# Rich Ask Input + Formatted User Bubbles — Design

Date: 2026-10-07
Branch: `feature/tweak_input_of_chatbox`
Scope: `apps/native` webview only — no Rust, no IPC changes.

## Context

- `AskInput` (`src/components/bar/AskInput.tsx`) is a plain `<textarea>`
  shared by the input pill and the card composer. `useDictation`
  (`src/hooks/useDictation.ts`) tracks caret/ranges through
  `selectionStart`/`selectionEnd`, and the component deliberately handles
  CJK IME Enter (`keyCode === 229` — WKWebView bug 165004).
- Assistant replies already render markdown via the lazy `Markdown`
  component (react-markdown + remark-gfm, links through
  `@tauri-apps/plugin-opener`'s `openUrl`).
- User bubbles (`ChatSection.tsx`) render raw `whitespace-pre-wrap` text —
  no links, no inline formatting.

Goal: while typing, the input auto-detects links, emails, and inline
markdown marks (`**`, `_`, `` ` ``, `~~`, `[t](u)`) and shows them styled;
once sent, the user bubble renders the same formatting with clickable
links.

## Decision: overlay highlight, not contenteditable

Two viable approaches:

| Approach | Verdict |
| --- | --- |
| Transparent `<textarea>` over a mirrored highlight `<div>` (react-simple-code-editor technique) | Chosen — the real textarea keeps dictation caret math, `selectionStart`-based tracking, IME Enter handling, and `field-sizing-content` growth untouched. Every character stays 1:1 between text and overlay, so caret alignment is exact. |
| WYSIWYG `contenteditable` (Lexical/Tiptap), marks hidden | Rejected — rewrites the selection/dictation contract, risks the CJK composition edge cases, heavy dependency for a chat pill. |

The same tokenizer drives both the input overlay and the sent user
bubble, so what you see while typing is what the bubble renders.

## Components

### 1. `src/lib/richtext.ts` (new) — `tokenizeInline(text): Token[]`

Pure function, unit-tested alongside `dictation.test.ts` via `bun test`.

```ts
export type TokenRole =
  | 'text' | 'mark' // dimmed punctuation: ** _ ` ~~ [ ]( )
  | 'strong' | 'em' | 'strike' | 'code'
  | 'url' | 'email' | 'link'; // link = [label](url) label; href carries target

export interface Token {
  start: number;
  end: number;
  role: TokenRole;
  /** Set on url/email/link tokens — email hrefs are `mailto:`-prefixed. */
  href?: string;
}
```

Inline grammar, matched leftmost with this precedence (single combined
regex alternation or equivalent ordered scan; matches never overlap):

1. `` `code` `` — masks everything inside (`` `**x**` `` stays literal code)
2. `[label](url)` — emits `mark` `[`, `link` label, `mark` `](` + `mark`
   `)` with the url span as a `mark` token carrying no style of its own;
   the `link` token's `href` is the url
3. autolinks `https?://…` and `www.…` — need a non-word left boundary
   (`xhttps://a.b` stays literal); trailing `.,!?;:'"`, emphasis marks
   (`*_~`), and unbalanced `)`/`]` are stripped; `www.` tokens display
   verbatim but get `href = https://` + match so `openUrl` receives a
   valid URL
4. emails `user@host.tld` — `href = mailto:…`
5. `**bold**` / `__bold__`, `*italic*` / `_italic_`, `~~strike~~` — marks
   emit as `mark` tokens, inner content as `strong`/`em`/`strike`

Rules: no nesting; unmatched delimiters render literal; `_`/`__` require
a non-word boundary so `foo_bar_baz` stays plain; url/email ranges
already consumed by `[t](u)` or code are not re-matched.

### 2. `src/components/shared/RichText.tsx` (new) — token renderer

Named arrow export, one component, `interactive?: boolean` prop:

- default (input overlay, `interactive` unset): every token is a
  `<span>`; `mark` → `text-muted-foreground/60`, `strong` →
  `-webkit-text-stroke` fake-bold, `em` → `-skew-x-[8deg]` fragments
  split at whitespace, `strike` → `line-through`, `code` → `bg-fg-soft`
  chip with no font change, `url`/`email`/`link` → `text-accent
  underline`. Every class is metric-safe — no font-weight/family/size
  shifts — so each character keeps the textarea's advance width and
  the overlay stays aligned with the invisible caret. `em` splits at
  whitespace because `inline-block` is atomic: an unsplit multi-word
  em couldn't wrap where the textarea beneath can.
- `interactive` (user bubble): marks are dropped entirely;
  `strong`/`em`/`strike`/`code` become semantic elements; `url`/`email`/
  `link` become `<a>` whose click calls `openUrl(href)` (the
  `Markdown.tsx` pattern) with `e.preventDefault()`

### 3. `src/components/bar/AskInput.tsx` — mirrored overlay

- New wrapper `<div className='relative min-w-0 flex-1 self-center'>`;
  the visibility-collapse classes (`max-w-full`/`max-w-0`, `-mx-0.75`,
  `opacity-0`, `pointer-events-none`, the width/opacity transition) move
  from the textarea to this wrapper
- Overlay `<div aria-hidden>`: `absolute inset-0 overflow-hidden
  pointer-events-none select-none whitespace-pre-wrap wrap-break-word`,
  sharing a `METRICS` class constant with the textarea (`text-[14px]
  leading-5`, plus `pl-2` when `cardOpen`) so wrapping matches; renders
  `<RichText text={value} />` (tokenizer output `useMemo`-cached inside)
- Textarea: gains `w-full text-transparent`, keeps `caret-accent`,
  `field-sizing-content`, all existing handlers; loses `flex-1 min-w-0
  self-center` (wrapper owns them)
- Scroll sync: `onScroll` → `overlay.scrollTop = textarea.scrollTop`
  (`overflow:hidden` boxes scroll programmatically)
- IME caveat: `color: transparent` also hides the CJK composition
  preview, so a `composing` state (`onCompositionStart`/`End`) swaps the
  textarea back to `text-foreground` and hides the overlay for the
  composition's duration — transient unstyled text, correct behavior

### 4. `src/components/ChatSection.tsx` — user bubble

The user `<p>` keeps its bubble classes; `{m.content}` becomes
`<RichText text={m.content} interactive />`. Inline-only — no
block markdown (headers/lists), so the bubble stays compact and
right-aligned. Links open in the system browser via `openUrl`.

## Testing & verification

- `src/lib/richtext.test.ts`: each grammar element; precedence (`code`
  masking marks and urls); unmatched/nested delimiters literal;
  `foo_bar_baz` not emphasized; `[t](u)` url not double-linkified;
  trailing punctuation trimmed off autolinks; email vs `www.` disjoint
- `bun test` and `bun run check-types` in `apps/native`
- Manual via `bun run build:dev`: type URL/email/marks in pill and card;
  CJK IME composition stays visible; pill↔card collapse animates
  cleanly; overflow scroll stays aligned; dictation inserts still
  highlight; sent bubble shows clickable links

## Out of scope

- Block-level markdown (headers, lists, quotes) in input or user bubble
- Changes to assistant bubbles (already full markdown)
- True WYSIWYG hidden-mark editing inside the input
- Link unfurling / previews, autocomplete, mentions
