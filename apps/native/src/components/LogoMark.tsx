/**
 * Marvis "M" glyph — a 16×16 mark drawn inline so the transparent bar
 * window doesn't need an image asset.
 */
export function LogoMark({ className }: { className?: string }) {
  return (
    <svg
      viewBox='0 0 16 16'
      className={className}
      aria-hidden='true'
      data-tauri-drag-region>
      <rect
        width='16'
        height='16'
        rx='5'
        fill='currentColor'
      />
      <path
        d='M4.5 11.5v-7l3.5 4.4 3.5-4.4v7'
        fill='none'
        style={{ stroke: 'var(--card)' }}
        strokeWidth='1.5'
        strokeLinecap='round'
        strokeLinejoin='round'
      />
    </svg>
  );
}
