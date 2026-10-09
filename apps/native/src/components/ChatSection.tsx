/**
 * The card's chat section (was `?view=ask`): a true multi-turn
 * conversation — the active `ask` session's persisted history renders on
 * mount, then the `ask:*` live protocol drives the in-flight run's tail.
 *
 * `ask:state{loading}` is a RUN boundary, not an append: it fires once
 * per run AND once per failover retry (and `ask_current` resyncs a live
 * run whose user row is already persisted). The emit says WHICH it is
 * — `attempt`/`regenerate` on the payload — so `applyLoading` folds
 * without text-matching: retry/regenerate of the tail pair → reset its
 * assistant bubble; the pair already painted (the emit landed behind
 * the resync) → attach the live tail; anything else → append the new
 * pair, so a same-text re-send shows the second turn it persists.
 *
 * Packets are conversation-bound: `session_id` names the session the
 * run writes into — New Chat and resume detach the VIEW, never the
 * stream, so a detached run's packets drop at `sameSession` — and
 * `run` (the generation) retires on the terminal `idle`, so a
 * finished or stopped run's in-flight deliveries drop at `deadRuns`.
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
   *  emit fold); a retired run's in-flight packets drop (`deadRuns`). */
  run?: number;
  /** The session the run writes into (pipeline's emit wrap) — a run
   *  survives its session being ended/switched, so packets bound to a
   *  conversation this view isn't showing drop (`sameSession`). */
  session_id?: number;
  /** `loading` only — the 0-based failover index (ask.rs
   *  `make_loading`): `> 0` marks a retry re-emit WITHIN the run, so
   *  a same-text re-send (attempt 0 of a fresh run) appends a second
   *  pair instead of dropping the previous reply. */
  attempt?: number;
  /** `loading` only — `ask_retry`'s re-ask (ask.rs `re_asked`): the
   *  tail pair resets in place; the rejected reply's row was already
   *  deleted server-side. */
  regenerate?: boolean;
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

/** What kind of `loading` boundary this emit is — stated on the
 *  payload (`attempt`/`regenerate`) or reconstructed for a resync,
 *  never guessed from the question text. */
interface LoadingBoundary {
  /** The emit's run (`run` on the payload, `cur.run` on a resync). */
  run?: number;
  /** 0-based failover index — `> 0` is a retry of the tail pair's own
   *  run. */
  attempt?: number;
  /** `ask_retry`'s re-ask — the tail pair resets in place. */
  regenerate?: boolean;
  /** The run the painted tail pair belongs to — the emit's pair is
   *  already painted when they match (the mount resync folded the
   *  live tail before the `loading` emit landed). `null` when the
   *  tail is persisted history. */
  tailRun: number | null;
}

/** Fold a `loading` boundary into the message list (see file doc).
 *  `preset` is the run's armed preset id — only the appended user row
 *  carries it (retries/resyncs reuse the persisted row's own). */
