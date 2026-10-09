/**
 * Bar — where the floating capsule lives. It drags anywhere with the
 * pointer and remembers the resting place across restarts (the backend
 * persists `window.bar_x/y` on every move); this row is the quick snap —
 * pick an edge and the bar animates there.
 */
import { useEffect, useState } from 'react';
import { windowBarEdge, windowRecenter, windowSnapEdge } from '@/lib/commands';
import { BTN_LG, BTN_OUTLINE, H2, PRF_ROWS, SUB, cn } from '@/lib/classes';
import { PrefRow, Seg } from './bits';

type Edge = 'top' | 'bottom' | 'left' | 'right';
const EDGES: Edge[] = ['top', 'bottom', 'left', 'right'];
const asEdge = (v: string): Edge =>
  (EDGES as string[]).includes(v) ? (v as Edge) : 'top';

export const BarTab = () => {
  const [edge, setEdge] = useState<Edge>('top');

  // Seed from the bar's live position — a just-finished drag reads
  // through `window_bar_edge` (it refreshes the rect from the OS first).
  useEffect(() => {
    void windowBarEdge()
      .then((e) => setEdge(asEdge(e)))
      .catch(() => {});
  }, []);

  const snap = (e: Edge) => {
    setEdge(e);
    void windowSnapEdge(e).catch(() => {});
  };

  // Re-center lands at the work-area center — equidistant, so the
  // picker re-reads the real edge instead of assuming one.
  const recenter = () => {
    void windowRecenter()
      .then(() => windowBarEdge())
      .then((e) => setEdge(asEdge(e)))
      .catch(() => {});
  };

  return (
    <>
      <h2 className={H2}>Bar</h2>
      <p className={SUB}>
        Drag the bar anywhere — it stays where you leave it, across restarts.
      </p>

      <div className={PRF_ROWS}>
        <PrefRow
          label='Snap to edge'
          sub='Hug a work-area edge with a 12 px margin.'>
          <Seg
            ariaLabel='Snap bar to edge'
            value={edge}
            onChange={(v) => snap(asEdge(v))}
            options={[
              { id: 'top', label: 'Top' },
              { id: 'bottom', label: 'Bottom' },
              { id: 'left', label: 'Left' },
              { id: 'right', label: 'Right' },
            ]}
          />
        </PrefRow>
        <PrefRow
          label='Re-center'
          sub='Back to the default spot — the middle of the screen.'
          last>
          <button
            type='button'
            className={cn(BTN_LG, BTN_OUTLINE)}
            onClick={recenter}>
            Re-center
          </button>
        </PrefRow>
      </div>
    </>
  );
};
