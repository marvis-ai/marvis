import type { NextConfig } from 'next';

const nextConfig: NextConfig = {
  async rewrites() {
    return [
      {
        source: '/api/waitlist',
        destination: process.env.WAITLIST_API_URL ?? 'http://localhost:8787/',
      },
    ];
  },
};

export default nextConfig;
