/**
 * The Listen card's document model — pure helpers shared by
 * `ListenSection`, `ListenHeader`, `SpeakerFilter`, and
 * `TranscriptBlocks`. No React here so the whole pipeline (turn list →
 * speaker blocks → clipboard text) stays unit-testable.
 */
import {
  differenceInSeconds,
  format,
  formatDistanceToNowStrict,
  isThisYear,
  isToday,
  isYesterday,
} from 'date-fns';
import type { ListenSummaryPayload, ListenTurnPayload } from '@/lib/events';

export type Turn = ListenTurnPayload & { interim?: boolean };

/** The session the card is viewing instead of the live capture — set
 *  after stop (and, later, from History). `endedAt` is null only for a
 *  still-open session. `stt` is the engine label that recorded it —
 *  `sessions.stt`; null on sessions written before the column. */
export interface ListenViewing {
  id: number;
  startedAt: number;
  endedAt: number | null;
  stt: string | null;
}

/* ─── transcript document model ──────────────────────────────────
   Consecutive turns from one speaker identity (channel + diarized
   voice cluster) merge into a block: one header, a paragraph per
   turn. A trailing interim rides the open block. */

export interface TurnIdentity {
  speaker: 'me' | 'them';
  speaker_idx: number | null;
}

/** One chip per displayed identity: an unlabeled mic turn (`me` + null)
 *  joins the "You" cluster (`me:0`) rather than spawning a second
 *  indistinguishable "you" filter. `them` keeps null distinct —
 *  "Speaker" (unlabeled) and "Speaker 1" (cluster 0) name differently. */
export const speakerKey = (turn: TurnIdentity) =>
  `${turn.speaker}:${turn.speaker_idx ?? (turn.speaker === 'me' ? 0 : '')}`;

/** A `me`-channel turn the diarizer clustered to someone other than the
 *  enrolled voice — a guest on the user's mic, shown as "Guest N". */
const isMicGuest = (turn: TurnIdentity) =>
  turn.speaker === 'me' && turn.speaker_idx != null && turn.speaker_idx > 0;

export const speakerName = (turn: TurnIdentity) =>
  turn.speaker === 'me'
    ? isMicGuest(turn)
      ? `Guest ${turn.speaker_idx}`
      : 'You'
    : turn.speaker_idx == null
      ? 'Speaker'
      : `Speaker ${turn.speaker_idx + 1}`;

export const SPEAKER_COLOR_CLASSES = [
  'text-speaker-1',
  'text-speaker-2',
  'text-speaker-3',
  'text-speaker-4',
] as const;

export const speakerColor = (turn: TurnIdentity) =>
  turn.speaker === 'me' && !isMicGuest(turn)
    ? 'text-accent'
    : turn.speaker_idx == null
      ? 'text-fg-2'
      : SPEAKER_COLOR_CLASSES[turn.speaker_idx % SPEAKER_COLOR_CLASSES.length];

export interface TurnBlock {
  key: string;
  name: string;
  color: string;
  ts: number;
  finals: Turn[];
  interim: Turn | null;
}

/** Same-speaker merging stays scannable: a block re-headers once it
 *  spans ~30s of audio, or after a ~15s same-speaker pause — Otter /
 *  Granola-style paragraphing so a long monologue isn't one giant
 *  paragraph under a single stamp. */
const BLOCK_MAX_SPAN_SECS = 30;
const BLOCK_PAUSE_SECS = 15;

/** `ts` of the block's latest activity — the riding interim, else the
 *  last final, else the block's first turn. Turns carry no end stamp,
 *  so inter-turn delta is the pause proxy. */
const lastActivityTs = (b: TurnBlock) =>
  b.interim?.ts ?? b.finals[b.finals.length - 1]?.ts ?? b.ts;

