import { readFileSync } from 'node:fs';
import type { NextConfig } from 'next';

const { version: appVersion } = JSON.parse(
  readFileSync(new URL('../../package.json', import.meta.url), 'utf-8'),
) as { version: string };

const nextConfig: NextConfig = {
  env: {
    NEXT_PUBLIC_APP_VERSION: appVersion,
  },
};

export default nextConfig;