const applyLoading = (
  prev: ChatMsg[],
  q: string,
  boundary: LoadingBoundary,
  preset?: string | null,
  attachments?: MessageAttachment[],
): ChatMsg[] => {
  const last = prev[prev.length - 1];
  const pairTail =
    last?.role === 'assistant' &&
    prev[prev.length - 2]?.role === 'user' &&
    prev[prev.length - 2].content === q;
  const userTail = last?.role === 'user' && last.content === q;
  const sameRun = boundary.run != null && boundary.run === boundary.tailRun;
  const retry = (boundary.attempt ?? 0) > 0 && sameRun;
  const regen = boundary.regenerate === true;
  // A boundary can merge attachments that landed after the user row
  // painted (a retry's fresh screenshot). `[]` is "not yet known"
  // (the run's emit hasn't landed), never "none" — don't wipe
  // attachments the row already carries.
  const mergeAtts = (row: ChatMsg): ChatMsg =>
    attachments != null && attachments.length > 0
      ? { ...row, attachments }
      : row;

  if (retry || regen) {
    // Failover retry of the tail pair's OWN run / `ask_retry`'s
    // re-ask: reset its assistant bubble — a dead attempt's partial
    // chunks or the rejected reply — keeping the user row.
    if (pairTail) {
      return [...prev.slice(0, -1), { role: 'assistant', content: '' }];
    }
    if (userTail) {
      return [...prev, { role: 'assistant', content: '' }];
    }
    // The pair isn't painted — fall through and paint it.
  } else if (sameRun) {
    // A fresh run boundary whose pair the mount resync already
    // painted (the emit was in the IPC pipe behind `ask_current`) —
    // merge its attachments onto the run's user row; never double
    // the pair.
    if (pairTail) {
      return [...prev.slice(0, -2), mergeAtts(prev[prev.length - 2]), last];
    }
    if (userTail) {
      return [
        ...prev.slice(0, -1),
        mergeAtts(last),
        { role: 'assistant', content: '' },
      ];
    }
    // Nothing of this run painted yet — fall through and paint it.
  }
  // A fresh turn — append unconditionally. A same-text re-send is a
  // real second turn: the backend persists a second user row, so the
  // card must show exactly what reopening the history replays.
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
  /** `run` generations retired by a terminal `idle` — a finished run's
   *  own, or `abort`'s emit on the composer's stop (it tags `idle`
   *  with the killed gen). A retired run's packets already in the IPC
   *  pipe can't be recalled — they drop here instead of painting
   *  post-mortem. */
  const deadRunsRef = useRef(new Set<number>());
  /** Newest `run` generation seen — they only move forward, so a
   *  packet OLDER than it is stale by definition (`deadRuns` only
   *  remembers retirements witnessed by THIS mount). */
  const runRef = useRef<number | null>(null);
  /** The run the painted tail pair belongs to — `loading` compares
   *  the emit's `run` against this to tell "the pair is already
   *  painted, attach the tail" from "a fresh turn, append". Persisted
   *  history owns no run (null until a live fold tags one). */
  const tailRunRef = useRef<number | null>(null);

  /** Live-run gate: runless payloads (the service's own emits) always
   *  pass; a retired run's packet drops, and so does one older than
   *  the newest generation seen. Surviving packets advance the bound. */
  const liveRun = (run: number | undefined): boolean => {
    if (run == null) return true;
    if (deadRunsRef.current.has(run)) return false;
    const last = runRef.current;
    if (last != null && run < last) return false;
    runRef.current = run;
    return true;
  };

  /** Same-conversation gate: a packet belongs to THIS view only when
   *  the run's session is the one being shown — New Chat / resume
   *  never kill the stream, so a detached run keeps emitting into its
   *  own session and its `session_id`-tagged packets must drop here
   *  rather than paint into the wrong conversation. Sessionless
   *  packets (the service's own emits) pass. */
  const sameSession = (sid: number | undefined): boolean =>
    sid == null || sid === sessionRef.current;

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
        // Seed the stale bound — a mid-stream mount then "New chat"
        // must still recognize the live run's packets as current.
        if (!cancelled && cur.run != null) {
          runRef.current = cur.run;
        }
        // The live tail folds onto this view only when the run's
        // session IS the one being shown — a detached run (its session
        // was ended, or another was resumed) keeps streaming into its
        // own rows and must not paint its partial reply here.
        if (
          !cancelled &&
          cur.state !== 'idle' &&
          (cur.session_id == null || cur.session_id === sessionRef.current)
        ) {
          // The painted tail belongs to the live run — tag it so the
          // run's `loading` emit (in the IPC pipe behind this resync)
          // re-attaches instead of appending a second pair.
          tailRunRef.current = cur.run ?? null;
          setMsgs((prev) =>
            setTail(
              applyLoading(
                prev,
                cur.question,
                { run: cur.run, tailRun: cur.run ?? null },
                null,
                cur.attachments,
              ),
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
      // Chunks arriving before the fold commits buffer until it does —
      // arming now is safe even for a packet that turns out detached:
      // the fold drops them together at commit.
      chunkBufRef.current = '';
      const run = p.run;
      void (async () => {
        let base: ChatMsg[] | null = null;
        try {
          // The send may have switched sessions — a send from a listen
          // doc binds to that doc's own chat (ask.rs `listen_id`).
          // Refetch the now-active session rather than folding onto
          // the old one's rows.
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
        // Commit gates, re-checked after the async gap: a terminal
        // `idle` may have retired the run mid-refetch (stop), and the
        // emit's `session_id` — compared against the JUST-refetched
        // session — tells whether this run belongs to the conversation
        // being shown. A detached run's `loading` must not paint its
        // pair into another chat.
        if (!liveRun(run) || !sameSession(p.session_id)) return;
        setError(null);
        pinnedRef.current = true;
        // Read the pre-fold tail run BEFORE retagging — the boundary
        // compares against who owned the tail pair when this emit
        // arrived, not after.
        const tailRun = tailRunRef.current;
        tailRunRef.current = run ?? tailRun;
        setMsgs((prev) => {
          const next = applyLoading(
            base ?? prev,
            p.question ?? '',
            {
              run,
              attempt: p.attempt,
              regenerate: p.regenerate,
              tailRun,
            },
            p.preset,
            p.attachments,
          );
          return buffered ? appendTail(next, buffered) : next;
        });
        setPhase('loading');
      })();
      return;
    }
    // `streaming`/`idle` — a detached run's state packets drop on the
    // session gate; sessionless service emits pass.
    if (!sameSession(p.session_id)) return;
    // `loading` emits once — a send that opens the card mounts this
    // listener while the ~1s capture runs, so the emit can beat it
    // (a mount-resync that races the attachment persist then paints
    // the user row bare). The service-side fold always has the run's
    // attachments by now — merge them onto the painted row, matched
    // to the run's question so a same-text earlier row stays put.
    void askCurrent()
      .then((cur) => {
        if (cur.session_id != null && cur.session_id !== sessionRef.current) {
          return;
        }
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
    if (p.state === 'idle' && p.run != null) {
      // Terminal boundary — the run finished or was stopped: retire it
      // so packets it already emitted drop instead of painting late.
      deadRunsRef.current.add(p.run);
    }
    setPhase(p.state);
  });
  useTauriEvent<{ text: string; run?: number; session_id?: number }>(
    EV_ASK_CHUNK,
    (p) => {
      if (!liveRun(p.run)) return;
      if (chunkBufRef.current !== null) {
        // A `loading` fold owns the session decision — buffer until it
        // commits or drops them together.
        chunkBufRef.current += p.text;
        return;
      }
      if (!sameSession(p.session_id)) return;
      setMsgs((prev) => appendTail(prev, p.text));
    },
  );
  useTauriEvent<{
    full: string;
    provider?: string;
    model?: string;
    usage?: { input?: number | null; output?: number | null } | null;
    run?: number;
    session_id?: number;
  }>(EV_ASK_DONE, (p) => {
    if (!liveRun(p.run) || !sameSession(p.session_id)) return;
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
  useTauriEvent<{
    message: string;
    needs_setup?: boolean;
    run?: number;
    session_id?: number;
  }>(EV_ASK_ERROR, (p) => {
    if (!liveRun(p.run) || !sameSession(p.session_id)) return;
    setError({ message: p.message, needsSetup: p.needs_setup === true });
  });

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
    // The in-flight run is NOT killed — it keeps streaming into the
    // ended session and the finished pair lands in history. Only the
    // view detaches: `sessionRef` cleared, so the run's
    // `session_id`-tagged packets drop at `sameSession` instead of
    // painting into the fresh list (`runRef` stays — the stale bound
    // still covers older strays).
    tailRunRef.current = null;
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
