/// <reference types="bun-types" />
import { describe, expect, test } from 'bun:test';
import { format } from 'date-fns';
import type { ListenSummaryPayload } from '@/lib/events';
import {
  activeBlockAt,
  audioOffset,
  buildBlocks,
  exportFileName,
  resolveSpeakerNames,
  timeLabel,
  transcriptCopyText,
  transcriptMarkdown,
  type Turn,
} from './model';

const turn = (ts: number, over: Partial<Turn> = {}): Turn => ({
  speaker: 'them',
  speaker_idx: 0,
  audio_start_ms: null,
  text: `t${ts}`,
  ts,
  session_id: 1,
  final: true,
  ...over,
});

describe('buildBlocks', () => {
  test('merges dense same-speaker turns into one block', () => {
    const blocks = buildBlocks([turn(0), turn(4), turn(9), turn(14)]);
    expect(blocks).toHaveLength(1);
    expect(blocks[0]!.finals).toHaveLength(4);
    expect(blocks[0]!.ts).toBe(0);
  });

  test('re-headers when the block spans over 30s', () => {
    const blocks = buildBlocks([turn(0), turn(10), turn(20), turn(31)]);
    expect(blocks).toHaveLength(2);
    expect(blocks[1]!.ts).toBe(31);
    // The 30s boundary itself still merges — the cap is exclusive.
    expect(buildBlocks([turn(0), turn(10), turn(20), turn(30)])).toHaveLength(
      1,
    );
  });

  test('re-headers after a 15s same-speaker pause under the cap', () => {
    const blocks = buildBlocks([turn(0), turn(5), turn(25)]);
    expect(blocks).toHaveLength(2);
    expect(blocks[1]!.ts).toBe(25);
    // A 10s gap does not split.
    expect(buildBlocks([turn(0), turn(10)])).toHaveLength(1);
  });

  test('still splits on speaker change', () => {
    const blocks = buildBlocks([
      turn(0),
      turn(3, { speaker: 'me', speaker_idx: null }),
    ]);
    expect(blocks).toHaveLength(2);
    expect(blocks[1]!.name).toBe('You');
  });

  test('a riding interim keeps the block open and survives the checks', () => {
    const blocks = buildBlocks([
      turn(0),
      turn(6, { interim: true, final: false }),
      turn(8),
    ]);
    expect(blocks).toHaveLength(1);
    expect(blocks[0]!.interim).toBeNull();
    expect(blocks[0]!.finals.map((t) => t.ts)).toEqual([0, 8]);
  });

  test('a pause measured from the interim splits the next final', () => {
    // Long silence mid-thought: interim at t=5, next real turn at t=30.
    const blocks = buildBlocks([
      turn(0),
      turn(5, { interim: true, final: false }),
      turn(30),
    ]);
    expect(blocks).toHaveLength(2);
    expect(blocks[1]!.ts).toBe(30);
  });

  test('a split drops the stale interim off the previous block', () => {
    // The t=5 interim never finalized — once t=30 splits it must not
    // keep rendering as dimmed text on block 0.
    const blocks = buildBlocks([
      turn(0),
      turn(5, { interim: true, final: false }),
      turn(30),
    ]);
    expect(blocks[0]!.interim).toBeNull();
  });

  test('an interim-only block is reused by the turn that splits it', () => {
    // The interim at t=5 never finalized — clearing it would leave a
    // bare header over an empty paragraph, so the new turn takes the
    // block instead.
    const blocks = buildBlocks([
      turn(5, { interim: true, final: false }),
      turn(30),
    ]);
    expect(blocks).toHaveLength(1);
    expect(blocks[0]!.ts).toBe(30);
  });

  test('an interim-only block is dropped on speaker change', () => {
    const blocks = buildBlocks([
      turn(0),
      turn(5, {
        speaker: 'me',
        speaker_idx: null,
        interim: true,
        final: false,
      }),
      turn(10),
    ]);
    expect(blocks).toHaveLength(2);
    expect(blocks[1]!.ts).toBe(10);
  });
});

