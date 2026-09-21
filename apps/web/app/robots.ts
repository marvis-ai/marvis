import type { MetadataRoute } from 'next';

const robots = (): MetadataRoute.Robots => ({
  rules: {
    userAgent: '*',
    allow: '/',
  },
  sitemap: 'https://getmarvis.com/sitemap.xml',
  host: 'https://getmarvis.com',
});

export default robots;
