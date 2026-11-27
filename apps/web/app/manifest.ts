import type { MetadataRoute } from 'next';

const manifest = (): MetadataRoute.Manifest => ({
  name: 'Marvis AI — Private, Local-First Personal AI Agent',
  short_name: 'Marvis',
  description:
    'Private, local-first personal AI agent for macOS, Windows, and Linux that runs on your machine.',
  start_url: '/',
  display: 'standalone',
  background_color: '#ffffff',
  theme_color: '#31302e',
  icons: [
    {
      src: '/favicon.ico',
      sizes: '48x48',
      type: 'image/x-icon',
    },
    {
      src: '/icon.svg',
      sizes: 'any',
      type: 'image/svg+xml',
    },
    {
      src: '/apple-icon.png',
      sizes: '180x180',
      type: 'image/png',
    },
  ],
});

export default manifest;
