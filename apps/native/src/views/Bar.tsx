/**
 * The always-on-top bar (`?view=bar`) — a 64px-tall transparent,
 * frameless window whose width morphs: 112 idle, 480 expanded. The
 * capsule IS the window (liquid glass fills the frame), so the morph
 * is a real window resize — `expanded` reports to
 * `window_set_bar_expanded` and Rust animates/recenter/clamps.
 *
 * Content swaps inside it (DESIGN.md §6): at rest it's the centered
 * 3-icon row — iris + camera + mic — and clicking the iris or typing
 * swaps in the input row (iris morphs to a back arrow, the field
 * unfolds, the settings gear appears); gate states replace the row
 * entirely. All states share one DOM tree so the swap is a
 * class-driven transition, never a remount.
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
import type { SubmitEvent } from 'react';
import {
  ArrowLeftIcon,
  CameraIcon,
  GripVerticalIcon,
  MicIcon,
  SettingsIcon,
  ShieldAlertIcon,
  ShineBorder,
} from '@marvis/ui';
import {
  alertShow,
  askSend,
  askSendScreenOnly,
  permissionsOpenPrefs,
  permissionsRequestScreen,
  permissionsStatus,
  windowSetBarExpanded,
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
import {
  BTN_LINK,
  BTN_LINK_SM,
  BTN_PRIMARY,
  BTN_SM,
  ICON_BTN,
  cn,
} from '../lib/classes';

/** Mirrors `app_gate` in lib.rs so the first render doesn't wait on `app:state`. */
const gateFor = (screen: boolean): Gate =>
  screen ? 'main' : 'needs_permission';

/** Every bar error goes to the alert window — the pill has no room. */
const raise = (message: string) => void alertShow(message).catch(() => {});

/**
 * The drag handle — revealed on hover at the pill's front edge. It's a
 * bare span because Tauri refuses drags that start on interactive
 * elements (button/input/a): the capsule is wall-to-wall buttons, so
 * this is the only honest drag target it gets.
 */
/** Bar controls step up from the 26px overlay default (`ICON_BTN`) —
 *  the capsule is 64px tall, so buttons/icons scale ~1.3×; `fg-2` reads
 *  better than `muted` on glass. */
const BAR_BTN = cn(ICON_BTN, 'size-8.5 text-fg-2');

const grip = (
  <span
    className='-mx-0.75 grid w-4 max-w-0 flex-none cursor-grab place-items-center overflow-hidden text-muted-foreground opacity-0 transition-[max-width_var(--motion-base)_var(--ease),opacity_var(--motion-fast)_var(--ease),margin-inline_var(--motion-base)_var(--ease)] group-hover/bar:mx-0 group-hover/bar:max-w-4 group-hover/bar:opacity-100 active:cursor-grabbing motion-reduce:transition-none'
    data-tauri-drag-region='deep'
    title='Drag'
    aria-hidden='true'>
    <GripVerticalIcon className='size-3.75' />
  </span>
);

