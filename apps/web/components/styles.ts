import type { CSSProperties } from 'react';

/* Inline-style consts shared across the landing sections — keeps repeated
 * values (section stack gap, split-grid alignment, muted body copy) in one
 * place instead of duplicating the literals per file. */

export const sectionStack: CSSProperties = { gap: '56px' };

export const splitGrid: CSSProperties = { alignItems: 'start', gap: '64px' };

export const leadTop: CSSProperties = { marginTop: '20px' };

export const mutedBody: CSSProperties = {
  margin: 0,
  color: 'var(--muted)',
  fontSize: '15px',
};

export const surfaceBg: CSSProperties = { background: 'var(--surface)' };
