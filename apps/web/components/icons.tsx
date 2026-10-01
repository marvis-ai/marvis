import type { SVGProps } from 'react';

type IconProps = SVGProps<SVGSVGElement>;

/* The 48px mark — eye over text lines (design.md §1). Fills reference the
 * page tokens so it tracks the design system. */
export const Mark = (props: IconProps) => (
  <svg
    viewBox='0 0 48 48'
    role='img'
    aria-label='Marvis mark'
    {...props}>
    <rect
      x='3'
      y='3'
      width='42'
      height='42'
      rx='10'
      fill='var(--fg-2)'
    />
    <circle
      cx='24'
      cy='17.5'
      r='7.5'
      fill='var(--accent)'
    />
    <circle
      cx='24'
      cy='17.5'
      r='2.9'
      fill='var(--bg)'
    />
    <rect
      x='14'
      y='30'
      width='20'
      height='2.6'
      rx='1.3'
      fill='var(--surface)'
    />
    <rect
      x='17.5'
      y='34.8'
      width='13'
      height='2.6'
      rx='1.3'
      fill='var(--meta)'
    />
  </svg>
);

/* The app's own logo glyph used inside the bar (Bar.tsx). */
export const MvLogo = (props: IconProps) => (
  <svg
    className='mv-logo'
    viewBox='0 0 16 16'
    aria-hidden='true'
    {...props}>
    <rect
      width='16'
      height='16'
      rx='5'
      fill='currentColor'
    />
    <path
      d='M4.5 11.5v-7l3.5 4.4 3.5-4.4v7'
      fill='none'
      stroke='var(--mv-card)'
      strokeWidth='1.5'
      strokeLinecap='round'
      strokeLinejoin='round'
    />
  </svg>
);

/* The GitHub mark (octicons mark-github) — fills currentColor so it
 * tracks the button's text color. Lucide dropped brand icons, so it
 * lives here with the other brand marks. */
export const GithubMark = (props: IconProps) => (
  <svg
    viewBox='0 0 16 16'
    aria-hidden='true'
    fill='currentColor'
    {...props}>
    <path
      fillRule='evenodd'
      d='M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27s1.36.09 2 .27c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z'
    />
  </svg>
);

/* The macOS app icon — last tile in the dock shot (assets/marvis-icon.svg). */
export const MarvisAppIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 512 512'
    role='img'
    aria-label='Marvis'
    {...props}>
    <defs>
      <linearGradient
        id='marvis-tile'
        x1='0'
        y1='0'
        x2='0'
        y2='1'>
        <stop
          offset='0'
          stopColor='#3a3937'
        />
        <stop
          offset='1'
          stopColor='#242321'
        />
      </linearGradient>
    </defs>
    <rect
      x='28'
      y='28'
      width='456'
      height='456'
      rx='104'
      fill='url(#marvis-tile)'
    />
    <rect
      x='29'
      y='29'
      width='454'
      height='454'
      rx='103'
      fill='none'
      stroke='#ffffff'
      strokeOpacity='0.14'
      strokeWidth='2'
    />
    <circle
      cx='256'
      cy='192'
      r='84'
      fill='var(--accent)'
    />
    <circle
      cx='256'
      cy='192'
      r='31'
      fill='var(--bg)'
    />
    <rect
      x='156'
      y='314'
      width='200'
      height='30'
      rx='15'
      fill='var(--surface)'
    />
    <rect
      x='196'
      y='366'
      width='120'
      height='30'
      rx='15'
      fill='var(--meta)'
    />
  </svg>
);
