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
import { Suspense, lazy, useEffect, useRef, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { CheckIcon, CopyIcon, MessageSquareTextIcon } from '@marvis/ui';
import {
  askCurrent,
  askRetry,
  sessionEndActive,
  sessionGet,
  sessionList,
  type Message,
  type MessageAttachment,
} from '@/lib/commands';
import {
  EV_ASK_CHUNK,
  EV_ASK_DONE,
  EV_ASK_ERROR,
  EV_ASK_STATE,
  useTauriEvent,
} from '@/lib/events';
import {
  BTN_OUTLINE,
  BTN_SM,
  ICON_BTN,
  NUM,
  PANEL_BODY,
  SPIN,
  cn,
} from '@/lib/classes';
import { sessionDateLabel } from '@/components/listen/model';
import { CardHeader } from '@/components/shared/CardHeader';
import { EmptyState } from '@/components/shared/EmptyState';
import { ErrorBanner } from '@/components/shared/ErrorBanner';
import { ChatMsgMenu, type ChatMsgMeta } from '@/components/ChatMsgMenu';
import { RichText } from '@/components/shared/RichText';
import { usePresets } from '@/hooks/usePresets';

const Markdown = lazy(() =>
  import('@/components/Markdown').then((m) => ({ default: m.Markdown })),
);

type AskPhase = 'loading' | 'streaming' | 'idle';

interface AskStatePayload {
  state: AskPhase;
  /** Present on `loading` — the submitted question (ask.rs). */
  question?: string;
  /** Present on `send_chain`'s `loading` — the armed preset id
   *  (ask.rs); absent on `pre_spawn_error`'s `loading` emit. */
  preset?: string | null;
  /** Present on `loading` when the user turn carries images — the
   *  persisted `message_attachments` metadata (ask.rs). */
  attachments?: MessageAttachment[];
  /** The run's generation — every `ask:*` packet carries it (ask.rs
   *  emit fold); `liveRun` drops packets a killed run emitted before
   *  the backend's `abort` landed. */
  run?: number;
}

/** Meta fields surface in the ⋯ menu, not inline; absent while the
 *  reply is in flight. */
interface ChatMsg extends ChatMsgMeta {
  role: 'user' | 'assistant';
  content: string;
  /** Epoch seconds — persisted `messages.ts`, or a local stamp for live
   *  rows (send time on user turns, finish time on replies). */
  ts?: number;
  /** The armed `instruct` preset id — user rows only; the meta row
   *  renders it as `· {name}` (falls back to the raw id). */
  preset?: string | null;
  /** Attached images — user rows only; persisted rows carry metadata,
   *  live `loading` rows get it from the payload. */
  attachments?: MessageAttachment[];
}

/** Distance from the bottom that still counts as pinned for autoscroll. */
const PIN_PX = 24;

const nowSecs = () => Math.floor(Date.now() / 1000);

/** Fold a `loading` boundary into the message list (see file doc).
 *  `preset` is the run's armed preset id — only the appended user row
 *  carries it (retries/resyncs reuse the persisted row's own). */
const applyLoading = (
  prev: ChatMsg[],
  q: string,
  preset?: string | null,
  attachments?: MessageAttachment[],
): ChatMsg[] => {
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
    // Resync: the persisted user row already rendered — merge any
    // attachments that landed after the row painted (a retry's fresh
    // screenshot), then attach the tail. An empty payload list is "not
    // yet known" (the run's emit hasn't landed), never "none" — don't
    // wipe attachments the row already carries.
    const user =
      attachments != null && attachments.length > 0
        ? { ...last, attachments }
        : last;
    return [...prev.slice(0, -1), user, { role: 'assistant', content: '' }];
  }
  return [
    ...prev,
    { role: 'user', content: q, ts: nowSecs(), preset, attachments },
    { role: 'assistant', content: '' },
  ];
};

/** Set the tail assistant bubble's content (`done.full` / resync
 *  buffer); `meta` lands the persisted provider/model/usage row data. */
const setTail = (
  prev: ChatMsg[],
  content: string,
  meta?: ChatMsgMeta & { ts?: number },
): ChatMsg[] => {
  const last = prev[prev.length - 1];
  if (last?.role !== 'assistant') {
    return [...prev, { role: 'assistant', content, ...meta }];
  }
  return [
    ...prev.slice(0, -1),
    { role: 'assistant', ts: last.ts, content, ...meta },
  ];
};

