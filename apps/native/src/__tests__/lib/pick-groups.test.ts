import { describe, expect, test } from 'bun:test';
import { groupCandidates } from '@/lib/pick-groups';
import type { PickCandidate } from '@/lib/commands';

const c = (id: string, kind: PickCandidate['kind']): PickCandidate => ({
  id,
  kind,
  label: id,
  sub: null,
  w: 100,
  h: 100,
  thumb_of: null,
});

describe('groupCandidates', () => {
  test('partitions by kind preserving order', () => {
    const g = groupCandidates([
      c('d:1', 'display'),
      c('w:9', 'window'),
      c('w:3', 'window'),
      c('a:x', 'app'),
      c('d:2', 'display'),
    ]);
    expect(g.screens.map((c) => c.id)).toEqual(['d:1', 'd:2']);
    expect(g.windows.map((c) => c.id)).toEqual(['w:9', 'w:3']);
    expect(g.apps.map((c) => c.id)).toEqual(['a:x']);
  });

  test('empty input gives empty sections', () => {
    const g = groupCandidates([]);
    expect(g).toEqual({ screens: [], windows: [], apps: [] });
  });
});
