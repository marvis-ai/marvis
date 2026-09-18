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
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Button, X } from '@marvis/ui';
import {
  askClose,
  windowAdjustHeight,
  windowShowSettings,
} from '../lib/commands';
import {
  EV_ASK_CHUNK,
  EV_ASK_DONE,
  EV_ASK_ERROR,
  EV_ASK_SCROLL,
  EV_ASK_STATE,
  useTauriEvent,
} from '../lib/events';

type AskPhase = 'loading' | 'streaming' | 'idle';

interface AskStatePayload {
  state: AskPhase;
  /** Present on `loading` — the submitted question (ask.rs). */
  question?: string;
}

/** `Panel::Ask::max_height` (windows/mod.rs) — mirrored client-side. */
const WINDOW_CAP = 900;
/** Outer `p-1` wrapper: 4 px top + bottom of transparent chrome. */
const CHROME_PX = 8;
/** Panel's own ceiling so the reported height never exceeds the cap. */
const PANEL_MAX = WINDOW_CAP - CHROME_PX;
const SCROLL_STEP = 200;
/** Reported-height deadband + invoke throttle. */
const HEIGHT_EPS = 4;
const HEIGHT_MS = 150;
/** Distance from the bottom that still counts as pinned for autoscroll. */
const PIN_PX = 24;

export default function AskPanel() {
  const [phase, setPhase] = useState<AskPhase>('idle');
  const [question, setQuestion] = useState('');
  const [response, setResponse] = useState('');
  const [error, setError] = useState<{
    message: string;
    needsUnlock: boolean;
  } | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  /** Autoscroll is on until the user scrolls away from the bottom. */
  const pinnedRef = useRef(true);

  useEffect(() => {
    document.body.classList.add('ask');
    return () => document.body.classList.remove('ask');
  }, []);

  // Report content height: leading + trailing throttle, only on a real
  // (>EPS) change — `adjust_height` animates the window per call.
  useEffect(() => {
    const el = panelRef.current;
    if (!el) {
      return;
    }
    let lastValue = -1;
    let lastSentAt = 0;
    let timer: number | undefined;
    const report = () => {
      const h = Math.min(Math.ceil(el.offsetHeight + CHROME_PX), WINDOW_CAP);
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
  useTauriEvent<{ full: string }>(EV_ASK_DONE, (p) => {
    // `full` is authoritative — covers a dropped or duplicated chunk.
    setResponse(p.full);
  });
  useTauriEvent<{ message: string; needs_unlock?: boolean }>(
    EV_ASK_ERROR,
    (p) => {
      setError({ message: p.message, needsUnlock: p.needs_unlock === true });
    },
  );
  // Hotkey-driven scroll (lib.rs ScrollUp/ScrollDown).
  useTauriEvent<{ dir: 'up' | 'down' }>(EV_ASK_SCROLL, (p) => {
    scrollRef.current?.scrollBy({
      top: p.dir === 'down' ? SCROLL_STEP : -SCROLL_STEP,
      behavior: 'smooth',
    });
  });

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
    <div className='p-1'>
      <div
        ref={panelRef}
        style={{ maxHeight: PANEL_MAX }}
        className='flex flex-col overflow-hidden rounded-2xl border border-border bg-card/90 shadow-lg backdrop-blur'>
        <header className='flex items-start gap-1 border-b border-border px-3 py-2'>
          <p
            className='min-w-0 flex-1 select-text text-xs leading-5 font-medium break-words whitespace-pre-wrap text-foreground line-clamp-2'
            title={question}>
            {question || 'Ask Marvis'}
          </p>
          <Button
            type='button'
            size='icon-xs'
            variant='ghost'
            title='Close'
            className='-mt-0.5 shrink-0 text-muted-foreground'
            onClick={() => void askClose().catch(() => {})}>
            <X />
          </Button>
        </header>
        {error && (
          <div className='flex items-center gap-2 border-b border-border bg-destructive/10 px-3 py-2'>
            <span className='min-w-0 flex-1 text-xs break-words text-destructive'>
              {error.message}
            </span>
            {error.needsUnlock && (
              <Button
                size='xs'
                variant='outline'
                onClick={() => void windowShowSettings().catch(() => {})}>
                Unlock in settings
              </Button>
            )}
          </div>
        )}
        <div
          ref={scrollRef}
          onScroll={onScroll}
          className='min-h-0 flex-1 overflow-y-auto px-3 py-2 select-text'>
          {phase === 'loading' && (
            <div className='flex items-center gap-2 py-0.5 text-xs text-muted-foreground'>
              <span className='size-3 animate-spin rounded-full border-2 border-muted-foreground/30 border-t-foreground' />
              Thinking…
            </div>
          )}
          {response && (
            <div className='ask-md text-sm text-foreground'>
              <ReactMarkdown remarkPlugins={[remarkGfm]}>
                {response}
              </ReactMarkdown>
            </div>
          )}
          {phase === 'idle' && !response && !error && (
            <p className='text-xs text-muted-foreground'>
              Ask Marvis from the bar.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
