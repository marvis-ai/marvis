/**
 * Small shared controls for the prefs window — the building blocks the
 * prototype calls `pref-row`, `seg`, `tag`, `kbd`. One accent rule
 * applies throughout: the accent hue only marks live/primary state.
 */
import type { ReactNode } from 'react';
import { kbdTokens } from '../../lib/format';

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
  <div
    className='prf-row'
    style={last ? { borderBottom: 0 } : undefined}>
    <div>
      <div className='pr-label'>{label}</div>
      {sub !== undefined && <div className='pr-sub'>{sub}</div>}
    </div>
    {children !== undefined && <span className='pr-ctl'>{children}</span>}
  </div>
);

/** macOS segmented control — `is-on` gets the surface pill. */
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
    className='prf-seg'
    role='group'
    aria-label={ariaLabel}>
    {options.map((o) => (
      <button
        key={o.id}
        type='button'
        className={o.id === value ? 'is-on' : ''}
        onClick={() => onChange(o.id)}>
        {o.label}
      </button>
    ))}
  </span>
);

/** Muted pill — metadata labels (config.toml, marvis://…, MIT). */
export const Tag = ({ children }: { children: ReactNode }) => (
  <span className='prf-tag'>{children}</span>
);

/**
 * macOS toggle — the provider enable switch. `role='switch'` +
 * `aria-checked` carry the state; the visual is the `is-on` track fill
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
    className={`prf-switch${checked ? ' is-on' : ''}`}
    onClick={(e) => {
      e.stopPropagation();
      onChange(!checked);
    }}>
    <span className='prf-switch-knob' />
  </button>
);

/** One keycap chip per binding — `Cmd+Shift+S` renders `⌘⇧S`. */
export const Kbd = ({ accel }: { accel: string }) => (
  <span className='kbd'>{kbdTokens(accel).join('')}</span>
);