describe('resolveSpeakerNames', () => {
  test('assigns one You identity and unique anonymous names by first sighting', () => {
    const turns = [
      turn(0, { speaker: 'them', speaker_idx: 0 }),
      turn(1, { speaker: 'me', speaker_idx: null }),
      turn(2, { speaker: 'me', speaker_idx: 1 }),
      turn(3, { speaker: 'them', speaker_idx: 1 }),
    ];

    expect([...resolveSpeakerNames(turns)]).toEqual([
      ['them:0', 'Speaker 1'],
      ['me:0', 'You'],
      ['me:1', 'Speaker 2'],
      ['them:1', 'Speaker 3'],
    ]);
  });

  test('overrides one identity without changing its stable filter key', () => {
    const blocks = buildBlocks(
      [
        turn(0, { speaker: 'them', speaker_idx: 0 }),
        turn(1, { speaker: 'me', speaker_idx: 1 }),
        turn(2, { speaker: 'them', speaker_idx: 0 }),
      ],
      new Map([['them:0', 'Alice']]),
    );

    expect(blocks.map((block) => [block.key, block.name])).toEqual([
      ['them:0', 'Alice'],
      ['me:1', 'Speaker 2'],
      ['them:0', 'Alice'],
    ]);
    expect(blocks[0]!.canRename).toBe(true);
    expect(blocks[1]!.canRename).toBe(true);
  });

  test('You and an unlabelled system turn are not renameable', () => {
    const blocks = buildBlocks([
      turn(0, { speaker: 'me', speaker_idx: null }),
      turn(1, { speaker: 'them', speaker_idx: null }),
    ]);

    expect(blocks.map((block) => [block.name, block.canRename])).toEqual([
      ['You', false],
      ['Speaker', false],
    ]);
  });

  test('copy and Markdown export use the resolved names', () => {
    const blocks = buildBlocks(
      [turn(1_700_000_042, { speaker: 'them', speaker_idx: 0 })],
      new Map([['them:0', 'Alice']]),
    );

    expect(transcriptCopyText(blocks, 1_700_000_000, null)).toContain(
      'Alice: t1700000042',
    );
    expect(
      transcriptMarkdown(blocks, { startedAt: 1_700_000_000 }, null),
    ).toContain('Alice:** t1700000042');
  });
});

describe('transcriptMarkdown', () => {
  const summary: ListenSummaryPayload = {
    tldr: 'Talked about the roadmap.',
    bullets: ['Phase 1 continues', 'Export ships'],
    follow_ups: ['Write the plan'],
    topic: 'Weekly standup',
  };
  const meta = {
    title: 'Weekly standup',
    startedAt: 1_700_000_000,
    stt: 'whisper tiny',
  };

  test('title, meta line, summary, and transcript sections', () => {
    const md = transcriptMarkdown(
      buildBlocks([
        turn(1_700_000_042, {
          text: 'first turn',
          speaker: 'me',
          speaker_idx: null,
        }),
        turn(1_700_000_134, { text: 'reply' }),
      ]),
      meta,
      summary,
    );
    expect(md).toBe(
      [
        '# Weekly standup',
        '',
        `_${format(
          1_700_000_000 * 1000,
          'MMM d, yyyy · HH:mm',
        )} · whisper tiny_`,
        '',
        '## Summary',
        '',
        'Talked about the roadmap.',
        '',
        '- Phase 1 continues',
        '- Export ships',
        '',
        '### Follow-ups',
        '',
        '- Write the plan',
        '',
        '## Transcript',
        '',
        '- **[0:42] You:** first turn',
        '- **[2:14] Speaker 1:** reply',
        '',
      ].join('\n'),
    );
  });

  test('degrades: no summary, no stt, wall-clock stamps without startedAt', () => {
    const md = transcriptMarkdown(
      buildBlocks([
        turn(1_700_000_042, {
          text: 'hi',
          speaker: 'me',
          speaker_idx: null,
        }),
      ]),
      { startedAt: null },
      null,
    );
    expect(md).not.toContain('## Summary');
    expect(md).not.toContain('Follow-ups');
    expect(md).toContain('# Listen session');
    expect(md).toContain(`- **[${timeLabel(1_700_000_042)}] You:** hi`);
  });

  test('no turns drops the Transcript section; empty follow_ups drops its heading', () => {
    const md = transcriptMarkdown([], meta, { ...summary, follow_ups: [] });
    expect(md).not.toContain('## Transcript');
    expect(md).not.toContain('Follow-ups');
    expect(md).toContain('## Summary');
  });

  test('empty bullets do not leave a double blank line', () => {
    const md = transcriptMarkdown([], meta, {
      ...summary,
      bullets: [],
      follow_ups: [],
    });
    expect(md).toContain('## Summary');
    expect(md).not.toContain('\n\n\n');
  });

  test('a riding interim is excluded — export is finals only', () => {
    const md = transcriptMarkdown(
      buildBlocks([
        turn(1_700_000_042, { text: 'done' }),
        turn(1_700_000_050, {
          text: 'draft',
          interim: true,
          final: false,
        }),
      ]),
      meta,
      null,
    );
    expect(md).toContain('done');
    expect(md).not.toContain('draft');
  });

  test('an interim-only block does not create an empty transcript bullet', () => {
    const md = transcriptMarkdown(
      buildBlocks([
        turn(1_700_000_050, {
          text: 'draft',
          interim: true,
          final: false,
        }),
      ]),
      meta,
      null,
    );
    expect(md).not.toContain('## Transcript');
    expect(md).not.toContain('draft');
  });
});

