import { useEffect, useMemo, useRef, useState } from 'react';

import { CaptionsIcon } from '@marvis/ui';
import {
  configGet,
  listenPause,
  listenResume,
  listenStatus,
  listenStop,
  transcriptsFor,
  summaryLatest,
  windowShowSettings,
  type Config,
  type ListenStatus,
} from '@/lib/commands';
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
} from '@/lib/events';
import { BTN_OUTLINE, BTN_SM, cn } from '@/lib/classes';
import {
  buildBlocks,
  elapsedLabel,
  sessionDateLabel,
  transcriptCopyText,
  type ListenViewing,
  type Turn,
} from '@/components/listen/model';
import { ListenHeader } from '@/components/listen/ListenHeader';
import { SpeakerFilter } from '@/components/listen/SpeakerFilter';
import { SummaryStrip } from '@/components/listen/SummaryStrip';
import { TranscriptBlocks } from '@/components/listen/TranscriptBlocks';
import { EmptyState } from '@/components/shared/EmptyState';

/** The structured meeting document — header (title, badge, timer,
 *  controls), speaker filter, timestamped transcript blocks, and the
 *  jump-to-live scroll affordance. `viewing === null` is the live
 *  capture; a set `viewing` renders a finished session read off
 *  `transcripts_for`/`summary_latest` while live events stay walled
 *  off behind `viewingRef`. */
