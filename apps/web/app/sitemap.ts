import type { MetadataRoute } from 'next';

const sitemap = (): MetadataRoute.Sitemap => [
  {
    url: 'https://getmarvis.com',
    lastModified: new Date(),
    changeFrequency: 'weekly',
    priority: 1.0,
  },
  {
    url: 'https://getmarvis.com/llms.txt',
    lastModified: new Date(),
    changeFrequency: 'monthly',
    priority: 0.8,
  },
];

export default sitemap;
