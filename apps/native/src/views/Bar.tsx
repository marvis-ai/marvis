/**
 * The 353×47 always-on-top bar (`?view=bar`). The window is transparent,
 * frameless and non-resizable, so every state renders inside the same
 * pill — gate states can't grow the window, and `window_adjust_height`
 * is panel-only (windows/mod.rs).
 *
 * The pill itself morphs (DESIGN.md §6): at rest it's the 104px capsule —
 * iris + camera + mic — and clicking the iris, typing, or entering the
 * permission gate card opens the 345px input bar, which also carries the
 * settings gear (the capsule has no room for a fourth control; the
 * tray's Settings item and `Cmd+,` reach it from any state). All states
 * share one DOM tree so the morph is a class-driven transition, never a
 * remount.
 *
 * The bar is only visible once onboarding has completed — while the
 * wizard is up, the backend keeps this window hidden (windows/mod.rs
 * `sync_bar_visibility`), so no gate card can appear behind it.
 *
 * Errors do NOT render here: the window can't grow, so an error row
 * squeezed the pill's content. They go to the `alert` window instead
 * (`alertShow` → views/AlertToast.tsx).
 *
 * `data-tauri-drag-region` goes on the container chrome and
 * non-interactive children only: Tauri's drag walk treats bare
 * attributes as "direct clicks drag" and clickable descendants
 * (input/button) block it, so inputs and buttons stay usable.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { ArrowLeft, Camera, Mic, Settings, ShieldAlert } from '@marvis/ui';
import { currentMonitor, getCurrentWindow } from '@tauri-apps/api/window';
import {
  alertShow,
  askSend,
  askSendScreenOnly,
  permissionsOpenPrefs,
  permissionsRequestScreen,
  permissionsStatus,
  windowShowSettings,
  type AppStatePayload,
  type Gate,
} from '../lib/commands';
import {
  EV_APP_STATE,
  EV_CAPTURE_PERMISSION_NEEDED,
  useTauriEvent,
} from '../lib/events';
import { Iris } from '../components/Iris';
import { RetryCard } from '../components/RetryCard';

/** Mirrors `app_gate` in lib.rs so the first render doesn't wait on `app:state`. */
const gateFor = (screen: boolean): Gate =>
  screen ? 'main' : 'needs_permission';

/** Every bar error goes to the alert window — the pill has no room. */
const raise = (message: string) => void alertShow(message).catch(() => {});

/** Which screen edge the bar hugs — the breath bobs away from it. */
type Edge = 'top' | 'bottom' | 'left' | 'right';

const edgeFor = async (): Promise<Edge> => {
  try {
    const win = getCurrentWindow();
    const [pos, mon] = await Promise.all([
      win.outerPosition(),
      currentMonitor(),
    ]);
    if (!mon) {
      return 'top';
    }
    // All physical pixels: window is a logical 353×47, so scale up.
    const scale = mon.scaleFactor;
    const cx = pos.x + (353 * scale) / 2;
    const cy = pos.y + (47 * scale) / 2;
    const wa = mon.workArea;
    const dTop = Math.abs(cy - wa.position.y);
    const dBottom = Math.abs(wa.position.y + wa.size.height - cy);
    const dLeft = Math.abs(cx - wa.position.x);
    const dRight = Math.abs(wa.position.x + wa.size.width - cx);
    const min = Math.min(dTop, dBottom, dLeft, dRight);
    if (min === dBottom) return 'bottom';
    if (min === dLeft) return 'left';
    if (min === dRight) return 'right';
    return 'top';
  } catch {
    return 'top';
  }
};

