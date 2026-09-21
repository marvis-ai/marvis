# Marvis Web — Waitlist Capture on Download CTAs

Date: 2026-09-21
Status: Approved design (pending written-spec review)

## Goal

Every "Download" CTA on the landing page (`apps/web`) opens a modal collecting
**name + email** to join the waitlist. Submitting stores the signup in a Neon
Postgres table and sends a notification email via Resend from
`Marvis AI <notification@updates.getmarvis.com>`.

## User decisions

| Question | Decision |
| --- | --- |
| Capture UX | Button opens modal (all three CTAs: top-nav, hero, `#download` section) |
| Form fields | Email + name |
| Resend API key | Not yet — code wires `RESEND_API_KEY` env; key/domain verification is a follow-up user action |
| Browser → function call | Same-origin `POST /api/waitlist` proxied via Next.js `rewrites()` (no CORS, function URL stays server-side) |
| Modal tech | shadcn `Dialog`/`Input`/`Button` from `@marvis/ui`, theme scoped to the dialog |

## Flow

```text
CTA click (nav / hero / #download section)
  → WaitlistDialog opens (Base UI Dialog, shadcn-styled)
  → submit name + email
  → POST /api/waitlist  (same origin)
  → next.config rewrites → Neon Function `waitlist` (WAITLIST_API_URL env)
  → validate → INSERT INTO waitlist ON CONFLICT (email) DO NOTHING
  → new row → Resend send; duplicate → skip send
  → { ok: true } → success state in modal
```

## Backend — Neon Function (repo root)

| File | Contents |
| --- | --- |
| `neon.ts` | `defineConfig` declaring `functions.waitlist` → `source: ./functions/waitlist.ts`, `env: { RESEND_API_KEY: process.env.RESEND_API_KEY, FROM_EMAIL: 'Marvis AI <notification@updates.getmarvis.com>' }`. NOTE: user's pasted snippet used a `preview:` block; the documented form is top-level `functions:` — confirm against `neon config init` scaffold and adjust if needed. |
| `functions/waitlist.ts` | Hono app (default export). `POST /` = signup handler; `GET /` = health `{ ok: true }`. `pg` `Pool` at module scope on injected `DATABASE_URL` + `attachDatabasePool(pool)` from `@neon/functions` (per Neon docs — do NOT use `@neondatabase/serverless`). |
| `db/001_waitlist.sql` | Table DDL below. |
| `scripts/migrate.ts` | Bun script applying `db/001_waitlist.sql` via `pg` on `DATABASE_URL` (bun auto-loads root `.env`). |
| `functions/waitlist.test.ts` | `bun test` unit tests for the pure validator (exported from the function module). |
| `functions/tsconfig.json` | Minimal TS config (NodeNext + DOM types) so the editor/`tsc` resolves `hono`, `pg`, `process` at the repo root. |

Root `package.json` deps: `@neon/config`, `@neon/functions`, `hono`, `pg`,
`resend`; devDep `@types/pg`. Deploy: `neon deploy --env .env`.

### POST / contract

| Case | Behavior | Response |
| --- | --- | --- |
| Valid new signup | insert + send email | `200 { ok: true }` |
| Valid duplicate email | no-op, no email | `200 { ok: true }` |
| Honeypot `company` non-empty | discard silently | `200 { ok: true }` |
| Invalid name/email or malformed JSON | reject | `400 { ok: false, error: 'invalid' }` |
| Resend fails or key missing | log, keep the row | `200 { ok: true }` |

Validation: `name` 1–120 chars trimmed; `email` ≤254 chars, simple RFC-ish
regex. The notification email is best-effort — a send failure never loses a
signup.

### Email

- From: `Marvis AI <notification@updates.getmarvis.com>` (`FROM_EMAIL` env)
- Subject: `You're on the Marvis waitlist`
- Body: short plain-text + minimal HTML greeting `{name}`, confirms they're
  on the list and that we'll email when the macOS build is ready.
- If `RESEND_API_KEY` unset: skip send, `console.warn` — keeps `neon dev`
  usable without a key.

### Schema

```sql
CREATE TABLE IF NOT EXISTS waitlist (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name       text NOT NULL,
  email      citext NOT NULL UNIQUE,
  created_at timestamptz NOT NULL DEFAULT now()
);
```

(`citext` requires `CREATE EXTENSION IF NOT EXISTS citext` in the migration.)

## Web app (`apps/web`)

