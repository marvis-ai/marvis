import type { ChangeEvent, RefObject, SyntheticEvent } from 'react';
import { cn } from '@/lib/classes';

/** The Ask field — a growing textarea shared by the input pill and the
 *  card header. Enter submits; Cmd/Ctrl+Enter submits with a forced
 *  screen read (plain Enter submits normally); Shift+Enter is the
 *  textarea's default newline. `onChange`/`onSelect` feed the dictation
 *  tracker. */
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
}) => (
  <textarea
    ref={ref}
    value={value}
    rows={1}
    onKeyDown={(e) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        onSubmit(e.metaKey || e.ctrlKey);
      }
    }}
    onChange={onChange}
    onSelect={onSelect}
    onFocus={onFocus}
    placeholder='Ask Marvis…'
    aria-label='Ask Marvis'
    className={cn(
      'field-sizing-content min-w-0 flex-1 resize-none self-center overflow-y-auto border-0 bg-transparent text-[13.5px] leading-5 text-foreground caret-accent outline-none select-text placeholder:text-muted-foreground focus-visible:shadow-none transition-[max-width_var(--motion-base)_var(--ease),opacity_var(--motion-fast)_var(--ease),margin-inline_var(--motion-base)_var(--ease)] motion-reduce:transition-none',
      // Line cap: 2 inside the fixed-height pill (scrolls past),
      // ~6 in the card — its ResizeObserver reports growth up.
      cardOpen ? 'max-h-30' : 'max-h-10',
      visible ? 'max-w-full' : 'pointer-events-none -mx-0.75 max-w-0 opacity-0',
    )}
  />
);
