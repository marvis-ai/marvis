import {
  useRef,
  useState,
  type ChangeEvent,
  type RefObject,
  type SyntheticEvent,
} from 'react';
import { cn } from '@/lib/classes';
import { PALETTE_KEYS } from '@/lib/presets';
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
 *  the dictation tracker. `onDisarm` fires on Backspace/Delete at a
 *  collapsed caret-0 — the armed preset badges' remove gesture; it
 *  returns whether it disarmed (a `true` swallows the key so forward
 *  Delete can't eat the first character too). While `paletteOpen`,
 *  the nav/pick/dismiss keys forward to the unfocused preset palette
 *  via `onPaletteKey` (its `/` session leaves this field focused). */
export const AskInput = ({
  ref,
  value,
  cardOpen,
  visible,
  paletteOpen,
  onPaletteKey,
  onChange,
  onSelect,
  onFocus,
  onSubmit,
  onDisarm,
  dropActive,
}: {
  ref: RefObject<HTMLTextAreaElement | null>;
  value: string;
  /** Card mode raises the line cap — its ResizeObserver reports growth. */
  cardOpen: boolean;
  /** The icon-row ⇄ input-row swap — hidden collapses to `max-w-0` so
   *  the capsule's controls take the width. */
  visible: boolean;
  /** The preset palette is up — nav/pick/dismiss keys are its. */
  paletteOpen: boolean;
  onPaletteKey: (key: string) => void;
  onChange: (e: ChangeEvent<HTMLTextAreaElement>) => void;
  onSelect: (e: SyntheticEvent<HTMLTextAreaElement>) => void;
  onFocus: () => void;
  onSubmit: (withScreen: boolean) => void;
  onDisarm: () => boolean;
  /** A file drag hovers the form — ring the field as the drop target. */
  dropActive?: boolean;
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
        'relative flex min-w-0 flex-1 items-center self-center',
        'transition-[max-width_var(--motion-base)_var(--ease),' +
          'opacity_var(--motion-fast)_var(--ease),' +
          'margin-inline_var(--motion-base)_var(--ease)]',
        'motion-reduce:transition-none',
        visible
          ? 'max-w-full'
          : 'pointer-events-none -mx-0.75 max-w-0 opacity-0',
        dropActive && 'rounded-lg shadow-(--focus-ring)',
      )}>
      <div className='relative min-w-0 flex-1'>
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
            const composing = e.nativeEvent.isComposing || e.keyCode === 229;
            if (paletteOpen && !composing && PALETTE_KEYS.includes(e.key)) {
              // The palette owns this key set — forward it AND consume
              // the event: preventDefault alone still bubbles to the
              // window keydown, where Esc would collapse the bar.
              e.preventDefault();
              e.stopPropagation();
              onPaletteKey(e.key);
              return;
            }
            if (e.key === 'Enter' && !e.shiftKey && !composing) {
              e.preventDefault();
              onSubmit(e.metaKey || e.ctrlKey);
            } else if (
              (e.key === 'Backspace' || e.key === 'Delete') &&
              !composing &&
              e.currentTarget.selectionStart === 0 &&
              e.currentTarget.selectionEnd === 0 &&
              onDisarm()
            ) {
              e.preventDefault();
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
            // `block` — an inline textarea sits on the wrapper's anonymous
            // line-box baseline, leaving a strut-descent strip below it
            // that pushes the text off the pill's vertical center.
            'field-sizing-content relative block w-full resize-none',
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
    </div>
  );
};
