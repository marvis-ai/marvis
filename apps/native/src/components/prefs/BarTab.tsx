/**
 * Bar — where the floating capsule lives. It drags anywhere with the
 * pointer and remembers the resting place across restarts (the backend
 * persists `window.bar_x/y` on every move); this row is the quick snap —
 * pick an edge and the bar animates there.
 */
import { useEffect, useState } from 'react';
import {
  windowBarEdge,
  windowRecenter,
  windowSnapEdge,
} from '../../lib/commands';
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

  // Re-center always lands top-center on the primary display — the
  // nearest edge is 'top', so the picker follows without a re-read.
  const recenter = () => {
    setEdge('top');
    void windowRecenter().catch(() => {});
  };

  return (
    <>
      <h2>Bar</h2>
      <p className='sub'>
        Drag the bar anywhere — it stays where you leave it, across restarts.
      </p>

      <div className='prf-rows'>
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
          sub='Back to the default spot — centered under the menu bar.'
          last>
          <button
            type='button'
            className='mv-btn mv-btn-outline'
            onClick={recenter}>
            Re-center
          </button>
        </PrefRow>
      </div>
    </>
  );
};