/** The old `blocks` useMemo body as a pure function. */
export const buildBlocks = (turns: Turn[]): TurnBlock[] => {
  const out: TurnBlock[] = [];
  for (const turn of turns) {
    const key = speakerKey(turn);
    let block = out[out.length - 1];
    if (
      !block ||
      block.key !== key ||
      turn.ts - block.ts > BLOCK_MAX_SPAN_SECS ||
      turn.ts - lastActivityTs(block) > BLOCK_PAUSE_SECS
    ) {
      // A newer turn supersedes whatever interim rode the previous
      // block — left behind it renders as stale dimmed text
      // mid-document. An interim-only block would become a bare
      // header over an empty paragraph, so it's dropped outright.
      if (block) {
        if (block.finals.length === 0) out.pop();
        else block.interim = null;
      }
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
};

/** Wall-clock `HH:MM` — the per-block stamp fallback when a session
 *  start is unavailable (e.g. old sessions in viewing mode). */
export const timeLabel = (ts: number) => format(ts * 1000, 'HH:mm');

/** `m:ss`, uncapped minutes (mockup's 107:36). */
export const elapsedLabel = (secs: number) => {
  const s = Math.max(0, Math.floor(secs));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
};

/** Adaptive stamp — today `14:32`, yesterday `Yesterday · 14:32`, older
 *  `Sep 26 · 14:32` (`Sep 26, 2025 · 14:32` across years). Viewed-session
 *  header subtitle + chat row stamp. */
export const sessionDateLabel = (ts: number) => {
  const d = new Date(ts * 1000);
  const time = format(d, 'HH:mm');
  if (isToday(d)) return time;
  if (isYesterday(d)) return `Yesterday · ${time}`;
  return `${format(d, isThisYear(d) ? 'MMM d' : 'MMM d, yyyy')} · ${time}`;
};

/** Plain-text block content (finals + interim) — for the clipboard;
 *  the rendered paragraph keeps the dimmed interim + caret markup. */
export const blockText = (b: TurnBlock) =>
  b.finals.map((t) => t.text).join(' ') +
  (b.interim ? (b.finals.length ? ' ' : '') + b.interim.text : '');

export const blockCopyText = (b: TurnBlock) => `${b.name}: ${blockText(b)}`;

/** Whole-document copy: `[m:ss] Name: text` lines + a summary tail. */
export const transcriptCopyText = (
  blocks: TurnBlock[],
  startedAt: number | null,
  summary: ListenSummaryPayload | null,
) => {
  const lines = blocks.map((b) => {
    const stamp =
      startedAt != null ? elapsedLabel(b.ts - startedAt) : timeLabel(b.ts);
    return `[${stamp}] ${b.name}: ${blockText(b)}`;
  });
  if (summary) {
    lines.push(
      '---',
      `TLDR: ${summary.tldr}`,
      ...summary.bullets.map((b) => `- ${b}`),
    );
  }
  return lines.join('\n');
};

export const relTime = (ts: number) => {
  const secs = differenceInSeconds(Date.now(), ts * 1000);
  if (secs < 60) return 'now';
  if (secs < 86400)
    return formatDistanceToNowStrict(ts * 1000, { addSuffix: true });
  return format(ts * 1000, isThisYear(ts * 1000) ? 'MMM d' : 'MMM d, yyyy');
};

/** Markdown document — the export/copy counterpart of
 *  `transcriptCopyText`: same block model and the same stamp rule
 *  (elapsed off `startedAt`, wall-clock without), richer shape.
 *  Sections degrade — a missing summary or an empty transcript drops
 *  its heading rather than rendering an empty shell. */
export const transcriptMarkdown = (
  blocks: TurnBlock[],
  meta: {
    title?: string | null;
    startedAt: number | null;
    stt?: string | null;
  },
  summary: ListenSummaryPayload | null,
): string => {
  const metaLine = [
    meta.startedAt != null
      ? format(meta.startedAt * 1000, 'MMM d, yyyy · HH:mm')
      : null,
    meta.stt,
  ]
    .filter(Boolean)
    .join(' · ');
  const out: string[] = [`# ${meta.title?.trim() || 'Listen session'}`];
  if (metaLine) out.push('', `_${metaLine}_`);
  if (summary) {
    out.push('', '## Summary', '', summary.tldr, '');
    out.push(...summary.bullets.map((b) => `- ${b}`));
    if (summary.follow_ups.length) {
      out.push('', '### Follow-ups', '');
      out.push(...summary.follow_ups.map((f) => `- ${f}`));
    }
  }
  const finalBlocks = blocks.filter((b) => b.finals.length > 0);
  if (finalBlocks.length) {
    out.push('', '## Transcript', '');
    for (const b of finalBlocks) {
      const stamp =
        meta.startedAt != null
          ? elapsedLabel(b.ts - meta.startedAt)
          : timeLabel(b.ts);
      // Finals only — a riding interim isn't in the exported document.
      out.push(
        `- **[${stamp}] ${b.name}:** ${b.finals.map((t) => t.text).join(' ')}`,
      );
    }
  }
  out.push('');
  return out.join('\n');
};

/** `marvis-{slug}-YYYYMMDD-HHmm.md` — slug off the doc title (`listen`
 *  fallback); the stamp comes from `startedAt`, else now. */
export const exportFileName = (
  topic: string | null | undefined,
  startedAt: number | null,
): string => {
  const slug = (topic ?? '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40)
    .replace(/-+$/g, '');
  const stamp = format(
    (startedAt ?? Date.now() / 1000) * 1000,
    'yyyyMMdd-HHmm',
  );
  return `marvis-${slug || 'listen'}-${stamp}.md`;
};
