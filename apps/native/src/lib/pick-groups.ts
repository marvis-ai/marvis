import type { PickCandidate } from './commands';

export interface PickGroups {
  screens: PickCandidate[];
  windows: PickCandidate[];
  apps: PickCandidate[];
}

/** Section the flat candidate list — backend order is already
 *  displays → windows → apps; grouping just partitions. */
export const groupCandidates = (cs: PickCandidate[]): PickGroups => ({
  screens: cs.filter((c) => c.kind === 'display'),
  windows: cs.filter((c) => c.kind === 'window'),
  apps: cs.filter((c) => c.kind === 'app'),
});
