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
import { Settings, X } from '@marvis/ui';
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

const AskPanel = () => {
  const [phase, setPhase] = useState<AskPhase>('idle');
  const [question, setQuestion] = useState('');
  const [response, setResponse] = useState('');
  const [model, setModel] = useState<ModelSelection | null>(null);
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

  // The model chip is honest metadata — the active provider+model pair.
  useEffect(() => {
    void modelGetSelected()
      .then(setModel)
      .catch(() => {});
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
        className='mv-panel'>
        <header className='mv-panel-head'>
          <p
            className='mv-panel-q'
            title={question}>
            {question || 'Ask Marvis'}
          </p>
          <button
            type='button'
            className='mv-icon-btn -mt-0.5 shrink-0'
            title='Settings'
            aria-label='Settings'
            onClick={() => void windowShowSettings().catch(() => {})}>
            <Settings />
          </button>
          <button
            type='button'
            className='mv-icon-btn -mt-0.5 shrink-0'
            title='Close'
            aria-label='Close'
            onClick={() => void askClose().catch(() => {})}>
            <X />
          </button>
        </header>
        {error && (
          <div className='mv-panel-err'>
            <span className='err-msg'>{error.message}</span>
            {error.needsUnlock && (
              <button
                type='button'
                className='mv-btn mv-btn-outline'
                onClick={() => void windowShowSettings().catch(() => {})}>
                Unlock in settings
              </button>
            )}
          </div>
        )}
        <div
          ref={scrollRef}
          onScroll={onScroll}
          className='mv-panel-body'>
          {phase === 'loading' && (
            <div className='mv-thinking'>
              <span className='mv-spin' />
              Thinking…
            </div>
          )}
          {response && (
            <div className='ask-md'>
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
          {phase === 'streaming' && <span className='mv-caret' />}
          {phase === 'idle' && !response && !error && (
            <p className='mv-empty'>Ask Marvis from the bar.</p>
          )}
          {phase === 'idle' && response && model && (
            <div className='mv-chiprow'>
              <span className='mv-chip'>
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
