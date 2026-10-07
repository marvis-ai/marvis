import { openUrl } from '@tauri-apps/plugin-opener';
import { useMemo, type ReactNode } from 'react';
import { tokenizeInline, type Token, type TokenRole } from '@/lib/richtext';

/**
 * Renders `tokenizeInline` output in two modes:
 *
 * - `highlight` (default) — the Ask input's overlay. Every token is a
 *   span and `mark` delimiters just dim: the characters stay 1:1 with
 *   the real textarea text, so the invisible caret never drifts. Every
 *   class here is metric-safe — no font-weight/family/size changes
 *   (bold is a `text-stroke`, italic a `skewX`), or the overlay would
 *   land at a different x than the caret. Italic spans additionally
 *   split at whitespace: `inline-block` is atomic, so an unsplit
 *   multi-word `em` couldn't wrap where the textarea beneath can.
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

const CODE_CHIP = 'rounded-[3px] bg-fg-soft px-0.5 font-mono text-[0.92em]';

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
          if (t.role === 'text') return slice;
          // `inline-block` makes a span atomic, so a multi-word em
          // couldn't wrap internally while the textarea beneath can —
          // split at whitespace: words stay skewed spans, whitespace
          // stays wrappable inline text (same advance widths).
          if (t.role === 'em') {
            return slice.split(/(\s)/).map((frag, j) =>
              frag.trim() ? (
                <span
                  key={j}
                  className={HL.em}>
                  {frag}
                </span>
              ) : (
                frag
              ),
            );
          }
          return (
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
