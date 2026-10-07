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
