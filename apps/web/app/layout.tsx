import type { ReactNode } from 'react';
import type { Metadata, Viewport } from 'next';
import { Galada, Inter, Outfit } from 'next/font/google';
import { GoogleTagManager } from '@next/third-parties/google';
import { ThemeProvider } from '@/components/theme-provider';
import { FAQS } from '@/lib/faq';
import { APP_VERSION, GITHUB_RELEASES, GITHUB_REPO } from '@/lib/constants';

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
  name: 'Marvis AI',
  url: 'https://getmarvis.com',
  title: 'Marvis AI — Privacy-First Desktop AI Copilot',
  description:
    'Free, open-source personal AI assistant for macOS, Windows, and Linux. Floating bar with screen context, local meeting transcription, and zero cloud storage.',
};

const GA_ID = 'G-DRDLX9D2CV';

export const viewport: Viewport = {
  themeColor: '#31302e',
  width: 'device-width',
  initialScale: 1,
};

export const metadata: Metadata = {
  metadataBase: new URL(SITE.url),
  title: {
    default: SITE.title,
    template: '%s | Marvis AI',
  },
  description: SITE.description,
  applicationName: 'Marvis',
  authors: [{ name: 'Marvis AI LLC', url: SITE.url }],
  creator: 'Marvis AI LLC',
  publisher: 'Marvis AI LLC',
  category: 'ProductivityApplication',
  keywords: [
    'Marvis',
    'Marvis AI',
    'private AI assistant',
    'personal AI',
    'desktop AI',
    'screen-aware AI',
    'AI assistant macOS',
    'AI assistant Windows',
    'AI assistant Linux',
    'meeting transcription',
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
  icons: {
    icon: [
      { url: '/favicon.ico', sizes: '48x48' },
      { url: '/icon.svg', type: 'image/svg+xml' },
    ],
    apple: [{ url: '/apple-icon.png', sizes: '180x180', type: 'image/png' }],
  },
  manifest: '/manifest.webmanifest',
  openGraph: {
    title: SITE.title,
    description: SITE.description,
    url: SITE.url,
    siteName: SITE.name,
    type: 'website',
    locale: 'en_US',
    images: [
      {
        url: '/opengraph-image.png',
        width: 1200,
        height: 630,
        alt: 'Marvis — private AI for your screen and your meetings',
      },
    ],
  },
  twitter: {
    card: 'summary_large_image',
    title: SITE.title,
    description: SITE.description,
    creator: '@marvis_ai',
    images: ['/twitter-image.png'],
  },
  robots: {
    index: true,
    follow: true,
    googleBot: {
      index: true,
      follow: true,
      'max-video-preview': -1,
      'max-image-preview': 'large',
      'max-snippet': -1,
    },
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
      publisher: {
        '@id': `${SITE.url}/#organization`,
      },
      inLanguage: 'en-US',
    },
    {
      '@type': 'Organization',
      '@id': `${SITE.url}/#organization`,
      name: 'Marvis AI LLC',
      url: SITE.url,
      logo: `${SITE.url}/logo.png`,
      sameAs: [GITHUB_REPO],
    },
    {
      '@type': 'SoftwareApplication',
      '@id': `${SITE.url}/#software`,
      name: SITE.name,
      description: SITE.description,
      operatingSystem: 'macOS, Windows, Linux',
      applicationCategory: 'ProductivityApplication',
      applicationSubCategory: 'Personal Desktop AI Copilot',
      softwareVersion: APP_VERSION,
      license: 'https://www.apache.org/licenses/LICENSE-2.0',
      downloadUrl: GITHUB_RELEASES,
      screenshot: `${SITE.url}/opengraph-image.png`,
      author: {
        '@id': `${SITE.url}/#organization`,
      },
      publisher: {
        '@id': `${SITE.url}/#organization`,
      },
      featureList: [
        'Screen-aware context with in-memory 60s frame ring',
        'Live meeting transcription with local whisper.cpp and sherpa models',
        'Zero-cloud architecture with local SQLite database in ~/.marvis',
        'Bring Your Own Key (BYOK) support for OpenAI, Anthropic, Gemini, OpenRouter, and Ollama',
        'Native cross-platform desktop overlay for macOS, Windows, and Linux',
      ],
      offers: {
        '@type': 'Offer',
        price: '0',
        priceCurrency: 'USD',
      },
    },
    {
      '@type': 'FAQPage',
      '@id': `${SITE.url}/#faq`,
      mainEntity: FAQS.map((faq) => ({
        '@type': 'Question',
        name: faq.question,
        acceptedAnswer: {
          '@type': 'Answer',
          text: faq.answer,
        },
      })),
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
