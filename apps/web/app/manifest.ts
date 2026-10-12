import type { MetadataRoute } from 'next';

const manifest = (): MetadataRoute.Manifest => ({
  name: 'Marvis AI — Private Desktop AI Copilot',
  short_name: 'Marvis',
  description:
    'Private personal AI assistant for macOS, Windows, and Linux that runs locally on your machine.',
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
