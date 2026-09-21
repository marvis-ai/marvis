/**
 * Hotkeys — the four configurable actions (`hotkeys.*` in config.toml).
 * Click a binding to rebind it: the row arms, the next chord becomes the
 * accelerator (a modifier is required — a bare key would hijack normal
 * typing), Esc cancels. Writes go through `config_set`, which
 * re-registers the live set; a chord already taken by another action is
 * refused here so it can't silently win the backend's first-wins dedup.
 */
import { useEffect, useState } from 'react';
import { configSet } from '../../lib/commands';
import {
  H2,
  META,
  PR_LABEL,
  PR_SUB,
  PRF_ROW,
  PRF_ROWS,
  SUB,
  cn,
} from '../../lib/classes';
import { Kbd } from './bits';
import type { PrefsData } from './types';

const ACTIONS: { id: string; label: string }[] = [
  { id: 'toggle_visibility', label: 'Show / hide everything' },
  { id: 'next_step', label: 'Send ask' },
  { id: 'screen_only', label: 'Screenshot → ask' },
  { id: 'show_settings', label: 'Settings' },
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
      const clash = ACTIONS.find(
        (a) => a.id !== listening && hk[a.id] === accel,
      );
      if (clash) {
        setHint(`Already bound to “${clash.label}”`);
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

  return (
    <>
      <h2 className={H2}>Hotkeys</h2>
      <p className={SUB}>
        Global, registered at the OS level. Click a binding, then press the new
        chord — Esc cancels.
      </p>

      <div className={PRF_ROWS}>
        {ACTIONS.map((a, i) => {
          const accel = hk[a.id];
          const armed = listening === a.id;
          return (
            <div
              key={a.id}
              className={cn(PRF_ROW, i === ACTIONS.length - 1 && 'border-b-0')}>
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
      </div>
      <p className={cn(META, 'mt-3.5')}>
        before setup finishes, only Show/hide and Settings respond.
      </p>
    </>
  );
};
