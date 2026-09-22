/**
 * The always-on-top bar (`?view=bar`) — the UNIFIED window. Two shapes:
 *
 *  - Pill modes (mini | input | permission): the 136⇄480×64 capsule⇄
 *    input morph — the capsule IS the window under liquid glass, so
 *    `expanded` reports to `window_set_bar_expanded` and Rust animates
 *    the width change. Unchanged mechanics.
 *  - Card modes (chat | listen): the same window grown to
 *    600×(64+content) — the bar row becomes the card's header, pinned
 *    to the anchored edge (`flex-col-reverse` when growing up puts the
 *    row at the bottom and it never visually jumps).
 *
 * `cardOpen` is read off the window itself: `innerHeight > BAR_H` means
 * Rust expanded us — `set_chat_open` emits nothing by design, so the
 * `resize` event is the single open/close signal for every path
 * (Cmd+/, ask send, `ask_close`, `window_set_chat_open`).
 *
 * `growDir` is detected at expand time: the anchored edge is fixed for
 * grow-down and rises for grow-up, so the first expanded y-read compared
 * to the last collapsed y gives the direction. While collapsed the
 * baseline refreshes on every `tauri://move`/`resize` tick.
 *
 * `data-tauri-drag-region` lives on the bar ROW only in card mode — a
 * stage-level region would intercept text selection in the scrollable
 * conversation. In pill mode it stays on the capsule chrome as before.
 *
 * Errors go to the `alert` window (`alertShow`) — the pill has no room.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { SubmitEvent } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  ArrowLeftIcon,
  CameraIcon,
  GripVerticalIcon,
  MicIcon,
  SettingsIcon,
  ShieldAlertIcon,
  ShineBorder,
  cn,
} from '@marvis/ui';
import {
  alertShow,
  askClose,
  askSend,
  askSendScreenOnly,
  permissionsOpenPrefs,
  permissionsRequestScreen,
  permissionsStatus,
  windowAdjustHeight,
  windowSetBarExpanded,
  windowSetChatOpen,
  windowShowSettings,
  type AppStatePayload,
  type Gate,
} from '../lib/commands';
import {
  EV_ASK_STATE,
  EV_APP_STATE,
  EV_CAPTURE_PERMISSION_NEEDED,
  useTauriEvent,
} from '../lib/events';
import { ChatSection } from '../components/ChatSection';
import { Iris } from '../components/Iris';
import { ListenSection } from '../components/ListenSection';
import { RetryCard } from '../components/RetryCard';
import {
  BTN_LINK,
  BTN_LINK_SM,
  BTN_PRIMARY,
  BTN_SM,
  ICON_BTN,
  PANEL,
} from '../lib/classes';

/** Mirrors `app_gate` in lib.rs so the first render doesn't wait on `app:state`. */
const gateFor = (screen: boolean): Gate =>
  screen ? 'main' : 'needs_permission';

/** Every bar error goes to the alert window — the pill has no room. */
const raise = (message: string) => void alertShow(message).catch(() => {});

/** The bar row's height — the pill's window height in pill modes and
 *  the card header's height in card modes (spec: 64). */
const BAR_H = 64;
/** Card-open read: any window taller than the pill is a card. */
const OPEN_EPS = 2;
/** Card's max-height so the reported height never exceeds Rust's 900
 *  cap — frost keeps the stage's `p-1` (8 px of chrome). */