const Bar = () => {
  const [gate, setGate] = useState<Gate | null>(null);
  const [bootError, setBootError] = useState(false);
  const [busy, setBusy] = useState(false);
  const [text, setText] = useState('');
  const [open, setOpen] = useState(false);
  const [edge, setEdge] = useState<Edge>('top');
  const inputRef = useRef<HTMLInputElement>(null);

  // Capsule ⇄ input morph: the gate card and boot errors take the full
  // 345px pill; `main` rests as the capsule until the iris opens it or
  // the user starts typing on the focused window.
  const expanded = bootError
    ? true
    : gate === 'main'
      ? open || text.length > 0
      : gate !== null;

  const bootstrap = useCallback(async () => {
    try {
      const perms = await permissionsStatus();
      setGate(gateFor(perms.screen));
      setBootError(false);
    } catch {
      setBootError(true);
    }
  }, []);

  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);

  useEffect(() => {
    void edgeFor().then(setEdge);
    const unlisten = getCurrentWindow().onMoved(
      () => void edgeFor().then(setEdge),
    );
    return () => void unlisten.then((f) => f());
  }, []);

  useTauriEvent<AppStatePayload>(EV_APP_STATE, (p) => setGate(p.gate));
  // Mid-session screen-permission revocation (ask.rs detects it when a
  // stale frame would have shipped): flip back to the permission card.
  useTauriEvent<{ permission: string }>(EV_CAPTURE_PERMISSION_NEEDED, () =>
    setGate('needs_permission'),
  );

  // Focus the field whenever the pill opens.
  useEffect(() => {
    if (expanded && gate === 'main') {
      inputRef.current?.focus();
    }
  }, [expanded, gate]);

  // Type-to-wake: the collapsed capsule still owns the focused window —
  // a printable keypress opens the field. Esc collapses back.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setText('');
        setOpen(false);
        inputRef.current?.blur();
        return;
      }
      if (gate !== 'main' || open || e.metaKey || e.ctrlKey || e.altKey) {
        return;
      }
      if (e.key.length === 1) {
        // Seed with the waking keypress — it fired before the field
        // could take focus, so it would otherwise be swallowed.
        setText(e.key);
        setOpen(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [gate, open]);

  const collapse = () => {
    setOpen(false);
    inputRef.current?.blur();
  };

  const grantScreen = async () => {
    if (busy) {
      return;
    }
    setBusy(true);
    try {
      await permissionsRequestScreen();
      await bootstrap();
    } catch {
      raise('Permission request failed');
    } finally {
      setBusy(false);
    }
  };

  const submitAsk = (e: FormEvent) => {
    e.preventDefault();
    const t = text.trim();
    if (!t) {
      return;
    }
    setText('');
    collapse();
    void askSend(t).catch(() => raise('Send failed'));
  };

  const pill = `mv-bar${expanded ? ' is-input' : ' is-mini'}`;
  const body = () => {
    if (bootError) {
      return (
        <div
          className='mv-bar-inner justify-center'
          data-tauri-drag-region>
          <RetryCard onRetry={() => void bootstrap()} />
        </div>
      );
    }
    if (gate === 'needs_permission') {
      return (
        <div
          className='mv-bar-inner'
          data-tauri-drag-region>
          <ShieldAlert
            className='mv-gate-ico'
            data-tauri-drag-region
          />
          <span
            className='mv-bar-label'
            title='Marvis needs screen recording to see your screen'
            data-tauri-drag-region>
            Screen recording needed
          </span>
          <button
            type='button'
            className='mv-btn mv-btn-primary'
            onClick={() => void grantScreen()}
            disabled={busy}>
            Grant
          </button>
          <button
            type='button'
            className='mv-btn mv-btn-link'
            onClick={() => void permissionsOpenPrefs('Privacy_ScreenCapture')}>
            Open settings
          </button>
        </div>
      );
    }
    // `main` (and the null boot frame — the same capsule, inert until
    // the gate resolves).
    return (
      <form
        onSubmit={submitAsk}
        className='mv-bar-inner'
        data-tauri-drag-region>
        <button
          type='button'
          className='mv-icon-btn mv-ask'
          aria-label={open ? 'Back to capsule' : 'Ask Marvis'}
          onClick={() => (open ? collapse() : setOpen(true))}
          disabled={gate !== 'main'}>
          <Iris />
          <span className='mv-back'>
            <ArrowLeft />
          </span>
        </button>
        <input
          ref={inputRef}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onFocus={() => gate === 'main' && setOpen(true)}
          placeholder='Ask Marvis…'
          aria-label='Ask Marvis'
          className='mv-input'
        />
        <button
          type='button'
          className='mv-icon-btn'
          aria-label='Ask about the screen'
          title='Ask about the screen'
          disabled={gate !== 'main'}
          onClick={() =>
            void askSendScreenOnly().catch(() => raise('Send failed'))
          }>
          <Camera />
        </button>
        <button
          type='button'
          className='mv-icon-btn mv-mic'
          aria-label='Listen — arrives in Phase 2'
          title='Listen — arrives in Phase 2'
          disabled>
          <Mic />
        </button>
        {/* Collapses to nothing in the capsule (no room in 104px) — the
            tray's Settings item and Cmd+, reach it from any state. */}
        <button
          type='button'
          className='mv-icon-btn mv-gear'
          aria-label='Settings'
          title='Settings (⌘,)'
          tabIndex={expanded ? 0 : -1}
          disabled={gate !== 'main'}
          onClick={() => void windowShowSettings().catch(() => {})}>
          <Settings />
        </button>
      </form>
    );
  };

  return (
    <div
      className='mv-stage'
      data-pos={edge}
      data-tauri-drag-region>
      <div
        className={pill}
        data-tauri-drag-region='deep'>
        {body()}
      </div>
    </div>
  );
};

export default Bar;
