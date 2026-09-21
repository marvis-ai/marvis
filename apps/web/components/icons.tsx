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

export const CameraIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 24 24'
    fill='none'
    stroke='currentColor'
    strokeWidth='2'
    strokeLinecap='round'
    strokeLinejoin='round'
    aria-hidden='true'
    {...props}>
    <path d='M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z' />
    <circle
      cx='12'
      cy='13'
      r='4'
    />
  </svg>
);

export const MicIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 24 24'
    fill='none'
    stroke='currentColor'
    strokeWidth='2'
    strokeLinecap='round'
    strokeLinejoin='round'
    aria-hidden='true'
    {...props}>
    <path d='M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z' />
    <path d='M19 10v2a7 7 0 0 1-14 0v-2' />
    <line
      x1='12'
      x2='12'
      y1='19'
      y2='22'
    />
  </svg>
);

export const GearIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 24 24'
    fill='none'
    stroke='currentColor'
    strokeWidth='2'
    strokeLinecap='round'
    strokeLinejoin='round'
    aria-hidden='true'
    {...props}>
    <path d='M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z' />
    <circle
      cx='12'
      cy='12'
      r='3'
    />
  </svg>
);

export const CloseIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 24 24'
    fill='none'
    stroke='currentColor'
    strokeWidth='2'
    strokeLinecap='round'
    aria-hidden='true'
    {...props}>
    <path d='M18 6 6 18' />
    <path d='m6 6 12 12' />
  </svg>
);

export const ShieldIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 24 24'
    fill='none'
    stroke='currentColor'
    strokeWidth='2'
    strokeLinecap='round'
    strokeLinejoin='round'
    aria-hidden='true'
    {...props}>
    <path d='M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z' />
    <path d='M12 8v4' />
    <path d='M12 16h.01' />
  </svg>
);

export const SunIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 24 24'
    fill='none'
    stroke='currentColor'
    strokeWidth='2'
    strokeLinecap='round'
    strokeLinejoin='round'
    aria-hidden='true'
    {...props}>
    <circle
      cx='12'
      cy='12'
      r='4'
    />
    <path d='M12 2v2' />
    <path d='M12 20v2' />
    <path d='m4.93 4.93 1.41 1.41' />
    <path d='m17.66 17.66 1.41 1.41' />
    <path d='M2 12h2' />
    <path d='M20 12h2' />
    <path d='m6.34 17.66-1.41 1.41' />
    <path d='m19.07 4.93-1.41 1.41' />
  </svg>
);

export const MoonIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 24 24'
    fill='none'
    stroke='currentColor'
    strokeWidth='2'
    strokeLinecap='round'
    strokeLinejoin='round'
    aria-hidden='true'
    {...props}>
    <path d='M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z' />
  </svg>
);

export const MonitorIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 24 24'
    fill='none'
    stroke='currentColor'
    strokeWidth='1.6'
    strokeLinecap='round'
    aria-hidden='true'
    {...props}>
    <rect
      x='3'
      y='4'
      width='18'
      height='13'
      rx='2'
    />
    <path d='M8 21h8M12 17v4' />
  </svg>
);

export const KeyIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 24 24'
    fill='none'
    stroke='currentColor'
    strokeWidth='1.6'
    strokeLinecap='round'
    aria-hidden='true'
    {...props}>
    <circle
      cx='8'
      cy='12'
      r='4'
    />
    <path d='M12 12h9M18 12v4M15 12v3' />
  </svg>
);

export const CommandIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 24 24'
    fill='none'
    stroke='currentColor'
    strokeWidth='1.6'
    strokeLinecap='round'
    strokeLinejoin='round'
    aria-hidden='true'
    {...props}>
    <path d='M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3' />
  </svg>
);

export const LockIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 24 24'
    fill='none'
    stroke='currentColor'
    strokeWidth='1.6'
    strokeLinecap='round'
    aria-hidden='true'
    {...props}>
    <rect
      x='5'
      y='10'
      width='14'
      height='10'
      rx='2'
    />
    <path d='M8 10V7a4 4 0 0 1 8 0v3' />
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