const CARD_MAX = 900 - 8;
/** Reported-height deadband + invoke throttle (was AskPanel's). */
const HEIGHT_EPS = 4;
const HEIGHT_MS = 150;

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
  const [cardOpen, setCardOpen] = useState(
    () => window.innerHeight > BAR_H + OPEN_EPS,
  );
  const [growDir, setGrowDir] = useState<'up' | 'down'>('down');
  const [listenWanted, setListenWanted] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  /** Last collapsed-mode outer y — the baseline the expand direction
   *  is detected against. */
  const collapsedY = useRef<number | null>(null);

  // Icon-row ⇄ input-row swap: the gate card and boot errors count as
  // expanded; `main` rests as the icon row until the iris opens it or
  // the user starts typing on the focused window.
  const expanded = bootError
    ? true
    : gate === 'main'
      ? open || text.length > 0
      : gate !== null;
  /** The row shows the input row in card modes regardless of `open` —
   *  it's the card header and the follow-up field. */
  const showInputRow = expanded || cardOpen;
  const section: 'chat' | 'listen' | null = !cardOpen
    ? null
    : listenWanted
      ? 'listen'
      : 'chat';

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
  // stale frame would have shipped): collapse the card — NOT `askClose`,
  // which would cancel the text-only fallback — and show the
  // permission card.
  useTauriEvent<{ permission: string }>(EV_CAPTURE_PERMISSION_NEEDED, () => {
    void windowSetChatOpen(false).catch(() => {});
    setGate('needs_permission');
  });
  // A send while in listen mode reasserts chat (`loading` = a run).
  useTauriEvent<{ state: string }>(EV_ASK_STATE, (p) => {
    if (p.state === 'loading') {
      setListenWanted(false);
    }
  });

  // Card open/close is learned from the window itself; grow direction
  // from the y-delta at expand time. While collapsed the baseline y
  // refreshes on move + resize ticks (a drag moves without resizing).
  useEffect(() => {
    const win = getCurrentWindow();
    let alive = true;
    const unMove = win.onMoved((e) => {
      if (window.innerHeight <= BAR_H + OPEN_EPS) {
        collapsedY.current = e.payload.y;
      }
    });
    const read = () => {
      const openNow = window.innerHeight > BAR_H + OPEN_EPS;
      void win
        .outerPosition()
        .then((p) => {
          if (!alive) {
            return;
          }
          setCardOpen((was) => {
            if (openNow && !was && collapsedY.current !== null) {
              setGrowDir(p.y < collapsedY.current - 0.5 ? 'up' : 'down');
            }
            return openNow;
          });
          if (!openNow) {
            collapsedY.current = p.y;
          }
        })
        .catch(() => setCardOpen(openNow));
    };
    window.addEventListener('resize', read);
    read();
    return () => {
      alive = false;
      window.removeEventListener('resize', read);
      void unMove.then((u) => u());
    };
  }, []);

  // Collapsing resets the section pick — the next open is chat.
  useEffect(() => {
    if (!cardOpen) {
      setListenWanted(false);
    }
  }, [cardOpen]);

  // The capsule IS the window under liquid glass — the pill⇄input morph
  // resizes it (idle 136 ⇄ 480). While the card is open the morph is
  // dormant: the width report is skipped so `bar_rect` (the canonical
  // pill) restores verbatim on collapse.
  useEffect(() => {
    if (!cardOpen) {
      void windowSetBarExpanded(expanded).catch(() => {});
    }
  }, [expanded, cardOpen]);

  // Report the card's desired TOTAL window height: leading + trailing
  // throttle, only on a real (>EPS) change — `adjust_height` animates
  // per call. `scrollHeight` measures the card's content (the stage is
  // window-filling): frost keeps its `p-1`, glass strips it — measuring
  // it self-corrects for both modes.
  useEffect(() => {
    const el = stageRef.current;
    if (!el || !cardOpen) {
      return;
    }
    let lastValue = -1;
    let lastSentAt = 0;
    let timer: number | undefined;
    const report = () => {
      const h = Math.min(Math.ceil(el.scrollHeight), 900);
      if (Math.abs(h - lastValue) <= HEIGHT_EPS) {
        return;
      }
      const wait = HEIGHT_MS - (Date.now() - lastSentAt);
      if (wait <= 0) {
        lastValue = h;
        lastSentAt = Date.now();
        void windowAdjustHeight(h).catch(() => {});
      } else if (timer === undefined) {
        timer = window.setTimeout(() => {
          timer = undefined;
          report();
        }, wait);
      }
    };
    const observer = new ResizeObserver(report);
    observer.observe(el);
    report();
    return () => {
      observer.disconnect();
      window.clearTimeout(timer);
    };
  }, [cardOpen]);

  // Focus the field whenever the input row shows.
  useEffect(() => {
    if (showInputRow && gate === 'main') {
      inputRef.current?.focus();
    }
  }, [showInputRow, gate]);

  // Type-to-wake on the collapsed pill; Esc collapses input → capsule,
  // and collapses the card via `ask_close` (cancel + `set_chat_open`).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (cardOpen) {
          void askClose().catch(() => {});
          return;
        }
        setText('');
        setOpen(false);
        inputRef.current?.blur();
        return;
      }
      if (
        gate !== 'main' ||
        open ||
        cardOpen ||
        e.metaKey ||
        e.ctrlKey ||
        e.altKey
      ) {
        return;
      }
      if (e.key.length === 1) {
        setText(e.key);
        setOpen(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [gate, open, cardOpen]);

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

  // Submit = ask (a follow-up while the card is open). The backend
  // expands the window itself — no local collapse needed either way.
  const submitAsk = (e: SubmitEvent<HTMLFormElement>) => {
    e.preventDefault();
    const t = text.trim();
    if (!t) {
      return;
    }
    setText('');
    void askSend(t).catch(() => raise('Send failed'));
  };

  const rowCls = cn(
    'flex min-h-0 w-full flex-none items-center gap-1.5',
    cardOpen
      ? cn(
          'h-16 border-border px-2.75',
          // The divider sits between the row and the section — which
          // side depends on the grow direction (the row is bottom-
          // pinned under `flex-col-reverse` when growing up).
          growDir === 'up' ? 'border-t' : 'border-b',
        )
      : showInputRow
        ? 'px-2.75'
        : 'justify-center px-1.75',
  );

  const row = () => {
    if (bootError) {
      return (
        <div
          className={cn(rowCls, 'justify-center')}
          data-tauri-drag-region>
          {grip}
          <RetryCard onRetry={() => void bootstrap()} />
        </div>
      );
    }
    if (gate === 'needs_permission') {
      return (
        <div
          className={rowCls}
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
        className={rowCls}
        data-tauri-drag-region>
        {grip}
        <button
          type='button'
          className={cn(BAR_BTN, 'relative')}
          aria-label={
            cardOpen ? 'Close chat' : open ? 'Back to capsule' : 'Ask Marvis'
          }
          onClick={() =>
            cardOpen
              ? void askClose().catch(() => {})
              : open
                ? collapse()
                : setOpen(true)
          }
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
            showInputRow
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
          aria-label='Listen'
          title='Listen'
          disabled={gate !== 'main'}
          onClick={() => {
            setListenWanted(true);
            void windowSetChatOpen(true).catch(() => {});
          }}>
          <MicIcon className='size-5' />
        </button>
        {/* Only rendered in the input row — the idle capsule has no
            room for a fourth control (tray menu + Cmd+, reach it
            anyway). */}
        {showInputRow && (
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
      ref={stageRef}
      className={cn(
        'group/stage glass-stage flex h-full flex-col p-1',
        growDir === 'up' ? 'justify-end' : 'justify-start',
      )}
      data-pos={growDir === 'up' ? 'bottom' : 'top'}
      data-dir={growDir}>
      <div
        className={cn(
          'group/bar glass-surface relative flex w-full flex-none select-none',
          cardOpen
            ? cn(PANEL, growDir === 'up' ? 'flex-col-reverse' : 'flex-col')
            : 'h-full flex-col justify-center rounded-full bg-[color-mix(in_oklch,var(--surface)_80%,transparent)] backdrop-blur-[14px] transition-[border-color,box-shadow] duration-(--motion-base) ease-(--ease) motion-reduce:transition-none',
        )}
        style={cardOpen ? { maxHeight: CARD_MAX } : undefined}
        data-expanded={showInputRow || undefined}
        data-tauri-drag-region={cardOpen ? undefined : 'deep'}>
        {row()}
        {section === 'chat' && <ChatSection />}
        {section === 'listen' && <ListenSection />}
        {/* Capsule shimmer — accent duotone follows light/dark via the
            tokens; masked to the border ring, pointer-events-none. */}
        <ShineBorder
          shineColor={['#A07CFE', '#FE8FB5', '#FFBE7B', 'var(--accent)']}
          borderWidth={1.8}
        />
      </div>
    </div>
  );
};

export default Bar;
