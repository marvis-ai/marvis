import { defineConfig } from '@neon/config/v1';

export default defineConfig({
  // Declare your Neon services here
  auth: false,
  functions: {
    waitlist: {
      name: 'waitlist',
      source: './functions/waitlist.ts',
      env: {
        FROM_EMAIL: 'Marvis AI <notification@updates.getmarvis.com>',
        // OMIT the key entirely when unset: defineConfig throws on
        // `undefined`, and coercing to '' would upload an empty string that
        // deletes the live key. (per .agents/skills/neon-functions)
        ...(process.env.RESEND_API_KEY
          ? { RESEND_API_KEY: process.env.RESEND_API_KEY }
          : {}),
      },
    },
  },
  // Branch policy: per-branch tuning
  branch: (branch) => {
    if (branch.isDefault) {
      // Default branch: no overrides, uses project defaults
      return {};
    }
    if (!branch.exists) {
      // New non-default branches: auto-expire
      // Run `neon checkout <name>` to create a new branch with these settings
      return { ttl: '7d' };
    }
    // Existing branch: no changes
    return {};
  },
});
