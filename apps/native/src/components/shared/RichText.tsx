import { openUrl } from '@tauri-apps/plugin-opener';
import { useMemo, type ReactNode } from 'react';
import { tokenizeInline, type Token, type TokenRole } from '@/lib/richtext';

/**
 * Renders `tokenizeInline` output in two modes:
 *
 * - `highlight` (default) — the Ask input's overlay. Every token is a
 *   span and `mark` delimiters just dim: the characters stay 1:1 with
 *   the real textarea text, so the invisible caret never drifts. Every
 *   class here preserves the text size and family. Bold uses a text
 *   stroke; emphasis uses an inline oblique style so even long words
 *   can wrap inside the span. Outfit has no italic face, so the browser
 *   synthesizes the slant without changing glyph advance widths.
 * - `interactive` — the sent user bubble. Marks drop out entirely and
 *   the roles become semantic elements with real `<a>`s that open via
 *   the opener plugin (the `Markdown.tsx` pattern).
 */

const HL: Record<Exclude<TokenRole, 'text'>, string> = {
  mark: 'text-muted-foreground/60',
  strong: 'font-normal [-webkit-text-stroke:0.45px]',
  em: '[font-style:oblique_8deg]',
  strike: 'line-through decoration-muted-foreground/70',
  code: 'rounded-[3px] bg-fg-soft',
  url: 'text-accent underline decoration-accent/60 underline-offset-2',
  email: 'text-accent underline decoration-accent/60 underline-offset-2',
  link: 'text-accent underline decoration-accent/60 underline-offset-2',
};

const CODE_CHIP = 'rounded-[3px] bg-fg-soft px-0.5 font-mono text-[0.92em]';

/** Only `http(s)`/`mailto:` hrefs reach the DOM — `tokenizeInline`
 *  already whitelists schemes at emit time, and this sink-side check
 *  keeps the invariant if a new href producer is ever added (a
 *  `javascript:`/`data:` URL in an anchor href is a stored-XSS sink). */
const safeHref = (href?: string): string | undefined =>
  href !== undefined && /^(?:https?:|mailto:)/i.test(href) ? href : undefined;

/** Render inline rich text, preserving all characters for input highlighting.
 *  `interactive` hides parsed marks and opens links through the Tauri opener
 *  on click; opener failures are not caught or displayed here. */
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
          case 'link': {
            const href = safeHref(t.href);
            return (
              <a
                key={i}
                href={href}
                onClick={(e) => {
                  e.preventDefault();
                  if (href) void openUrl(href);
                }}
                className='underline underline-offset-2'>
                {slice}
              </a>
            );
          }
          default:
            return slice;
        }
      }),
    [text, interactive],
  );
  return <>{nodes}</>;
};