const Bar = () => {
  const [gate, setGate] = useState<Gate | null>(null);
  const [bootError, setBootError] = useState(false);
  const [busy, setBusy] = useState(false);
  const [text, setText] = useState('');
  const [open, setOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  // Icon-row ⇄ input-row swap: the gate card and boot errors count as
  // expanded; `main` rests as the icon row until the iris opens it or
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

  useTauriEvent<AppStatePayload>(EV_APP_STATE, (p) => setGate(p.gate));
  // Mid-session screen-permission revocation (ask.rs detects it when a
  // stale frame would have shipped): flip back to the permission card.
  useTauriEvent<{ permission: string }>(EV_CAPTURE_PERMISSION_NEEDED, () =>
    setGate('needs_permission'),
  );

  // The capsule IS the window under liquid glass, so the morph resizes
  // it: idle rests at 112 (the 3-icon row), expanded grows to 480 —
  // recentred on center-x by the backend.
  useEffect(() => {
    void windowSetBarExpanded(expanded).catch(() => {});
  }, [expanded]);

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

  const submitAsk = (e: SubmitEvent<HTMLFormElement>) => {
    e.preventDefault();
    const t = text.trim();
    if (!t) {
      return;
    }
    setText('');
    collapse();
    void askSend(t).catch(() => raise('Send failed'));
  };

  const inner = cn(
    'flex min-h-0 flex-1 items-center gap-1.5',
    expanded ? 'px-2.75' : 'justify-center px-1.75',
  );
  const pill = cn(
    'group/bar glass-surface relative flex h-full w-full flex-none flex-col justify-center rounded-full border border-border bg-[color-mix(in_oklch,var(--surface)_80%,transparent)] backdrop-blur-[14px] select-none transition-[border-color,box-shadow] duration-(--motion-base) ease-(--ease) motion-reduce:transition-none',
  );
  const body = () => {
    if (bootError) {
      return (
        <div
          className={cn(inner, 'justify-center')}
          data-tauri-drag-region>
          {grip}
          <RetryCard onRetry={() => void bootstrap()} />
        </div>
      );
    }
    if (gate === 'needs_permission') {
      return (
        <div
          className={inner}
          data-tauri-drag-region>
          {grip}
          <ShieldAlertIcon
            className='size-4.5 flex-none text-muted-foreground'
            data-tauri-drag-region
          />
          <span
            className='min-w-0 flex-1 truncate text-xs text-muted-foreground'
            title='Marvis needs screen recording to see your screen'
            data-tauri-drag-region>
            Screen recording needed
          </span>
          <button
            type='button'
            className={cn(BTN_SM, BTN_PRIMARY)}
            onClick={() => void grantScreen()}
            disabled={busy}>
            Grant
          </button>
          <button
            type='button'
            className={cn(BTN_LINK_SM, BTN_LINK)}
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
        className={inner}
        data-tauri-drag-region>
        {grip}
        <button
          type='button'
          className={cn(BAR_BTN, 'relative')}
          aria-label={open ? 'Back to capsule' : 'Ask Marvis'}
          onClick={() => (open ? collapse() : setOpen(true))}
          disabled={gate !== 'main'}>
          <Iris />
          <span className='pointer-events-none absolute inset-0 grid -rotate-90 scale-[0.4] place-items-center opacity-0 transition-[rotate_var(--motion-base)_var(--ease)_55ms,scale_var(--motion-base)_var(--ease)_55ms,opacity_var(--motion-fast)_var(--ease)_55ms] group-data-expanded/bar:rotate-none group-data-expanded/bar:scale-100 group-data-expanded/bar:opacity-100 motion-reduce:transition-none'>
            <ArrowLeftIcon className='size-5.5' />
          </span>
        </button>
        <input
          ref={inputRef}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onFocus={() => gate === 'main' && setOpen(true)}
          placeholder='Ask Marvis…'
          aria-label='Ask Marvis'
          className={cn(
            'min-w-0 flex-1 self-stretch border-0 bg-transparent text-[13.5px] text-foreground caret-accent outline-none select-text placeholder:text-muted-foreground focus-visible:shadow-none transition-[max-width_var(--motion-base)_var(--ease),opacity_var(--motion-fast)_var(--ease),margin-inline_var(--motion-base)_var(--ease)] motion-reduce:transition-none',
            expanded
              ? 'max-w-80'
              : 'pointer-events-none -mx-1.5 max-w-0 opacity-0',
          )}
        />
        <button
          type='button'
          className={BAR_BTN}
          aria-label='Ask about the screen'
          title='Ask about the screen'
          disabled={gate !== 'main'}
          onClick={() =>
            void askSendScreenOnly().catch(() => raise('Send failed'))
          }>
          <CameraIcon className='size-5' />
        </button>
        <button
          type='button'
          className={BAR_BTN}
          aria-label='Listen — arrives in Phase 2'
          title='Listen — arrives in Phase 2'
          disabled>
          <MicIcon className='size-5' />
        </button>
        {/* Only rendered in the input bar — the idle capsule has no
            room for a fourth control (tray menu + Cmd+, reach it
            anyway). */}
        {expanded && (
          <button
            type='button'
            className={BAR_BTN}
            aria-label='Settings'
            title='Settings (⌘,)'
            disabled={gate !== 'main'}
            onClick={() => void windowShowSettings().catch(() => {})}>
            <SettingsIcon className='size-5' />
          </button>
        )}
      </form>
    );
  };

  return (
    <div
      className='group/stage glass-stage flex h-full flex-col items-center justify-center p-1'
      data-tauri-drag-region>
      <div
        className={pill}
        data-expanded={expanded || undefined}
        data-tauri-drag-region='deep'>
        {body()}
        {/* Capsule shimmer — accent duotone follows light/dark via the
            tokens; masked to the border ring, pointer-events-none. */}
        <ShineBorder shineColor={['var(--accent)', 'var(--accent-text)']} />
      </div>
    </div>
  );
};

export default Bar;
