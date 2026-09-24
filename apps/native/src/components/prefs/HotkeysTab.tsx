/**
 * Hotkeys — the one rebindable action (`hotkeys.toggle_input` in
 * config.toml) plus the fixed keys the bar webview owns. Click the
 * global binding to rebind it: the row arms, the next chord becomes
 * the accelerator (a modifier is required — a bare key would hijack
 * normal typing), Esc cancels. Writes go through `config_set`, which
 * delta-swaps the registered set.
 */
import { useEffect, useState } from 'react';
import { configSet } from '@/lib/commands';
import {
  H2,
  PR_LABEL,
  PR_SUB,
  PRF_ROW,
  PRF_ROWS,
  SUB,
  cn,
} from '@/lib/classes';
import { Kbd } from './bits';
import type { PrefsData } from './types';

const ACTIONS: { id: string; label: string }[] = [
  { id: 'toggle_input', label: 'Show / hide the input' },
];

/** The bar webview's fixed bindings — displayed, never rebindable. */
const FIXED: { label: string; sub: string; accels: string[] }[] = [
  {
    label: 'Settings',
    sub: 'Only while the bar is active',
    accels: ['Cmd+,'],
  },
  {
    label: 'Send · new line',
    sub: 'Enter sends, Shift+Enter adds a line',
    accels: ['Enter', 'Shift+Enter'],
  },
  {
    label: 'Send with screenshot',
    sub: 'At the input',
    accels: ['Cmd+Enter'],
  },
];

/** Keys that are a modifier being held, not a bindable key press. */
const HELD_MODIFIERS = new Set([
  'Meta',
  'Control',
  'Alt',
  'Shift',
  'CapsLock',
  'Fn',
]);

/** `KeyboardEvent.key` → the accelerator token `parse_accelerator`
 * accepts. Single letters/digits and the spec's symbol set pass
 * verbatim (letters uppercased); arrows and friends get named. */
const keyToken = (key: string): string | null => {
  const named: Record<string, string> = {
    Enter: 'Enter',
    Tab: 'Tab',
    Backspace: 'Backspace',
    Delete: 'Delete',
    ArrowUp: 'Up',
    ArrowDown: 'Down',
    ArrowLeft: 'Left',
    ArrowRight: 'Right',
    ' ': 'Space',
  };
  if (named[key]) {
    return named[key];
  }
  if (key.length === 1) {
    return /[a-z]/.test(key) ? key.toUpperCase() : key;
  }
  return /^F\d{1,2}$/.test(key) ? key : null;
};

export const HotkeysTab = ({ data }: { data: PrefsData }) => {
  const hk = data.config?.hotkeys ?? {};
  /** The action id currently listening for a chord. */
  const [listening, setListening] = useState<string | null>(null);
  const [hint, setHint] = useState<string | null>(null);

  useEffect(() => {
    if (!listening) {
      return;
    }
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') {
        setListening(null);
        setHint(null);
        return;
      }
      if (HELD_MODIFIERS.has(e.key)) {
        return; // still holding modifiers — keep listening
      }
      const mods = [
        e.metaKey && 'Cmd',
        e.ctrlKey && 'Ctrl',
        e.altKey && 'Alt',
        e.shiftKey && 'Shift',
      ].filter(Boolean) as string[];
      if (mods.length === 0) {
        setHint('Add ⌘, ⌃ or ⌥ — a bare key would fire while typing');
        return;
      }
      const token = keyToken(e.key);
      if (!token) {
        return;
      }
      const accel = [...mods, token].join('+');
      if (accel === hk[listening]) {
        // The existing chord — keep it. The binding stays registered,
        // so this press fired the real action through the OS anyway.
        setListening(null);
        setHint(null);
        return;
      }
      setListening(null);
      setHint(null);
      void configSet(`hotkeys.${listening}`, accel)
        .then(data.setConfig)
        .catch(() => setHint("That chord doesn't parse — try another"));
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
    // `listening`/`hk` are read fresh per render's closure — the listener
    // re-arms whenever the armed action or the bindings change.
  }, [listening, hk]);

  const rows = ACTIONS.length + FIXED.length;
  let rowIndex = -1;

  return (
    <>
      <h2 className={H2}>Hotkeys</h2>
      <p className={SUB}>
        Show / hide the input is the only global chord — click its binding, then
        press the new one; Esc cancels. The rest are fixed keys inside the bar.
      </p>

      <div className={PRF_ROWS}>
        {ACTIONS.map((a) => {
          rowIndex += 1;
          const accel = hk[a.id];
          const armed = listening === a.id;
          return (
            <div
              key={a.id}
              className={cn(PRF_ROW, rowIndex === rows - 1 && 'border-b-0')}>
              <div>
                <div className={PR_LABEL}>{a.label}</div>
                {armed && (
                  <div className={PR_SUB}>
                    {hint ?? 'press a shortcut with ⌘, ⌃ or ⌥'}
                  </div>
                )}
              </div>
              <button
                type='button'
                className={cn(
                  'cursor-pointer rounded-md border-0 bg-transparent p-0',
                  armed && 'shadow-(--focus-ring)',
                )}
                title='Click to rebind'
                onClick={() => {
                  setListening(armed ? null : a.id);
                  setHint(null);
                }}>
                <Kbd accel={accel ?? ''} />
              </button>
            </div>
          );
        })}
        {FIXED.map((f) => {
          rowIndex += 1;
          return (
            <div
              key={f.label}
              className={cn(PRF_ROW, rowIndex === rows - 1 && 'border-b-0')}>
              <div>
                <div className={PR_LABEL}>{f.label}</div>
                <div className={PR_SUB}>{f.sub}</div>
              </div>
              <span className='flex items-center gap-1.5'>
                {f.accels.map((accel) => (
                  <Kbd
                    key={accel}
                    accel={accel}
                  />
                ))}
              </span>
            </div>
          );
        })}
      </div>
    </>
  );
};
