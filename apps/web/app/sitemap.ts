import type { MetadataRoute } from 'next';

const sitemap = (): MetadataRoute.Sitemap => [
  {
    url: 'https://getmarvis.com',
    lastModified: new Date('2025-02-01'),
    changeFrequency: 'weekly',
    priority: 1.0,
  },
  {
    url: 'https://getmarvis.com/llms.txt',
    lastModified: new Date('2025-02-01'),
    changeFrequency: 'monthly',
    priority: 0.8,
  },
];

export default sitemap;