export const ListenSection = ({
  viewing,
  onSessionEnded,
  onBack,
}: {
  viewing: ListenViewing | null;
  onSessionEnded: (v: ListenViewing) => void;
  onBack: () => void;
}) => {
  const live = viewing === null;
  /** Live mirror of `viewing` for Tauri event handlers — an event that
   *  lands while a finished doc is on screen must never mutate it. */
  const viewingRef = useRef(viewing);
  viewingRef.current = viewing;

  const [status, setStatus] = useState<ListenStatus>({
    state: 'idle',
    provider: null,
    session_id: null,
    turns: 0,
    mic: false,
    error: null,
    started_at: null,
    paused_secs: 0,
    paused_since: null,
  });
  const [turns, setTurns] = useState<Turn[]>([]);
  const sessionRef = useRef<number | null>(null);
  const [summary, setSummary] = useState<ListenSummaryPayload | null>(null);
  const [error, setError] = useState<ListenErrorPayload | null>(null);
  const [provider, setProvider] = useState<string | null>(null);
  const [model, setModel] = useState<string | null>(null);
  const [copiedAll, setCopiedAll] = useState(false);
  const [filterKey, setFilterKey] = useState<string | null>(null);

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

  // Live resync — on mount AND whenever `viewing` returns to null (the
  // Listen tab's "back to live"). A same-mount viewing→null transition
  // would otherwise keep the finished doc's turns/status/sessionRef on
  // screen; clearing first keeps stale rows out of the merge below.
  useEffect(() => {
    if (!live) return;
    setTurns([]);
    setSummary(null);
    let cancelled = false;
    void (async () => {
      try {
        const current = await listenStatus();
        if (cancelled || viewingRef.current) return;
        setStatus(current);
        setError(current.error);
        sessionRef.current = current.session_id;
        if (current.session_id === null) return;
        const [rows, latest] = await Promise.all([
          transcriptsFor(current.session_id),
          summaryLatest(current.session_id),
        ]);
        if (
          cancelled ||
          viewingRef.current ||
          sessionRef.current !== current.session_id
        )
          return;
        setTurns((liveTurns) => {
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
            liveTurns.map((turn) => `${turn.speaker}:${turn.ts}:${turn.text}`),
          );
          return [
            ...persisted.filter(
              (row) => !keys.has(`${row.speaker}:${row.ts}:${row.text}`),
            ),
            ...liveTurns,
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
  }, [live]);

  // A viewed session re-reads its document — persisted turns plus the
  // last summary. `turns` wholesale replaces (the live list and the
  // stored rows can disagree on a still-open interim).
  useEffect(() => {
    if (!viewing) return;
    let cancelled = false;
    void Promise.all([transcriptsFor(viewing.id), summaryLatest(viewing.id)])
      .then(([rows, latest]) => {
        if (cancelled) return;
        setTurns(
          rows.map((r) => ({
            speaker: r.speaker,
            speaker_idx: r.speaker_idx,
            text: r.content,
            ts: r.ts,
            session_id: r.session_id,
            final: true,
          })),
        );
        if (latest) setSummary(latest);
        else setSummary(null);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [viewing?.id]);

  // The speaker filter is display-only; a new session or a new viewed
  // document clears it.
  useEffect(() => {
    setFilterKey(null);
  }, [viewing?.id, status.session_id]);

  useTauriEvent<ListenStatePayload>(EV_LISTEN_STATE, (next) => {
    if (viewingRef.current) return;
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
    if (viewingRef.current) return;
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
    if (viewingRef.current) return;
    setSummary(next);
    setError(null);
  });
  useTauriEvent<ListenErrorPayload>(EV_LISTEN_ERROR, (next) => {
    if (viewingRef.current) return;
    setError(next);
  });

  const listening = status.state === 'listening';
  const paused = status.state === 'paused';
  /** The STT engine label — `status.provider` names the running
   *  session's provider when there is one, config's otherwise. It
   *  doubles as the `stt` handed to the finished-doc view on stop. */
  const engine = `${status.provider ?? provider ?? 'stt'}${model ? ` ${model}` : ''}`;

  // Elapsed RECORDING time: live = (paused_since ?? now) − started_at −
  // paused_secs (the interval stops on pause so `now` freezes — but
  // `paused_since` carries the frozen value anyway); a viewed doc shows
  // its duration (endedAt ?? now for a still-open session).
  const [now, setNow] = useState(() => Date.now() / 1000);
  const startedAt = live ? status.started_at : viewing.startedAt;
  const ticking = live ? listening : viewing.endedAt == null;
  useEffect(() => {
    if (!ticking) return;
    // Re-anchor on (re)start — after a pause `now` is stale by the whole
    // paused span, so the first tick would dip the timer for ~1s.
    setNow(Date.now() / 1000);
    const id = window.setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => window.clearInterval(id);
  }, [ticking]);
  const elapsed =
    startedAt == null
      ? 0
      : Math.max(
          0,
          live
            ? (status.paused_since ?? now) - startedAt - status.paused_secs
            : (viewing.endedAt ?? now) - startedAt,
        );

  // Read the ids BEFORE `listenStop` — the command clears the session
  // snapshot; the card then stays open on the finished document.
  const stop = () => {
    const id = status.session_id;
    const started = status.started_at;
    // The doc swap waits on the invoke settling (finally, not then) —
    // even a failed stop leaves the session dead backend-side, so the
    // finished document is still the right surface.
    void listenStop()
      .catch(() => {})
      .finally(() => {
        if (id != null && started != null) {
          onSessionEnded({
            id,
            startedAt: started,
            endedAt: Date.now() / 1000,
            stt: engine,
          });
        }
      });
  };

  const blocks = useMemo(() => buildBlocks(turns), [turns]);
  const speakers = useMemo(() => {
    const seen = new Map<
      string,
      { key: string; name: string; color: string }
    >();
    for (const block of blocks) {
      if (!seen.has(block.key)) {
        seen.set(block.key, {
          key: block.key,
          name: block.name,
          color: block.color,
        });
      }
    }
    return [...seen.values()];
  }, [blocks]);
  const shown = filterKey
    ? blocks.filter((block) => block.key === filterKey)
    : blocks;

  const copyAll = () => {
    void navigator.clipboard
      .writeText(transcriptCopyText(blocks, startedAt, summary))
      .then(() => {
        setCopiedAll(true);
        window.setTimeout(() => setCopiedAll(false), 1500);
      })
      .catch(() => {});
  };

  // The state pill is live-only — a viewed doc (from History or the
  // just-stopped transition) carries no badge; its duration sits on
  // the SpeakerFilter row.
  const badge = live
    ? paused
      ? ('PAUSED' as const)
      : status.session_id != null && listening
        ? ('LISTENING' as const)
        : null
    : null;
  const title = summary?.topic ?? 'Listen';
  const subtitle = live
    ? `${status.mic ? 'mic + system audio' : 'system audio only'} · ${engine}`
    : `${sessionDateLabel(viewing.startedAt)}${viewing.stt ? ` · ${viewing.stt}` : ''}`;

  // Document scroll: follow live output while pinned; scrolling up
  // releases the pin and offers a way back to the live edge.
  const scrollRef = useRef<HTMLDivElement>(null);
  const [pinned, setPinned] = useState(true);
  const handleScroll = () => {
    const el = scrollRef.current;
    if (el) setPinned(el.scrollHeight - el.scrollTop - el.clientHeight < 48);
  };
  useEffect(() => {
    const el = scrollRef.current;
    if (el && pinned) el.scrollTop = el.scrollHeight;
  }, [blocks, summary, pinned]);

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      <ListenHeader
        onBack={onBack}
        title={title}
        subtitle={subtitle}
        badge={badge}
        listening={live && listening}
        paused={live && paused}
        onPause={() => void listenPause().catch(() => {})}
        onResume={() => void listenResume().catch(() => {})}
        onStop={stop}
      />
      {live && error && (
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
      {live && listening && !status.mic && !error && (
        <div className='border-b border-border px-3 py-2 text-xs text-muted-foreground'>
          Microphone unavailable; system audio only.
        </div>
      )}
      <SpeakerFilter
        speakers={speakers}
        active={filterKey}
        count={shown.length}
        elapsed={startedAt == null ? null : elapsedLabel(elapsed)}
        copied={copiedAll}
        onPick={setFilterKey}
        onCopy={copyAll}
      />
      <div className='relative min-h-0 flex-1'>
        <div
          ref={scrollRef}
          onScroll={handleScroll}
          data-card-scroll
          className='h-full overflow-y-auto px-3.5 py-3 text-[13px] leading-[1.6]'>
          <div
            data-card-content
            className='flow-root'>
            <TranscriptBlocks
              blocks={shown}
              startedAt={startedAt}
            />
            {turns.length === 0 && !summary && (!error || !live) && (
              <EmptyState
                icon={CaptionsIcon}
                title='No Transcript Yet'
                description={
                  live
                    ? listening
                      ? 'Speak naturally — your transcript will appear here.'
                      : 'Start listening to capture a conversation.'
                    : 'No transcript captured.'
                }
              />
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
      <SummaryStrip summary={summary} />
    </div>
  );
};
