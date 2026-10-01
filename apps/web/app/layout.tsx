import type { ReactNode } from 'react';
import type { Metadata } from 'next';
import { Galada, Inter, Outfit } from 'next/font/google';
import { GoogleTagManager } from '@next/third-parties/google';
import { ThemeProvider } from '@/components/theme-provider';

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

const SITE = {
  name: 'Marvis',
  url: 'https://getmarvis.com',
  title: 'Marvis — a private AI for your desktop on all platforms',
  description:
    'Marvis is a private AI assistant for macOS, Windows, and Linux. A floating bar sees your screen with permission, transcribes your meetings, and answers through the providers you choose — your keys and screen data never leave your device.',
};

const GA_ID = 'G-DRDLX9D2CV';

export const metadata: Metadata = {
  metadataBase: new URL(SITE.url),
  title: SITE.title,
  description: SITE.description,
  keywords: [
    'Marvis',
    'private AI assistant',
    'desktop AI',
    'screen-aware AI',
    'AI assistant macOS',
    'AI assistant Windows',
    'AI assistant Linux',
    'meeting transcription',
    'screen-aware AI',
    'local AI assistant',
    'offline transcription',
    'whisper transcription',
    'bring your own API key',
    'BYOK AI',
    'Ollama desktop app',
    'open source AI assistant',
    'AI overlay',
  ],
  alternates: {
    canonical: '/',
  },
  openGraph: {
    title: SITE.title,
    description: SITE.description,
    url: SITE.url,
    siteName: SITE.name,
    type: 'website',
    locale: 'en_US',
  },
  twitter: {
    card: 'summary_large_image',
    title: SITE.title,
    description: SITE.description,
  },
  robots: {
    index: true,
    follow: true,
  },
};

const jsonLd = {
  '@context': 'https://schema.org',
  '@graph': [
    {
      '@type': 'WebSite',
      '@id': `${SITE.url}/#website`,
      url: SITE.url,
      name: SITE.name,
      description: SITE.description,
    },
    {
      '@type': 'SoftwareApplication',
      '@id': `${SITE.url}/#software`,
      name: SITE.name,
      description: SITE.description,
      operatingSystem: 'macOS, Windows, Linux',
      applicationCategory: 'ProductivityApplication',
      offers: {
        '@type': 'Offer',
        price: '0',
        priceCurrency: 'USD',
      },
    },
  ],
};

const RootLayout = ({ children }: { children: ReactNode }) => {
  return (
    <html
      lang='en'
      suppressHydrationWarning
      className={`${inter.variable} ${galada.variable} ${outfit.variable}`}>
      <body>
        <script
          type='application/ld+json'
          dangerouslySetInnerHTML={{ __html: JSON.stringify(jsonLd) }}
        />
        <ThemeProvider>{children}</ThemeProvider>
        <GoogleTagManager gtmId={GA_ID} />
      </body>
    </html>
  );
};

export default RootLayout;
