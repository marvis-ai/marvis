/**
 * `?view=ask` — the streaming answer panel (600 px wide, transparent,
 * frameless, non-resizable).
 *
 * One ask run at a time: `ask:state{loading}` opens a run — it carries
 * the question and resets the response buffer, so a cancelled run's
 * trailing chunks can never append into the next answer (the backend
 * generation-guards stale emits too, but the reset is what makes a new
 * run's panel clean regardless). Chunks accumulate into `response` and
 * re-render as markdown per token — reparse-per-token is the accepted
 * Phase-1 approach.
 *
 * The panel reports its own content height via `window_adjust_height`
 * (throttled, deadbanded — Rust animates every call and clamps to 900).
 * The panel's own `max-height` ceiling keeps the measured height under
 * that cap so overflow scrolls inside the answer area instead of
 * clipping past the window edge.
 *
 * No `data-tauri-drag-region` here: the bar is the grab handle and the
 * window pool moves panels programmatically.
 */
import { useEffect, useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { SettingsIcon, XIcon } from '@marvis/ui';
import {
  askClose,
  modelGetSelected,
  windowAdjustHeight,
  windowShowSettings,
  type ModelSelection,
} from '../lib/commands';
import {
  EV_ASK_CHUNK,
  EV_ASK_DONE,
  EV_ASK_ERROR,
  EV_ASK_STATE,
  useTauriEvent,
} from '../lib/events';
import {
  ASK_MD,
  BTN_OUTLINE,
  BTN_SM,
  CHIP,
  EMPTY,
  ICON_BTN,
  PANEL,
  PANEL_BODY,
  PANEL_HEAD,
  SPIN,
  cn,
} from '../lib/classes';

type AskPhase = 'loading' | 'streaming' | 'idle';

interface AskStatePayload {
  state: AskPhase;
  /** Present on `loading` — the submitted question (ask.rs). */
  question?: string;
}

/** `Panel::Ask::max_height` (windows/mod.rs) — mirrored client-side. */
const WINDOW_CAP = 900;
/** Panel's own ceiling so the reported height never exceeds the cap —
    the frost-mode stage wraps the card in `p-1` (8 px of chrome), so
    the card stops that far short of the window cap. */
const PANEL_MAX = WINDOW_CAP - 8;
/** Reported-height deadband + invoke throttle. */
const HEIGHT_EPS = 4;
const HEIGHT_MS = 150;
/** Distance from the bottom that still counts as pinned for autoscroll. */
const PIN_PX = 24;

const AskPanel = () => {
  const [phase, setPhase] = useState<AskPhase>('idle');
  const [question, setQuestion] = useState('');
  const [response, setResponse] = useState('');
  const [model, setModel] = useState<ModelSelection | null>(null);
  const [error, setError] = useState<{
    message: string;
    needsSetup: boolean;
  } | null>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  /** Autoscroll is on until the user scrolls away from the bottom. */
  const pinnedRef = useRef(true);

  // The model chip is honest metadata — the active provider+model pair.
  useEffect(() => {
    void modelGetSelected()
      .then(setModel)
      .catch(() => {});
  }, []);

  // Report content height: leading + trailing throttle, only on a real
  // (>EPS) change — `adjust_height` animates the window per call. The
  // stage is the window-filling element: frost keeps its `p-1` (card
  // + 8 px), glass strips the padding so it IS the card — measuring it
  // self-corrects for both modes.
  useEffect(() => {
    const el = stageRef.current;
    if (!el) {
      return;
    }
    let lastValue = -1;
    let lastSentAt = 0;
    let timer: number | undefined;
    const report = () => {
      const h = Math.min(Math.ceil(el.offsetHeight), WINDOW_CAP);
      if (Math.abs(h - lastValue) <= HEIGHT_EPS) {
        return;
      }
      const wait = HEIGHT_MS - (Date.now() - lastSentAt);
      if (wait <= 0) {
        lastValue = h;
        lastSentAt = Date.now();
        void windowAdjustHeight('ask', h).catch(() => {});
      } else if (timer === undefined) {
        timer = window.setTimeout(() => {
          timer = undefined;
          report();
        }, wait);
      }
    };
    const observer = new ResizeObserver(report);
    observer.observe(el);
    return () => {
      observer.disconnect();
      window.clearTimeout(timer);
    };
  }, []);

  useTauriEvent<AskStatePayload>(EV_ASK_STATE, (p) => {
    if (p.state === 'loading') {
      // New run boundary: drop the previous buffer/error and re-pin
      // autoscroll. `question` arrives on every `loading`.
      setResponse('');
      setError(null);
      pinnedRef.current = true;
      if (p.question !== undefined) {
        setQuestion(p.question);
      }
    }
    setPhase(p.state);
  });
  useTauriEvent<{ text: string }>(EV_ASK_CHUNK, (p) => {
    setResponse((r) => r + p.text);
  });
  useTauriEvent<{ full: string; provider?: string; model?: string }>(
    EV_ASK_DONE,
    (p) => {
      // `full` is authoritative — covers a dropped or duplicated chunk.
      // `provider`/`model` report who ACTUALLY answered — under failover
      // that may not be the chain head the mount-time read resolved.
      setResponse(p.full);
      if (p.provider && p.model) {
        setModel({ provider: p.provider, model: p.model });
      }
    },
  );
  useTauriEvent<{ message: string; needs_setup?: boolean }>(
    EV_ASK_ERROR,
    (p) => {
      setError({ message: p.message, needsSetup: p.needs_setup === true });
    },
  );
  // Repaint-per-token: stay glued to the bottom while pinned.
  useEffect(() => {
    const el = scrollRef.current;
    if (el && pinnedRef.current) {
      el.scrollTop = el.scrollHeight;
    }
  }, [response, phase]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (el) {
      pinnedRef.current =
        el.scrollHeight - el.scrollTop - el.clientHeight <= PIN_PX;
    }
  };

  return (
    <div
      ref={stageRef}
      className='glass-stage p-1'>
      <div
        style={{ maxHeight: PANEL_MAX }}
        className={PANEL}>
        <header className={PANEL_HEAD}>
          <p
            className='min-w-0 flex-1 text-xs leading-normal font-[550] wrap-break-word whitespace-pre-wrap line-clamp-2 select-text'
            title={question}>
            {question || 'Ask Marvis'}
          </p>
          <button
            type='button'
            className={cn(ICON_BTN, '-mt-0.5 shrink-0')}
            title='Settings'
            aria-label='Settings'
            onClick={() => void windowShowSettings().catch(() => {})}>
            <SettingsIcon className='size-4' />
          </button>
          <button
            type='button'
            className={cn(ICON_BTN, '-mt-0.5 shrink-0')}
            title='Close'
            aria-label='Close'
            onClick={() => void askClose().catch(() => {})}>
            <XIcon className='size-4' />
          </button>
        </header>
        {error && (
          <div className='flex items-center gap-2 border-b border-border bg-[color-mix(in_oklch,var(--destructive)_9%,transparent)] px-3 py-2 text-xs text-destructive'>
            <span className='min-w-0 flex-1 wrap-break-word'>
              {error.message}
            </span>
            {error.needsSetup && (
              <button
                type='button'
                className={cn(BTN_SM, BTN_OUTLINE)}
                onClick={() => void windowShowSettings().catch(() => {})}>
                Open settings
              </button>
            )}
          </div>
        )}
        <div
          ref={scrollRef}
          onScroll={onScroll}
          className={PANEL_BODY}>
          {phase === 'loading' && (
            <div className='flex items-center gap-2 py-0.5 text-xs text-muted-foreground'>
              <span className={SPIN} />
              Thinking…
            </div>
          )}
          {response && (
            <div className={ASK_MD}>
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                disallowedElements={['img']}
                components={{
                  a: ({ href, children }) => (
                    <a
                      href={href}
                      onClick={(e) => {
                        e.preventDefault();
                        if (href) void openUrl(href);
                      }}>
                      {children}
                    </a>
                  ),
                }}>
                {response}
              </ReactMarkdown>
            </div>
          )}
          {phase === 'streaming' && (
            <span className='ml-0.5 inline-block h-3.25 w-1.75 animate-caret bg-foreground align-[-2px] motion-reduce:animate-none' />
          )}
          {phase === 'idle' && !response && !error && (
            <p className={EMPTY}>Ask Marvis from the bar.</p>
          )}
          {phase === 'idle' && response && model && (
            <div className='mt-2.5 flex flex-wrap gap-1.5'>
              <span className={CHIP}>
                {model.model} · {model.provider}
              </span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default AskPanel;
