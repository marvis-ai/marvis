import { useEffect, useRef, useState } from 'react';
import {
  configGet,
  listenStatus,
  listenStop,
  transcriptsFor,
  summaryLatest,
  windowShowSettings,
  type ListenStatus,
} from '../lib/commands';
import {
  EV_LISTEN_ERROR,
  EV_LISTEN_STATE,
  EV_LISTEN_SUMMARY,
  EV_LISTEN_TURN,
  useTauriEvent,
  type ListenErrorPayload,
  type ListenStatePayload,
  type ListenSummaryPayload,
  type ListenTurnPayload,
} from '../lib/events';
import { BTN_OUTLINE, BTN_SM, CHIP, EMPTY, cn } from '../lib/classes';

type Turn = ListenTurnPayload & {
  interim?: boolean;
};

type Summary = ListenSummaryPayload;

const waveformHeights = ['h-2', 'h-3.5', 'h-4.5', 'h-3', 'h-1.75'];

export const ListenSection = () => {
  const [status, setStatus] = useState<ListenStatus>({
    state: 'idle',
    provider: null,
    session_id: null,
    turns: 0,
    mic: false,
  });
  const [turns, setTurns] = useState<Turn[]>([]);
  const sessionRef = useRef<number | null>(null);
  const [summary, setSummary] = useState<Summary | null>(null);
  const [error, setError] = useState<ListenErrorPayload | null>(null);
  const [model, setModel] = useState<string | null>(null);

  useEffect(() => {
    void configGet()
      .then((config) => setModel(config.models.stt_model || null))
      .catch(() => {});
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const current = await listenStatus();
        if (cancelled) return;
        setStatus(current);
        sessionRef.current = current.session_id;
        if (current.session_id === null) return;
        const [rows, latest] = await Promise.all([
          transcriptsFor(current.session_id),
          summaryLatest(current.session_id),
        ]);
        if (cancelled || sessionRef.current !== current.session_id) return;
        setTurns((live) => {
          const persisted = rows.map((row) => ({ ...row, final: true }));
          const keys = new Set(
            live.map((turn) => `${turn.speaker}:${turn.ts}:${turn.text}`),
          );
          return [
            ...persisted.filter(
              (row) => !keys.has(`${row.speaker}:${row.ts}:${row.text}`),
            ),
            ...live,
          ];
        });
        if (latest) setSummary(latest);
      } catch {
        // Live events remain authoritative when persistence is unavailable.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useTauriEvent<ListenStatePayload>(EV_LISTEN_STATE, (next) => {
    if (next.state === 'listening' && next.session_id !== sessionRef.current) {
      sessionRef.current = next.session_id;
      setTurns([]);
      setSummary(null);
      setError(null);
    } else if (next.state === 'listening') {
      setError(null);
    }
    setStatus((previous) => ({ ...previous, ...next }));
  });
  useTauriEvent<Turn>(EV_LISTEN_TURN, (turn) => {
    if (sessionRef.current !== null && turn.session_id !== sessionRef.current) {
      return;
    }
    setError(null);
    setTurns((previous) => {
      const withoutInterim = previous.filter(
        (item) => !(item.interim && item.speaker === turn.speaker),
      );
      return turn.final
        ? [...withoutInterim, { ...turn, interim: false }]
        : [...withoutInterim, { ...turn, interim: true }];
    });
  });
  useTauriEvent<ListenSummaryPayload>(EV_LISTEN_SUMMARY, (next) => {
    setSummary(next);
    setError(null);
  });
  useTauriEvent<ListenErrorPayload>(EV_LISTEN_ERROR, setError);

  const stop = () => {
    void listenStop().catch(() => {});
  };
  const listening = status.state === 'listening';
  const provider = status.provider ?? 'stt';

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      <div className='flex items-center justify-between border-b border-border px-3 py-2.25'>
        <div className='flex min-w-0 items-center gap-2'>
          <span
            className={cn(
              'flex h-4.5 items-center gap-0.75',
              listening && 'text-accent',
            )}
            aria-hidden>
            {waveformHeights.map((height, index) => (
              <i
                key={height}
                className={cn(
                  height,
                  'w-0.75 rounded-xs bg-current',
                  listening && 'animate-waveform',
                )}
                style={
                  listening ? { animationDelay: `${index * 90}ms` } : undefined
                }
              />
            ))}
          </span>
          <span className='text-xs font-[550]'>
            {listening ? 'Listening' : 'Listen'}
          </span>
        </div>
        {listening && (
          <button
            type='button'
            className={cn(BTN_SM, BTN_OUTLINE)}
            onClick={stop}>
            Stop
          </button>
        )}
      </div>
      {error && (
        <div className='flex items-center gap-2 border-b border-border bg-[color-mix(in_oklch,var(--destructive)_9%,transparent)] px-3 py-2 text-xs text-destructive'>
          <span className='min-w-0 flex-1 wrap-break-word'>
            {error.message}
          </span>
          {error.needs_setup && (
            <button
              type='button'
              className={cn(BTN_SM, BTN_OUTLINE)}
              onClick={() => void windowShowSettings().catch(() => {})}>
              Open settings
            </button>
          )}
        </div>
      )}
      {listening && !status.mic && !error && (
        <div className='border-b border-border px-3 py-2 text-xs text-muted-foreground'>
          Microphone unavailable; system audio only.
        </div>
      )}
      <div className='min-h-0 flex-1 overflow-y-auto px-3.5 py-3 text-[13px] leading-[1.6]'>
        {turns.map((turn, index) => (
          <div
            key={`${turn.ts}-${index}`}
            className={cn(
              'mb-2 flex gap-2',
              turn.speaker === 'me' && 'justify-end',
            )}>
            <span className='w-8 flex-none text-[10px] font-semibold uppercase text-muted-foreground'>
              {turn.speaker}
            </span>
            <p
              className={cn(
                'max-w-[82%] wrap-break-word whitespace-pre-wrap select-text',
                turn.interim && 'text-muted-foreground italic',
              )}>
              {turn.text}
            </p>
          </div>
        ))}
        {summary && (
          <section className='mt-3 border-t border-border pt-2.5'>
            <p className='text-xs font-semibold'>
              TLDR{summary.topic ? ` · ${summary.topic}` : ''}
            </p>
            <p className='mt-1 select-text'>{summary.tldr}</p>
            {summary.bullets.slice(0, 5).length > 0 && (
              <ul className='mt-1 list-disc pl-4'>
                {summary.bullets.slice(0, 5).map((bullet) => (
                  <li key={bullet}>{bullet}</li>
                ))}
              </ul>
            )}
            {summary.follow_ups.slice(0, 3).length > 0 && (
              <div className='mt-2 flex flex-wrap gap-1.5'>
                {summary.follow_ups.slice(0, 3).map((followUp) => (
                  <span
                    key={followUp}
                    className={CHIP}>
                    {followUp}
                  </span>
                ))}
              </div>
            )}
          </section>
        )}
        {turns.length === 0 && !summary && !error && (
          <p className={EMPTY}>
            {listening
              ? 'Speak naturally — your transcript will appear here.'
              : 'Start listening to capture a conversation.'}
          </p>
        )}
        <div className='mt-2 flex flex-wrap gap-1.5'>
          <span className={CHIP}>
            {provider}
            {model ? ` · ${model}` : ' · stt'}
          </span>
        </div>
      </div>
    </div>
  );
};
