import { useEffect, useMemo, useRef, useState } from 'react';
import {
  configGet,
  listenStatus,
  listenStop,
  transcriptsFor,
  summaryLatest,
  whisperStatus,
  windowShowSettings,
  type Config,
  type ListenStatus,
  type WhisperBinarySource,
  type WhisperStatus,
} from '../lib/commands';
import {
  EV_CONFIG_CHANGED,
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
import { BTN_OUTLINE, BTN_SM, CHIP, EMPTY, NUM, cn } from '../lib/classes';

type Turn = ListenTurnPayload & {
  interim?: boolean;
};

type Summary = ListenSummaryPayload;

const waveformHeights = ['h-2', 'h-3.5', 'h-4.5', 'h-3', 'h-1.75'];

/* ─── transcript document model ──────────────────────────────────
   Consecutive turns from one speaker identity (channel + diarized
   voice cluster) merge into a block: one header, a paragraph per
   turn. A trailing interim rides the open block. */

interface TurnIdentity {
  speaker: 'me' | 'them';
  speaker_idx: number | null;
}

const speakerKey = (turn: TurnIdentity) =>
  `${turn.speaker}:${turn.speaker_idx ?? ''}`;

const speakerName = (turn: TurnIdentity) =>
  turn.speaker === 'me'
    ? turn.speaker_idx != null && turn.speaker_idx > 0
      ? `Guest ${turn.speaker_idx}`
      : 'You'
    : turn.speaker_idx == null
      ? 'Speaker'
      : `Speaker ${turn.speaker_idx + 1}`;

const SPEAKER_COLOR_CLASSES = [
  'text-speaker-1',
  'text-speaker-2',
  'text-speaker-3',
  'text-speaker-4',
] as const;

const speakerColor = (turn: TurnIdentity) =>
  turn.speaker === 'me' && !(turn.speaker_idx != null && turn.speaker_idx > 0)
    ? 'text-accent'
    : turn.speaker_idx == null
      ? 'text-fg-2'
      : SPEAKER_COLOR_CLASSES[turn.speaker_idx % SPEAKER_COLOR_CLASSES.length];

interface TurnBlock {
  key: string;
  name: string;
  color: string;
  ts: number;
  finals: Turn[];
  interim: Turn | null;
}

const timeLabel = (ts: number) => {
  const date = new Date(ts * 1000);
  const hh = String(date.getHours()).padStart(2, '0');
  const mm = String(date.getMinutes()).padStart(2, '0');
  return `${hh}:${mm}`;
};

const whisperSourceLabel = (source: WhisperBinarySource | null) => {
  if (source === 'Bundled') return 'Bundled with Marvis';
  if (source) return 'Custom whisper-cli detected';
  return 'Whisper CLI unavailable';
};

export const ListenSection = () => {
  const [status, setStatus] = useState<ListenStatus>({
    state: 'idle',
    provider: null,
    session_id: null,
    turns: 0,
    mic: false,
    error: null,
  });
  const [turns, setTurns] = useState<Turn[]>([]);
  const sessionRef = useRef<number | null>(null);
  const [summary, setSummary] = useState<Summary | null>(null);
  const [error, setError] = useState<ListenErrorPayload | null>(null);
  const [provider, setProvider] = useState<string | null>(null);
  const [model, setModel] = useState<string | null>(null);
  const [whisper, setWhisper] = useState<WhisperStatus | null>(null);

  const applyConfig = (config: Config) => {
    setProvider(config.models.stt_provider || null);
    setModel(config.models.stt_model || null);
  };

  useEffect(() => {
    void configGet()
      .then(applyConfig)
      .catch(() => {});
  }, []);

  useTauriEvent<Config>(EV_CONFIG_CHANGED, applyConfig);

  useEffect(() => {
    void whisperStatus()
      .then(setWhisper)
      .catch(() =>
        setWhisper({
          binary: null,
          binary_status: { available: false, source: null },
          models: [],
          download: null,
        }),
      );
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const current = await listenStatus();
        if (cancelled) return;
        setStatus(current);
        setError(current.error);
        sessionRef.current = current.session_id;
        if (current.session_id === null) return;
        const [rows, latest] = await Promise.all([
          transcriptsFor(current.session_id),
          summaryLatest(current.session_id),
        ]);
        if (cancelled || sessionRef.current !== current.session_id) return;
        setTurns((live) => {
          // Persisted rows (`content`) fold into the live-turn shape (`text`).
          const persisted: Turn[] = rows.map((row) => ({
            speaker: row.speaker,
            speaker_idx: row.speaker_idx,
            text: row.content,
            ts: row.ts,
            session_id: row.session_id,
            final: true,
          }));
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
    }
    setError(next.error);
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
  const activeProvider = provider ?? status.provider ?? 'stt';

  // Document scroll: follow live output while pinned; scrolling up
  // releases the pin and offers a way back to the live edge.
  const scrollRef = useRef<HTMLDivElement>(null);
  const [pinned, setPinned] = useState(true);
  const handleScroll = () => {
    const el = scrollRef.current;
    if (el) setPinned(el.scrollHeight - el.scrollTop - el.clientHeight < 48);
  };
  const blocks = useMemo(() => {
    const out: TurnBlock[] = [];
    for (const turn of turns) {
      const key = speakerKey(turn);
      let block = out[out.length - 1];
      if (!block || block.key !== key) {
        block = {
          key,
          name: speakerName(turn),
          color: speakerColor(turn),
          ts: turn.ts,
          finals: [],
          interim: null,
        };
        out.push(block);
      }
      if (turn.interim) {
        block.interim = turn;
      } else {
        block.finals.push(turn);
        block.interim = null;
      }
    }
    return out;
  }, [turns]);
  useEffect(() => {
    const el = scrollRef.current;
    if (el && pinned) el.scrollTop = el.scrollHeight;
  }, [blocks, summary, pinned]);

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
      <div className='relative min-h-0 flex-1'>
        <div
          ref={scrollRef}
          onScroll={handleScroll}
          className='h-full overflow-y-auto px-3.5 py-3 text-[13px] leading-[1.6]'>
          {blocks.map((block) => (
            <div
              key={`${block.key}-${block.ts}`}
              className='mb-3.5'>
              <div className={cn('flex items-center gap-1.5', block.color)}>
                <span
                  aria-hidden
                  className='size-1.75 flex-none rounded-full bg-current'
                />
                <span className='text-[12px] font-[650] tracking-[-0.005em]'>
                  {block.name}
                </span>
                <span className={cn(NUM, 'text-[10px] text-muted-foreground')}>
                  {timeLabel(block.ts)}
                </span>
              </div>
              <p className='mt-1 wrap-break-word whitespace-pre-wrap select-text'>
                {block.finals.map((turn) => turn.text).join(' ')}
                {block.interim && (
                  <span className='text-muted-foreground'>
                    {block.finals.length > 0 ? ' ' : ''}
                    {block.interim.text}
                    <span
                      aria-hidden
                      className='animate-caret ml-0.5 inline-block h-[0.95em] w-[1.5px] translate-y-[0.15em] bg-current'
                    />
                  </span>
                )}
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
              {activeProvider}
              {model ? ` · ${model}` : ' · stt'}
            </span>
            {activeProvider === 'whisper' && whisper && (
              <span className={CHIP}>
                {whisperSourceLabel(whisper.binary_status.source)}
              </span>
            )}
          </div>
        </div>
        {!pinned && turns.length > 0 && (
          <button
            type='button'
            className={cn(
              BTN_SM,
              BTN_OUTLINE,
              'absolute bottom-2 left-1/2 -translate-x-1/2 bg-surface shadow-sm',
            )}
            onClick={() => {
              const el = scrollRef.current;
              if (el) el.scrollTop = el.scrollHeight;
              setPinned(true);
            }}>
            Jump to live
          </button>
        )}
      </div>
    </div>
  );
};
