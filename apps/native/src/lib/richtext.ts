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
    '(?<url>https?://[^\\s<>\'"]+|www\\.[^\\s<>\'"]+)',
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
 *  it has no opener inside the match — `x_(y)` keeps its paren. Repeats
 *  until stable: dropping a closer can expose more tail (`a.b,)` → `a.b`). */
const trimUrlTail = (raw: string): string => {
  let s = raw;
  let prev: string;
  do {
    prev = s;
    s = s.replace(TRAILING, '');
    const last = s.slice(-1);
    const open = last === ')' ? '(' : '[';
    if (
      (last === ')' || last === ']') &&
      s.split(last).length - 1 > s.split(open).length - 1
    ) {
      s = s.slice(0, -1);
    }
  } while (prev !== s);
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
      // The trimmed tail was never consumed — leaving cursor short lets
      // it merge into the surrounding text run instead of fragmenting
      // into its own token.
      cursor = at + url.length;
      continue;
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
