# Marvis Web

The marketing site for [Marvis](https://getmarvis.com) — a single-page
landing site for the desktop app. Part of the Marvis monorepo
(`@marvis/web`); the desktop app lives in [`../native`](../native/README.md).

## Stack

| Layer | Technology |
| --- | --- |
| Framework | Next.js 16 (App Router), React 19, TypeScript |
| Styling | Tailwind CSS 4, `tw-animate-css`, `@marvis/ui` (workspace) |
| Fonts | Inter, Galada, Outfit via `next/font/google` |
| Theming | `next-themes` (light/dark) |
| Analytics | Google Tag Manager via `@next/third-parties` |
| Deployment | Cloudflare Workers via `@opennextjs/cloudflare` (Wrangler) |

## Structure

```text
app/
  layout.tsx          # root layout: fonts, metadata/OG/Twitter, GTM, ThemeProvider
  page.tsx            # the landing page (server component — section composition only)
  globals.css         # Tailwind + design tokens
  robots.ts           # robots.txt
  sitemap.ts          # sitemap.xml
components/
  top-nav.tsx         # sticky site nav
  hero.tsx            # headline + primary CTA
  features.tsx        # feature grid
  privacy.tsx         # privacy section
  interface-section.tsx
  hotkeys.tsx         # hotkey table
  providers.tsx       # supported LLM/STT providers
  download-cta.tsx    # download / waitlist call-to-action
  waitlist-dialog.tsx # waitlist signup — posts to /api/waitlist
  site-footer.tsx
  theme-provider.tsx  # next-themes wrapper
  icons.tsx  mv.tsx  pre-lines.tsx  styles.ts   # shared visuals/helpers
next.config.ts        # /api/waitlist → WAITLIST_API_URL rewrite (functions/ service)
open-next.config.ts   # OpenNext build config
wrangler.jsonc        # Cloudflare Worker: marvis-web, assets + nodejs_compat
public/               # static assets
```

The waitlist backend is the Hono/Neon function in
[`../../functions`](../../functions) (SQL in [`../../db`](../../db)); the
site reaches it through the `/api/waitlist` rewrite.

## Scripts

```bash
bun run dev           # next dev on :4010
bun run build         # next build
bun run lint          # eslint
bun run check-types   # tsc --noEmit
bun run preview       # opennextjs-cloudflare build + preview
bun run deploy        # opennextjs-cloudflare build + deploy
bun run cf-typegen    # regenerate cloudflare-env.d.ts from wrangler types
```

## Conventions

- `page.tsx` stays a server component — client interactivity goes in
  dedicated `"use client"` children.
- Arrow-function components, named exports, `Icon`-suffixed icon names
  from `@marvis/ui`.
