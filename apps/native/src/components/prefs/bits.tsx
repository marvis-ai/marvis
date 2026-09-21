/**
 * Small shared controls for the prefs window — the building blocks the
 * prototype calls `pref-row`, `seg`, `tag`, `kbd`. One accent rule
 * applies throughout: the accent hue only marks live/primary state.
 */
import type { ReactNode } from 'react';
import { kbdTokens } from '../../lib/format';
import { KBD, PR_CTL, PR_LABEL, PR_SUB, PRF_ROW, cn } from '../../lib/classes';

/** One settings row: label + sub on the left, control pinned right. */
export const PrefRow = ({
  label,
  sub,
  children,
  last = false,
}: {
  label: string;
  sub?: ReactNode;
  children?: ReactNode;
  last?: boolean;
}) => (
  <div className={cn(PRF_ROW, last && 'border-b-0')}>
    <div>
      <div className={PR_LABEL}>{label}</div>
      {sub !== undefined && <div className={PR_SUB}>{sub}</div>}
    </div>
    {children !== undefined && <span className={PR_CTL}>{children}</span>}
  </div>
);

/** macOS segmented control — the on-segment gets the surface pill. */
export const Seg = <T extends string>({
  options,
  value,
  onChange,
  ariaLabel,
}: {
  options: { id: T; label: string }[];
  value: T;
  onChange: (id: T) => void;
  ariaLabel?: string;
}) => (
  <span
    className='inline-flex gap-px rounded-lg border border-border bg-fg-soft p-0.5'
    role='group'
    aria-label={ariaLabel}>
    {options.map((o) => (
      <button
        key={o.id}
        type='button'
        className={cn(
          'rounded-md border-0 bg-transparent px-2.5 py-0.75 text-[11.5px] font-[550] transition-[background,color] duration-(--motion-fast) ease-(--ease) motion-reduce:transition-none',
          o.id === value
            ? 'bg-surface text-foreground shadow-[0_1px_2px_color-mix(in_oklch,var(--fg)_20%,transparent)] dark:bg-[color-mix(in_oklch,var(--fg)_14%,var(--surface))]'
            : 'text-muted-foreground hover:text-foreground',
        )}
        onClick={() => onChange(o.id)}>
        {o.label}
      </button>
    ))}
  </span>
);

/** Muted pill — metadata labels (config.toml, marvis://…, MIT). */
export const Tag = ({ children }: { children: ReactNode }) => (
  <span className='inline-flex items-center rounded-full border border-border px-2.25 py-0.75 font-mono text-[11.5px] text-muted-foreground'>
    {children}
  </span>
);

/**
 * macOS toggle — the provider enable switch. `role='switch'` +
 * `aria-checked` carry the state; the visual is the on-track fill
 * (--primary per the segmented/switch rule) + the knob's slide.
 */
export const Switch = ({
  checked,
  onChange,
  disabled = false,
  ariaLabel,
}: {
  checked: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
  ariaLabel: string;
}) => (
  <button
    type='button'
    role='switch'
    aria-checked={checked}
    aria-label={ariaLabel}
    disabled={disabled}
    className={cn(
      'relative h-5 w-8.5 flex-none cursor-pointer rounded-full border p-0 transition-colors duration-(--motion-fast) ease-(--ease) focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2 disabled:cursor-default disabled:opacity-50 motion-reduce:transition-none',
      checked
        ? 'border-primary bg-primary'
        : 'border-border bg-[color-mix(in_oklch,var(--fg)_12%,var(--surface))]',
    )}
    onClick={(e) => {
      e.stopPropagation();
      onChange(!checked);
    }}>
    <span
      className={cn(
        'absolute top-px left-px size-4 rounded-full shadow-[0_1px_2px_color-mix(in_oklch,var(--fg)_40%,transparent)] transition-transform duration-(--motion-fast) ease-(--ease) motion-reduce:transition-none',
        checked ? 'translate-x-3.5 bg-primary-foreground' : 'bg-surface',
      )}
    />
  </button>
);

/** One keycap chip per binding — `Cmd+Shift+S` renders `⌘⇧S`. */
export const Kbd = ({ accel }: { accel: string }) => (
  <span className={KBD}>{kbdTokens(accel).join('')}</span>
);
