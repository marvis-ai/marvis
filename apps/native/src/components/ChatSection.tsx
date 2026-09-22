/**
 * The card's chat section (was `?view=ask`): a true multi-turn
 * conversation — the active `ask` session's persisted history renders on
 * mount, then the `ask:*` live protocol drives the in-flight run's tail.
 *
 * `ask:state{loading}` is a RUN boundary, not an append: it fires once
 * per run AND once per failover retry (and `ask_current` resyncs a live
 * run whose user row is already persisted). `applyLoading` folds all
 * three cases: retry of the current pair → drop the dead attempt's
 * partial text; resync over an existing user row → attach the live tail;
 * anything else → append the new pair.
 *
 * Height reporting moved OUT to Bar.tsx — the observer measures the
 * whole card (this section contributes height naturally) and reports
 * total window height via `window_adjust_height`.
 */
import { useEffect, useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { SettingsIcon, XIcon } from '@marvis/ui';
import {
  askClose,
  askCurrent,
  modelGetSelected,
  sessionEndActive,
  sessionGet,
  sessionList,
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

interface ChatMsg {
  role: 'user' | 'assistant';
  content: string;
}

/** Distance from the bottom that still counts as pinned for autoscroll. */
const PIN_PX = 24;

/** Fold a `loading` boundary into the message list (see file doc). */
const applyLoading = (prev: ChatMsg[], q: string): ChatMsg[] => {
  const last = prev[prev.length - 1];
  if (
    last?.role === 'assistant' &&
    prev[prev.length - 2]?.role === 'user' &&
    prev[prev.length - 2].content === q
  ) {
    // Failover retry of the current run — drop the dead attempt's text.
    return [...prev.slice(0, -1), { role: 'assistant', content: '' }];
  }
  if (last?.role === 'user' && last.content === q) {
    // Resync: the persisted user row already rendered — attach the tail.
    return [...prev, { role: 'assistant', content: '' }];
  }
  return [
    ...prev,
    { role: 'user', content: q },
    { role: 'assistant', content: '' },
  ];
};

/** Set the tail assistant bubble's content (`done.full` / resync buffer). */
const setTail = (prev: ChatMsg[], content: string): ChatMsg[] => {
  const last = prev[prev.length - 1];
  if (last?.role !== 'assistant') {
    return [...prev, { role: 'assistant', content }];
  }
  return [...prev.slice(0, -1), { role: 'assistant', content }];
};

/** Append a streamed token to the tail assistant bubble. */
const appendTail = (prev: ChatMsg[], text: string): ChatMsg[] => {
  const last = prev[prev.length - 1];
  if (last?.role !== 'assistant') {
    return [...prev, { role: 'assistant', content: text }];
  }
  return [
    ...prev.slice(0, -1),
    { role: 'assistant', content: last.content + text },
  ];
};

export const ChatSection = () => {
  const [msgs, setMsgs] = useState<ChatMsg[]>([]);
  const [phase, setPhase] = useState<AskPhase>('idle');
  const [model, setModel] = useState<ModelSelection | null>(null);
  const [error, setError] = useState<{
    message: string;
    needsSetup: boolean;
  } | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  /** Autoscroll is on until the user scrolls away from the bottom. */
  const pinnedRef = useRef(true);

  // Mount resync: active `ask` session → persisted history; then
  // `ask_current` folds an in-flight run's tail on top. Best-effort —
  // live events drive the card even if the reads fail.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const sessions = await sessionList();
        const active = sessions.find(
          (s) => s.kind === 'ask' && s.ended_at === null,
        );
        if (active) {
          const rows = await sessionGet(active.id);
          if (!cancelled) {
            setMsgs(
              rows
                .filter((r) => r.role === 'user' || r.role === 'assistant')
                .map((r) => ({
                  role: r.role as ChatMsg['role'],
                  content: r.content,
                })),
            );
          }
        }
        const cur = await askCurrent();
        if (!cancelled && cur.state !== 'idle') {
          setMsgs((prev) =>
            setTail(applyLoading(prev, cur.question), cur.response),
          );
          setPhase(cur.state);
        }
      } catch {
        /* history is best-effort */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // The model chip is honest metadata — the active provider+model pair.
  useEffect(() => {
    void modelGetSelected()
      .then(setModel)
      .catch(() => {});
  }, []);

  useTauriEvent<AskStatePayload>(EV_ASK_STATE, (p) => {
    if (p.state === 'loading') {
      setError(null);
      pinnedRef.current = true;
      setMsgs((prev) => applyLoading(prev, p.question ?? ''));
    }
    setPhase(p.state);
  });
  useTauriEvent<{ text: string }>(EV_ASK_CHUNK, (p) => {
    setMsgs((prev) => appendTail(prev, p.text));
  });
  useTauriEvent<{ full: string; provider?: string; model?: string }>(
    EV_ASK_DONE,
    (p) => {
      // `full` is authoritative — covers a dropped/duplicated chunk;
      // `provider`/`model` report who ACTUALLY answered under failover.
      setMsgs((prev) => setTail(prev, p.full));
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
  }, [msgs, phase]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (el) {
      pinnedRef.current =
        el.scrollHeight - el.scrollTop - el.clientHeight <= PIN_PX;
    }
  };

  const newChat = () => {
    void sessionEndActive('ask').catch(() => {});
    setMsgs([]);
    setError(null);
    setPhase('idle');
    pinnedRef.current = true;
  };

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      <header className={PANEL_HEAD}>
        <p className='min-w-0 flex-1 text-xs leading-normal font-[550] select-text'>
          Chat
        </p>
        <button
          type='button'
          className={cn(BTN_SM, BTN_OUTLINE)}
          onClick={newChat}>
          New chat
        </button>
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
        {msgs.map((m, i) =>
          m.role === 'user' ? (
            <div
              key={i}
              className='mb-2 flex justify-end'>
              <p className='max-w-[85%] rounded-2xl rounded-br-sm bg-fg-soft px-3 py-1.5 text-[13px] leading-[1.5] wrap-break-word whitespace-pre-wrap select-text'>
                {m.content}
              </p>
            </div>
          ) : (
            m.content && (
              <div
                key={i}
                className={cn(ASK_MD, 'mb-2.5')}>
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
                  {m.content}
                </ReactMarkdown>
              </div>
            )
          ),
        )}
        {phase === 'loading' && (
          <div className='flex items-center gap-2 py-0.5 text-xs text-muted-foreground'>
            <span className={SPIN} />
            Thinking…
          </div>
        )}
        {phase === 'streaming' && (
          <span className='ml-0.5 inline-block h-3.25 w-1.75 animate-caret bg-foreground align-[-2px] motion-reduce:animate-none' />
        )}
        {msgs.length === 0 && phase === 'idle' && !error && (
          <p className={EMPTY}>Ask Marvis — the conversation stays here.</p>
        )}
        {phase === 'idle' &&
          model &&
          msgs.some((m) => m.role === 'assistant' && m.content) && (
            <div className='mt-2.5 flex flex-wrap gap-1.5'>
              <span className={CHIP}>
                {model.model} · {model.provider}
              </span>
            </div>
          )}
      </div>
    </div>
  );
};