describe('exportFileName', () => {
  test('slugifies the topic, stamps from startedAt', () => {
    const name = exportFileName('Weekly Standup!', 1_700_000_000);
    expect(name).toBe(
      `marvis-weekly-standup-${format(
        1_700_000_000 * 1000,
        'yyyyMMdd-HHmm',
      )}.md`,
    );
  });

  test('fallback slug + long topics cap at 40 chars', () => {
    expect(exportFileName(null, null)).toMatch(
      /^marvis-listen-\d{8}-\d{4}\.md$/,
    );
    const long = exportFileName('a'.repeat(60), null);
    expect(long).toMatch(/^marvis-a{40}-\d{8}-\d{4}\.md$/);
  });
});

test('audioOffset clamps a block before the session start', () => {
  const [block] = buildBlocks([turn(90, { speaker: 'me', speaker_idx: null })]);
  expect(audioOffset(block!, 100)).toBe(0);
  expect(audioOffset(block!, 42)).toBe(48);
});

test('activeBlockAt resolves blocks and leaves gaps inactive', () => {
  const blocks = buildBlocks([
    turn(100, { speaker: 'me', speaker_idx: null }),
    turn(110, { speaker: 'them', speaker_idx: 0 }),
  ]);
  expect(activeBlockAt(blocks, 100, 0)).toBe('me:0-100');
  expect(activeBlockAt(blocks, 100, 9)).toBe('me:0-100');
  expect(activeBlockAt(blocks, 100, 10)).toBe('them:0-110');
  expect(activeBlockAt(blocks, 100, -1)).toBe(null);
});

test('exportFileName supports a WAV extension', () => {
  expect(exportFileName('Weekly Standup', 1_700_000_000, 'wav')).toMatch(
    /^marvis-weekly-standup-\d{8}-\d{4}\.wav$/,
  );
});

test('capture positions drive seeking and highlights despite persistence delay and pauses', () => {
  const blocks = buildBlocks([
    turn(200, { audio_start_ms: 1250, speaker: 'me' }),
    turn(400, { audio_start_ms: 3000 }),
  ]);
  expect(audioOffset(blocks[0]!, 100)).toBe(1.25);
  expect(audioOffset(blocks[1]!, 100)).toBe(3);
  expect(activeBlockAt(blocks, 100, 1)).toBeNull();
  expect(activeBlockAt(blocks, 100, 1.25)).toBe('me:0-200');
  expect(activeBlockAt(blocks, 100, 3)).toBe('them:0-400');
  expect(activeBlockAt([...blocks].reverse(), null, 3)).toBe('them:0-400');
});
