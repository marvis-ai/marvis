import type { MetadataRoute } from 'next';

const robots = (): MetadataRoute.Robots => ({
  rules: [
    {
      userAgent: '*',
      allow: '/',
    },
    {
      userAgent: [
        'Googlebot',
        'Google-Extended',
        'GPTBot',
        'ChatGPT-User',
        'ClaudeBot',
        'PerplexityBot',
        'Applebot',
        'Applebot-Extended',
      ],
      allow: '/',
    },
  ],
  sitemap: 'https://getmarvis.com/sitemap.xml',
});

export default robots;