| File | Change |
| --- | --- |
| `components/waitlist-dialog.tsx` (new) | `"use client"` named-export `WaitlistDialog({ triggerClassName, triggerLabel, triggerDataOdId })` — `Dialog` + `DialogTrigger render={<button className={triggerClassName} data-od-id={triggerDataOdId}>…}` + `DialogContent` holding the form. Local state: `idle`/`submitting`/`success`/`error`. Success view replaces the form ("You're on the list — check your inbox"). Honeypot `company` input visually hidden, `tabIndex={-1}`, `autoComplete='off'`. |
| `components/top-nav.tsx` | `<a href='#download'>` → `<WaitlistDialog triggerClassName='btn btn-secondary' triggerLabel='Download' triggerDataOdId='nav-cta' />`. Stays a server component. |
| `components/hero.tsx` | Primary CTA anchor → `WaitlistDialog` (`btn btn-primary`, `Download for macOS`, `hero-cta-primary`). |
| `components/download-cta.tsx` | Dead `href='#'` anchor → `WaitlistDialog` (`btn btn-primary`, `Download for macOS`, `cta-primary`). GitHub ghost link unchanged. |
| `app/globals.css` | Add `@source '../../../packages/ui/src';`, `@import 'tw-animate-css';`, `@theme inline` mappings (`--color-*` + `--font-heading: var(--font-outfit)` + `--radius-4xl`), and shadcn token values scoped to `[data-slot='dialog-content']` using the `mv-*` palette. |
| `next.config.ts` | `async rewrites()` → `[{ source: '/api/waitlist', destination: process.env.WAITLIST_API_URL ?? 'http://localhost:8787/' }]` (both values end at the function root `/`, matching the Hono `POST /` route; `invocation_url` already carries a trailing slash). |
| `apps/web/.env.local` | `WAITLIST_API_URL=<invocation_url from neon functions get waitlist>` (gitignored via `*.local`). |
| `apps/web/package.json` | `bun add -d tw-animate-css` (imported by globals.css). |

### Why scoped shadcn tokens

`apps/web` globals.css uses a custom Notion-style token set where `--accent`,
`--muted`, `--border` mean different things than the shadcn theme; importing
`@marvis/ui/index.css` wholesale would collide (`:root` unlayered, last wins).
Instead:

1. `@theme inline` declares the `--color-*` mappings globally (harmless —
   nothing else uses `bg-popover` etc.).
2. Value vars (`--popover`, `--popover-foreground`, `--foreground`,
   `--primary`, `--primary-foreground`, `--muted`, `--muted-foreground`,
   `--accent`, `--accent-foreground`, `--destructive`, `--border`, `--input`,
   `--ring`, `--radius`) are set **only** on `[data-slot='dialog-content']`,
   bound to the existing `mv-*` palette — so the dialog renders in the Marvis
   app's own shadcn look (Outfit/Manrope, light) while the page stays Notion.
3. Dialog internals use shadcn utilities only (no `.lead`/`.eyebrow`), so the
   redefined `--muted`/`--accent` can't leak wrong values into site classes.

### `@marvis/ui` barrel additions

`Dialog, DialogTrigger, DialogContent, DialogHeader, DialogTitle,
DialogDescription, DialogClose`, `Input`, `Label` — appended to
`packages/ui/src/index.ts`.

## Neon setup steps (executed during implementation)

```bash
bun install -g neon@latest        # bun, not npm
neon login                        # user completes browser OAuth
neon skills -y                    # agent skills
neon mcp -y                       # MCP config
neon link --project-id soft-bonus-30019447 --branch production -y
neon config init                  # scaffold neon.ts, then edit to the config above
bun scripts/migrate.ts            # create waitlist table
neon deploy --env .env            # deploy function with RESEND_API_KEY + FROM_EMAIL
neon functions get waitlist       # capture invocation_url → apps/web/.env.local
```

`.env` additions to `.gitignore`: `.env` is NOT covered today (`*.local` only
covers `.env.local`) — add `.env` (and keep `.neon`, which is non-secret link
config, committed).

## Conventions (AGENTS.md)

- `bun` for all package management.
- Arrow-function components, named exports in `components/*`.
- `page.tsx` stays a server component — all interactivity lives in
  `WaitlistDialog`.
- lucide imports via `@marvis/ui` barrel with `Icon` suffixes.

## Error handling & abuse

- Client validates `required` + `type='email'`; server re-validates
  (defense-in-depth).
- Duplicate submit while `submitting` → button disabled.
- Honeypot filters naive bots; rate limiting is out of scope for v1.
- Function logs errors (`console.error`) — viewable via `neon` logs/Console.

## Testing

1. `neon dev` → `curl -X POST localhost:8787 -d '{"name":"T","email":"t@t.co"}'`
   — expect `{ok:true}`; invalid → 400; duplicate → 200 no second email; then
   verify row via `psql`/Neon console.
2. `bun test functions/` — validator unit tests.
3. `bun run dev` (apps/web) → modal from all three CTAs → submit → row + Resend
   dashboard entry.
4. `bun run check-types` + `bun run lint` in `apps/web`; `turbo check-types`
  unaffected elsewhere.

## Risks / follow-ups (user action)

| Risk | Mitigation |
| --- | --- |
| Neon project `soft-bonus-30019447` may be in an unsupported region (Functions need us-east-1/2, eu-central-1, ap-southeast-1) | `neon deploy` will fail; create a supported-region project and re-link. |
| `updates.getmarvis.com` not verified in Resend (DKIM) | Emails won't send until domain verified — code path is ready either way. |
| `neon login` is interactive | I run it; user completes OAuth in browser. |
| No real download binary yet | Waitlist IS the download flow per this spec; swap trigger → direct download link later. |
