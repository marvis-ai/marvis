/// <reference types="bun-types" />
import { describe, expect, test } from 'bun:test';
import { buildBlocks, type Turn } from './model';

const turn = (ts: number, over: Partial<Turn> = {}): Turn => ({
  speaker: 'them',
  speaker_idx: 0,
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
});
