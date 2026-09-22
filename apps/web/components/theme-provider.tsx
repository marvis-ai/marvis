'use client';

import { ThemeProvider as NextThemesProvider } from 'next-themes';
import type { ReactNode } from 'react';

/* Interface shots can run in the app's real dark theme (design.md §6).
 * next-themes owns persistence and the pre-hydration script; it only writes
 * to <html>, so globals.css scopes every dark rule as
 * `[data-theme='dark'] .shots …` — the attribute sits on the root but still
 * re-themes the shots alone. storageKey keeps the old marvis-iface-theme
 * value so saved preferences carry over; enableSystem stays off because the
 * toggle offers exactly light/dark. */
export const ThemeProvider = ({ children }: { children: ReactNode }) => (
  <NextThemesProvider
    attribute='data-theme'
    defaultTheme='light'
    enableSystem={false}
    storageKey='marvis-iface-theme'>
    {children}
  </NextThemesProvider>
);
