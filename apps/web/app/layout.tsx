import type { Metadata } from 'next';
import { Galada, Inter, Outfit } from 'next/font/google';
import type { ReactNode } from 'react';
import './globals.css';

const inter = Inter({
  variable: '--font-inter',
  subsets: ['latin'],
});

const galada = Galada({
  variable: '--font-galada',
  weight: '400',
  subsets: ['latin'],
});

const outfit = Outfit({
  variable: '--font-outfit',
  subsets: ['latin'],
});

export const metadata: Metadata = {
  title: 'Marvis — a private AI that floats above your desktop',
  description:
    'Marvis is a small translucent bar that floats above your workspace. It sees your screen — only with your permission — and streams answers into an overlay panel, while your keys, history, and screen data never leave your machine.',
};

const RootLayout = ({ children }: { children: ReactNode }) => {
  return (
    <html
      lang='en'
      className={`${inter.variable} ${galada.variable} ${outfit.variable}`}>
      <body>{children}</body>
    </html>
  );
};

export default RootLayout;