/** Append a streamed token to the tail assistant bubble. */
const appendTail = (prev: ChatMsg[], text: string): ChatMsg[] => {
  const last = prev[prev.length - 1];
  if (last?.role !== 'assistant') {
    return [...prev, { role: 'assistant', content: text }];
  }
  return [...prev.slice(0, -1), { ...last, content: last.content + text }];
};

/** Persisted `messages` rows → the card's bubble model — shared by the
 *  mount resync and the session-switch refetch on `loading`. */
const rowsToMsgs = (rows: Message[]): ChatMsg[] =>
  rows
    .filter((r) => r.role === 'user' || r.role === 'assistant')
    .map((r) => ({
      role: r.role as ChatMsg['role'],
      content: r.content,
      ts: r.ts,
      preset: r.preset,
      attachments: r.attachments,
      provider: r.provider,
      model: r.model,
      tokensIn: r.tokens_in,
      tokensOut: r.tokens_out,
    }));

/** Show the active chat's saved and streaming messages, with interactive
 *  inline formatting for user text and Markdown for assistant replies.
 *  The header's Back action delegates to `onBack`. */
export const ChatSection = ({ onBack }: { onBack: () => void }) => {
  const [msgs, setMsgs] = useState<ChatMsg[]>([]);
  const [phase, setPhase] = useState<AskPhase>('idle');
  const [error, setError] = useState<{
    message: string;
    needsSetup: boolean;
  } | null>(null);
  /** Row index showing the copy check-flash / the open ⋯ menu. */
  const [copiedIdx, setCopiedIdx] = useState<number | null>(null);
  const [menuIdx, setMenuIdx] = useState<number | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  /** Autoscroll is on until the user scrolls away from the bottom. */
  const pinnedRef = useRef(true);
  /** The session `msgs` was loaded from — a send bound to a listen doc
   *  switches the active ask session, so `loading` refetches when this
   *  stops matching. */
  const sessionRef = useRef<number | null>(null);
  /** Chunks landing while a `loading` refetch is in flight — the send
   *  may have switched sessions, so they buffer until `applyLoading`
   *  commits rather than appending onto the stale list's tail (the
   *  previous reply's row would keep them permanently). */
  const chunkBufRef = useRef<string | null>(null);
  /** The run (`run` on every `ask:*` packet — ask.rs `generation`) the
   *  visible tail belongs to. `newChat` moves it into `deadRuns`: the
   *  backend abort kills the stream but can't recall packets already
   *  in the IPC pipe, so the dead run's late deliveries are dropped
   *  here instead of re-painting the cleared list. */
  const runRef = useRef<number | null>(null);
  const deadRunsRef = useRef(new Set<number>());

  /** Live-packet gate: runless payloads (the service's own `idle`
   *  settle after `abort`) always pass; a dead run's packet drops.
   *  Surviving packets teach `runRef` the live run — a mid-stream
   *  mount may have missed the `loading`. */
  const liveRun = (run: number | undefined): boolean => {
    if (run == null) return true;
    if (deadRunsRef.current.has(run)) return false;
    runRef.current = run;
    return true;
  };

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
            sessionRef.current = active.id;
            setMsgs(rowsToMsgs(rows));
          }
        }
        const cur = await askCurrent();
        // Seed the run filter — a mid-stream mount then "New chat" must
        // drop the live run's in-flight packets like any other.
        if (!cancelled && cur.run != null) {
          runRef.current = cur.run;
        }
        if (!cancelled && cur.state !== 'idle') {
          setMsgs((prev) =>
            setTail(
              applyLoading(prev, cur.question, null, cur.attachments),
              cur.response,
            ),
          );
          setPhase(cur.state);
        }
        // A pre-flight error's loading→error→idle completes before this
        // mount — `state` is already `idle`, so the error resyncs on its
        // own (same fold as the live `ask:error` listener).
        if (!cancelled && cur.error) {
          setError({
            message: cur.error.message,
            needsSetup: cur.error.needs_setup === true,
          });
        }
      } catch {
        /* history is best-effort */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // The `· {name}` suffix resolves a user row's preset id against the
  // merged list (`usePresets` refetches on `config:changed` — a custom
  // may be added/renamed/removed); an unknown id renders raw.
  const presets = usePresets();

  useTauriEvent<AskStatePayload>(EV_ASK_STATE, (p) => {
    if (!liveRun(p.run)) return;
    if (p.state === 'loading') {
      setError(null);
      pinnedRef.current = true;
      // The send may have switched sessions — a send from a listen doc
      // binds to that doc's own chat (ask.rs `listen_id`). Refetch the
      // now-active session rather than folding onto the old one's rows.
      // Chunks arriving mid-refetch buffer until the fold commits.
      chunkBufRef.current = '';
      const run = p.run;
      void (async () => {
        let base: ChatMsg[] | null = null;
        try {
          const sessions = await sessionList();
          const active = sessions.find(
            (s) => s.kind === 'ask' && s.ended_at === null,
          );
          if (active && active.id !== sessionRef.current) {
            sessionRef.current = active.id;
            base = rowsToMsgs(await sessionGet(active.id));
          }
        } catch {
          /* resync is best-effort — the fold below still applies */
        }
        const buffered = chunkBufRef.current ?? '';
        chunkBufRef.current = null;
        // `newChat` may have landed mid-refetch — the dead run's fold
        // must not paint its pair into the fresh list.
        if (!liveRun(run)) return;
        setMsgs((prev) => {
          const next = applyLoading(
            base ?? prev,
            p.question ?? '',
            p.preset,
            p.attachments,
          );
          return buffered ? appendTail(next, buffered) : next;
        });
      })();
    } else if (p.state === 'streaming' || p.state === 'idle') {
      // `loading` emits once — a send that opens the card mounts this
      // listener while the ~1s capture runs, so the emit can beat it
      // (a mount-resync that races the attachment persist then paints
      // the user row bare). The service-side fold always has the run's
      // attachments by now — merge them onto the painted row, matched
      // to the run's question so a same-text earlier row stays put.
      void askCurrent()
        .then((cur) => {
          if (cur.attachments.length === 0) return;
          setMsgs((prev) => {
            let idx = -1;
            for (let i = prev.length - 1; i >= 0; i--) {
              if (prev[i].role === 'user' && prev[i].content === cur.question) {
                idx = i;
                break;
              }
            }
            if (idx < 0 || (prev[idx].attachments?.length ?? 0) > 0) {
              return prev;
            }
            const next = [...prev];
            next[idx] = { ...next[idx], attachments: cur.attachments };
            return next;
          });
        })
        .catch(() => {
          /* heal is best-effort */
        });
    }
    setPhase(p.state);
  });
  useTauriEvent<{ text: string; run?: number }>(EV_ASK_CHUNK, (p) => {
    if (!liveRun(p.run)) return;
    if (chunkBufRef.current !== null) {
      chunkBufRef.current += p.text;
      return;
    }
    setMsgs((prev) => appendTail(prev, p.text));
  });
  useTauriEvent<{
    full: string;
    provider?: string;
    model?: string;
    usage?: { input?: number | null; output?: number | null } | null;
    run?: number;
  }>(EV_ASK_DONE, (p) => {
    if (!liveRun(p.run)) return;
    // `full` is authoritative — covers a dropped/duplicated chunk;
    // provider/model/usage name who ACTUALLY answered under failover.
    setMsgs((prev) =>
      setTail(prev, p.full, {
        ts: nowSecs(),
        provider: p.provider ?? null,
        model: p.model ?? null,
        tokensIn: p.usage?.input ?? null,
        tokensOut: p.usage?.output ?? null,
      }),
    );
  });
  useTauriEvent<{ message: string; needs_setup?: boolean; run?: number }>(
    EV_ASK_ERROR,
    (p) => {
      if (!liveRun(p.run)) return;
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
    // `session_end_active` aborts the in-flight run with its session —
    // mark its generation dead so packets it already emitted drop here
    // instead of re-painting the cleared list.
    if (runRef.current != null) {
      deadRunsRef.current.add(runRef.current);
      runRef.current = null;
    }
    void sessionEndActive('ask').catch(() => {});
    sessionRef.current = null;
    setMsgs([]);
    setError(null);
    setPhase('idle');
    setMenuIdx(null);
    pinnedRef.current = true;
  };

  const copyMsg = (i: number, text: string) => {
    void navigator.clipboard
      .writeText(text)
      .then(() => {
        setCopiedIdx(i);
        window.setTimeout(
          () => setCopiedIdx((k) => (k === i ? null : k)),
          1500,
        );
      })
      .catch(() => {});
  };

  /** The ⋯ menu's regenerate — `ask_retry` re-asks the session's LAST
   * user turn, so it's only meaningful on the last assistant reply. */
  const retry = () => {
    setMenuIdx(null);
    void askRetry().catch(() => {});
  };

  const lastAssistant = msgs.reduce(
    (acc, m, i) => (m.role === 'assistant' && m.content ? i : acc),
    -1,
  );

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      <CardHeader
        title='Chat'
        onBack={onBack}>
        <button
          type='button'
          className={cn(BTN_SM, BTN_OUTLINE)}
          onClick={newChat}>
          New chat
        </button>
      </CardHeader>
      {error && (
        <ErrorBanner
          message={error.message}
          needsSetup={error.needsSetup}
        />
      )}
      <div
        ref={scrollRef}
        onScroll={onScroll}
        data-card-scroll
        className={PANEL_BODY}>
        <div
          data-card-content
          className='flex flex-col gap-y-4'>
          {msgs.map((m, i) =>
            m.role === 'user' ? (
              <div
                key={i}
                className='group/row flex flex-col items-end gap-1'>
                {m.attachments != null && m.attachments.length > 0 && (
                  <div className='flex max-w-[85%] flex-wrap justify-end gap-1.5'>
                    {m.attachments.map((a) => (
                      <img
                        key={a.id}
                        src={convertFileSrc(a.path)}
                        alt={a.name}
                        className='h-16 w-auto max-w-40 rounded-xl border border-border/60 object-cover'
                      />
                    ))}
                  </div>
                )}
                <p
                  className={cn(
                    'max-w-[85%] rounded-2xl rounded-br-sm bg-accent/10 px-4 py-2',
                    'text-[14px] leading-normal wrap-break-word whitespace-pre-wrap',
                    'select-text text-accent',
                  )}>
                  <RichText
                    text={m.content}
                    interactive
                  />
                </p>
                <div className='flex items-center justify-end gap-1'>
                  {m.ts != null && (
                    <span
                      className={cn(
                        NUM,
                        'text-[10px] text-muted-foreground opacity-0 transition-opacity group-hover/row:opacity-100',
                      )}>
                      {sessionDateLabel(m.ts)}
                    </span>
                  )}
                  {m.preset && (
                    <span
                      className={cn(
                        NUM,
                        'text-[10px] text-muted-foreground opacity-0 transition-opacity group-hover/row:opacity-100',
                      )}>
                      ·{' '}
                      {presets.find((p) => p.id === m.preset)?.name ?? m.preset}
                    </span>
                  )}
                  <button
                    type='button'
                    onClick={() => copyMsg(i, m.content)}
                    aria-label='Copy message'
                    className={cn(
                      ICON_BTN,
                      'mt-1 size-4 opacity-0 transition-opacity group-hover/row:opacity-100 focus-visible:opacity-100',
                    )}>
                    {copiedIdx === i ? (
                      <CheckIcon className='size-3 text-accent' />
                    ) : (
                      <CopyIcon className='size-3' />
                    )}
                  </button>
                </div>
              </div>
            ) : (
              m.content && (
                <div
                  key={i}
                  className='group/row'>
                  <div className='prose prose-sm dark:prose-invert'>
                    <Suspense fallback={m.content}>
                      <Markdown>{m.content}</Markdown>
                    </Suspense>
                  </div>
                  <div
                    className={cn(
                      'mt-0.5 flex items-center gap-0.5 transition-opacity',
                      menuIdx === i
                        ? 'opacity-100'
                        : 'opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100',
                    )}>
                    <button
                      type='button'
                      onClick={() => copyMsg(i, m.content)}
                      aria-label='Copy response'
                      className={cn(ICON_BTN, 'size-5')}>
                      {copiedIdx === i ? (
                        <CheckIcon className='size-3 text-accent' />
                      ) : (
                        <CopyIcon className='size-3' />
                      )}
                    </button>
                    <ChatMsgMenu
                      open={menuIdx === i}
                      onOpenChange={(o) => setMenuIdx(o ? i : null)}
                      canRetry={i === lastAssistant && phase === 'idle'}
                      onRetry={retry}
                      meta={m}
                    />
                    {m.ts != null && (
                      <span
                        className={cn(
                          NUM,
                          'text-[10px] text-muted-foreground',
                        )}>
                        {sessionDateLabel(m.ts)}
                      </span>
                    )}
                  </div>
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
            <EmptyState
              icon={MessageSquareTextIcon}
              title='Ask Marvis'
              description='the conversation stays here.'
            />
          )}
        </div>
      </div>
    </div>
  );
};
