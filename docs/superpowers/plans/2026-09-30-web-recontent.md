# Web Landing Re-content Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-content `apps/web` around the app's real features — meeting-scene
hero with a faithful live-transcript Listen card — fixing every stale claim
(Listen "Phase 2", wrong hotkeys, missing providers/STT, "planned" models dir).

**Architecture:** Content + markup edits to existing section components, a
rebuilt `MvListen` mock in `mv.tsx`, `.editor*` CSS replaced by `.meet*` +
transcript `.mv-*` styles, then a `DESIGN.md` sync. No new sections, no routing
changes, page stays server-componented.

**Tech Stack:** Next.js 16 App Router, React 19, CSS tokens per `DESIGN.md`,
`@marvis/ui` icon barrel.

## Global Constraints

- `bun` for all scripts — never npm/yarn. Run inside `apps/web`.
- Arrow-function components + named exports only. `page.tsx` stays a server
  component.
- New icons come from the `@marvis/ui` barrel (`Icon`-suffixed names) —
  `lucide-react` is **not** a direct dep of `apps/web`.
- DESIGN.md rules are binding: `--accent #78a1bb` is the only saturated hue;
  colors only via tokens/`color-mix`; whisper borders; `| --- | --- |` table
  style.
- Every product claim must trace to `apps/native` source — no invented metrics.
- No unit-test harness exists for the web app (`bun test` is
  `--pass-with-no-tests`); verification = `bun run lint` + `bun run check-types`
  per task, `bun run build` at the end.
- Remove imports/symbols **your** changes orphan; leave pre-existing dead code
  alone.

---

### Task 1: Rebuild the `mv-*` mocks — real bar controls + live-transcript `MvListen`

**Files:**

- Modify: `apps/web/components/mv.tsx`
- Modify: `apps/web/app/globals.css` (mv scope + transcript styles)
- Modify: `apps/web/components/icons.tsx` (delete `CameraIcon` — orphaned by
  this task)

**Interfaces:**

- Produces: `MvListen` — `({ className?, ...rest }:
  HTMLAttributes<HTMLDivElement>)` — a static faithful mock of
  `ListenSection.tsx` (consumers: `hero.tsx`, `interface-section.tsx`). `MvBar`
  keeps `gate?: 'main' | 'permission' | 'mini'`.

- [ ] **Step 1: Update `mv.tsx` imports + `MvBar` controls**

Replace the import block and the `mini`/`main` gate bodies. The capsule now
mirrors `Bar.tsx`: iris, screen-capture toggle (`MonitorDotIcon`), Listen
recorder (`MicAudioLinesIcon`). The expanded bar's mic is working dictation —
drop `is-off`/"Coming soon".

```tsx
import type { HTMLAttributes, ReactNode } from 'react';
import {
  CaptionsIcon,
  ChevronDownIcon,
  CopyIcon,
  MicAudioLinesIcon,
  MonitorDotIcon,
  PauseIcon,
  SquareIcon,
  TimerIcon,
} from '@marvis/ui';
import { CloseIcon, GearIcon, MicIcon, MvLogo, ShieldIcon } from './icons';
```

```tsx
      {gate === 'mini' && (
        <>
          <MvLogo />
          <span
            className='mv-icon-btn'
            title='Screen capture'>
            <MonitorDotIcon />
          </span>
          <span
            className='mv-icon-btn'
            title='Start listening'>
            <MicAudioLinesIcon />
          </span>
        </>
      )}
      {gate === 'main' && (
        <>
          <MvLogo />
          <input
            className='mv-input'
            placeholder='Ask Marvis…'
            aria-label='Ask Marvis'
          />
          <span
            className='mv-icon-btn'
            title='Dictate'>
            <MicIcon />
          </span>
          <span
            className='mv-icon-btn'
            title='Settings'>
            <GearIcon />
          </span>
        </>
      )}
```

- [ ] **Step 2: Replace `MvListen` with the transcript card**

Delete the Phase-2 chrome body and render the real card structure (header →
speaker filter → blocks → pinned TLDR), matching `ListenSection.tsx` /
`TranscriptBlocks.tsx` / `SpeakerFilter.tsx` / `SummaryStrip.tsx`:

```tsx
/* Listen — the card's live capture surface (ListenSection.tsx): header
 * (title · sources/engine · LISTENING badge · pause/stop), speaker filter
 * row, timestamped speaker blocks with a dimmed interim caret, and the
 * pinned TLDR strip. Card modes share the 600px band. */
export const MvListen = ({ className = '', ...rest }: MvProps) => (
  <div
    className={`mv mv-panel mv-listen ${className}`}
    {...rest}>
    <div className='mv-listen-head'>
      <div className='mv-listen-id'>
        <p className='mv-listen-title'>Design sync</p>
        <p className='mv-listen-sub'>mic + system audio · deepgram nova-2</p>
      </div>
      <span className='mv-badge'>
        <i />
        LISTENING
      </span>
      <span
        className='mv-icon-btn'
        title='Pause'>
        <PauseIcon />
      </span>
      <span
        className='mv-icon-btn'
        title='Stop'>
        <SquareIcon />
      </span>
    </div>
    <div className='mv-filter'>
      <span className='mv-spk is-on'>all</span>
      <span className='mv-spk mv-spk-you'>
        <i />
        you
      </span>
      <span className='mv-spk mv-spk-1'>
        <i />
        speaker 1
      </span>
      <span className='mv-filter-meta'>
        <CaptionsIcon />3 lines
      </span>
      <span className='mv-filter-meta'>
        <TimerIcon />
        1:12
      </span>
      <span
        className='mv-icon-btn'
        title='Copy transcript'>
        <CopyIcon />
      </span>
    </div>
    <div className='mv-tt'>
      <div className='mv-tt-block mv-tt-you'>
        <div className='mv-tt-head'>
          <span className='mv-tt-time'>0:04</span>
          <i className='mv-tt-dot' />
          <span className='mv-tt-name'>You</span>
        </div>
        <p className='mv-tt-text'>
          Before we dive in — did everyone see the new launch date?
        </p>
      </div>
      <div className='mv-tt-block mv-tt-1'>
        <div className='mv-tt-head'>
          <span className='mv-tt-time'>0:41</span>
          <i className='mv-tt-dot' />
          <span className='mv-tt-name'>Speaker 1</span>
        </div>
        <p className='mv-tt-text'>
          Yes — moved to the 14th. Website copy needs to land by Friday,
          though.
        </p>
      </div>
      <div className='mv-tt-block mv-tt-you'>
        <div className='mv-tt-head'>
          <span className='mv-tt-time'>1:12</span>
          <i className='mv-tt-dot' />
          <span className='mv-tt-name'>You</span>
        </div>
        <p className='mv-tt-text'>
          I can take the landing page — draft this week,{' '}
          <span className='mv-tt-interim'>
            then share it Friday
            <span className='mv-caret' />
          </span>
        </p>
      </div>
    </div>
    <div className='mv-tldr'>
      <div className='mv-tldr-head'>
        <span>TLDR · Launch timing</span>
        <ChevronDownIcon />
      </div>
      <p className='mv-tldr-text'>
        Launch moved to the 14th; landing-page copy is due Friday; you&rsquo;re
        drafting it.
      </p>
    </div>
  </div>
);
```

- [ ] **Step 3: mv-scope tokens + transcript CSS**

In `globals.css`, inside the `.mv` token block (after `--mv-primary-fg`), add
the app's real accent + speaker hues (verbatim from `apps/native/src/index.css`
light set):

```css
  --mv-accent: oklch(0.53 0.08 237);
  --mv-speaker-1: oklch(0.55 0.12 160);
  --mv-speaker-2: oklch(0.62 0.12 60);
  --mv-speaker-3: oklch(0.56 0.15 355);
  --mv-speaker-4: oklch(0.55 0.12 295);
```

Replace the whole `/* listen — 400px window; … */ .mv-listen` /
`.mv-listen-chrome` / `.mv-wave` / `.mv-empty` block with:

```css
/* listen — live transcript card, faithful to ListenSection.tsx; the
 * 600px card band shared by chat · listen · history */
.mv-listen {
  width: 600px;
  max-width: 100%;
}

.mv-listen-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 14px;
  border-bottom: 1px solid var(--mv-border);
}

.mv-listen-id {
  flex: 1;
  min-width: 0;
}

.mv-listen-title {
  font-size: 12px;
  font-weight: 600;
}

.mv-listen-sub {
  font-family: var(--font-mono);
  font-size: 10.5px;
  color: var(--mv-muted-fg);
}

.mv-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex: none;
  border: 1px solid color-mix(in oklch, var(--mv-accent) 40%, transparent);
  border-radius: 999px;
  padding: 2px 8px;
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.04em;
  color: var(--mv-accent);
}

.mv-badge i {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--mv-accent);
  animation: mv-ping 2.6s cubic-bezier(0, 0, 0.2, 1) infinite;
}

@keyframes mv-ping {
  75%,
  100% {
    transform: scale(1.9);
    opacity: 0;
  }
}

.mv-filter {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 12px;
  border-bottom: 1px solid var(--mv-border);
}

.mv-spk {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex: none;
  border: 1px solid var(--mv-border);
  border-radius: 999px;
  padding: 2px 8px;
  font-size: 10.5px;
  color: var(--mv-muted-fg);
}

.mv-spk.is-on {
  background: color-mix(in oklch, var(--mv-fg) 6%, transparent);
  color: var(--mv-fg);
  border-color: color-mix(in oklch, var(--mv-fg) 30%, transparent);
}

.mv-spk i {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}

.mv-spk-you {
  color: var(--mv-accent);
}

.mv-spk-1 {
  color: var(--mv-speaker-1);
}

.mv-filter-meta {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-family: var(--font-mono);
  font-size: 10px;
  color: var(--mv-muted-fg);
}

.mv-filter-meta:first-of-type {
  margin-left: auto;
}

.mv-filter-meta svg {
  width: 12px;
  height: 12px;
}

.mv-tt {
  padding: 10px 14px;
  font-size: 13px;
  line-height: 1.6;
}

.mv-tt-block + .mv-tt-block {
  margin-top: 12px;
}

.mv-tt-head {
  display: flex;
  align-items: center;
  gap: 6px;
}

.mv-tt-time {
  flex: none;
  width: 32px;
  font-family: var(--font-mono);
  font-size: 10px;
  color: var(--mv-muted-fg);
}

.mv-tt-dot {
  flex: none;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: currentColor;
}

.mv-tt-name {
  font-size: 12px;
  font-weight: 650;
}

.mv-tt-you {
  color: var(--mv-accent);
}

.mv-tt-1 {
  color: var(--mv-speaker-1);
}

.mv-tt-text {
  margin: 4px 0 0 38px;
  color: var(--mv-fg);
}

.mv-tt-interim {
  color: var(--mv-muted-fg);
}

.mv-tldr {
  border-top: 1px solid var(--mv-border);
  padding: 8px 14px 10px;
}

.mv-tldr-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  font-weight: 600;
}

.mv-tldr-head svg {
  width: 14px;
  height: 14px;
  color: var(--mv-muted-fg);
}

.mv-tldr-text {
  margin: 2px 0 0;
  font-size: 12.5px;
  color: var(--mv-fg);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
```

- [ ] **Step 4: Wire dark-theme + reduced-motion**

In `[data-theme='dark'] .shots .mv` token block add the dark speaker set:

```css
  --mv-speaker-1: oklch(0.72 0.13 160);
  --mv-speaker-2: oklch(0.76 0.13 65);
  --mv-speaker-3: oklch(0.74 0.15 355);
  --mv-speaker-4: oklch(0.74 0.13 295);
```

In the `.shots …` transition selector list, replace `.shots .mv-listen-chrome`
with:

```css
.shots .mv-listen-head,
.shots .mv-filter,
.shots .mv-tldr,
```

In the `prefers-reduced-motion` media query, add `.mv-badge i` to the
killed-animation list.

- [ ] **Step 5: Delete orphaned `CameraIcon`**

Remove `CameraIcon` from `apps/web/components/icons.tsx` (nothing imports it
after Step 1).

- [ ] **Step 6: Verify**

```bash
cd apps/web && bun run lint && bun run check-types
```

Expected: clean (0 errors).

- [ ] **Step 7: Commit**

```bash
git add apps/web/components/mv.tsx apps/web/components/icons.tsx apps/web/app/globals.css
git commit -m "web: rebuild mv mocks — real bar controls, live-transcript Listen card"
```

---

### Task 2: Hero — meeting scene + broadened copy

**Files:**

- Modify: `apps/web/components/hero.tsx` (full rewrite of scene + h1/lead)
- Modify: `apps/web/app/globals.css` (`.editor*` → `.meet*`)

**Interfaces:**

- Consumes: `MvBar`, `MvListen` from `./mv` (Task 1).

- [ ] **Step 1: Rewrite `hero.tsx`**

```tsx
'use client';

import {
  Badge,
  MicIcon,
  MonitorIcon,
  VideoIcon,
  XIcon,
} from '@marvis/ui';
import { MvBar, MvListen } from './mv';
import { WaitlistDialog } from './waitlist-dialog';

const TILES: { initials: string; name: string; cls: string }[] = [
  { initials: 'AK', name: 'Ava', cls: 'av-1' },
  { initials: 'JO', name: 'Jonah', cls: 'av-2' },
  { initials: 'MR', name: 'Meera', cls: 'av-3' },
  { initials: 'RS', name: 'Rui', cls: 'av-4' },
];

export const Hero = () => (
  <section
    className='section hero'
    id='top'
    data-od-id='hero'>
    <div className='container hero-center'>
      <p className='eyebrow'>Private / Personal AI for All</p>
      <h1 className='hero-h1'>
        Ask about anything on your screen. Transcribe every meeting. Keep it
        all on your machine.
      </h1>
      <p className='lead'>
        Marvis is a small floating bar that lives above your work. It sees
        your screen only with permission, hears your meetings, and answers
        through the AI providers you choose — your keys, history, and screen
        data never leave your device.
      </p>
      <div className='hero-cta'>
        <WaitlistDialog
          triggerClassName='btn btn-primary'
          triggerLabel='Join the waitlist'
          triggerDataOdId='hero-cta-primary'
        />
        <a
          className='btn btn-ghost btn-arrow'
          href='#privacy'
          data-od-id='hero-cta-secondary'>
          How it stays private
        </a>
      </div>
      <p className='meta hero-note'>
        macOS build in private testing — you&rsquo;ll get a download link by
        email.
      </p>
      <div className='hero-meta'>
        <Badge
          variant='outline'
          className='tag'>
          Free &amp; Open Source · MIT
        </Badge>
        <Badge
          variant='outline'
          className='tag'>
          No Account
        </Badge>
        <Badge
          variant='outline'
          className='tag'>
          No Cloud Sync
        </Badge>
      </div>
    </div>

    <div className='container'>
      <div
        className='scene'
        data-od-id='hero-scene'>
        <MvBar
          className='marvis-bar'
          data-od-id='marvis-bar'
        />
        <div className='meet'>
          <div className='meet-chrome'>
            <span className='dot' />
            <span className='dot' />
            <span className='dot' />
            <span className='meta'>Design sync — 4 participants</span>
          </div>
          <div
            className='meet-body'
            aria-hidden='true'>
            {TILES.map((t) => (
              <div
                key={t.name}
                className='meet-tile'>
                <span className={`meet-av ${t.cls}`}>{t.initials}</span>
                <span className='meet-name'>{t.name}</span>
              </div>
            ))}
          </div>
          <div
            className='meet-bar'
            aria-hidden='true'>
            <span className='meet-btn'>
              <MicIcon />
            </span>
            <span className='meet-btn'>
              <VideoIcon />
            </span>
            <span className='meet-btn'>
              <MonitorIcon />
            </span>
            <span className='meet-btn is-leave'>
              <XIcon />
            </span>
          </div>
          <MvListen
            className='marvis-panel'
            data-od-id='listen-panel'
          />
        </div>
      </div>
    </div>
  </section>
);
```

`EDITOR_LINES` and the `PreLines` import are gone. `hero.tsx` stays `'use
client'` (unchanged from before — the WaitlistDialog trigger is rendered through
it).

- [ ] **Step 2: Swap `.editor*` for `.meet*` in `globals.css`**

Delete the entire `.editor`, `.editor-chrome`, `.editor-chrome .dot`,
`.editor-chrome .meta`, `.editor-body`, `.editor-side`, `.editor-side .file`,
`.editor-side .file.active`, `.editor-code`, `.editor-code .c-meta`,
`.editor-code .c-mut` block. Replace with:

```css
/* hero scene — a generic meeting window under the floating bar:
   2×2 participant tiles + call toolbar; the Listen card floats over it */
.meet {
  position: relative;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  overflow: hidden;
  box-shadow: var(--elev-raised);
}

.meet-chrome {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 11px 16px;
  border-bottom: 1px solid var(--border-soft);
}

.meet-chrome .dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: color-mix(in oklch, var(--fg) 16%, transparent);
}

.meet-chrome .meta {
  margin-left: 10px;
}

.meet-body {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
  padding: 14px;
  min-height: 380px;
}

.meet-tile {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  border: 1px solid var(--border-soft);
  border-radius: 10px;
  background: color-mix(in oklch, var(--fg) 3%, transparent);
}

.meet-av {
  width: 44px;
  height: 44px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  font-size: 14px;
  font-weight: 600;
  background: color-mix(in oklch, var(--av, var(--muted)) 16%, var(--surface));
  color: var(--av, var(--muted));
}

/* tile hues mirror the transcript's speaker palette (DESIGN.md §mv) */
.av-1 { --av: oklch(0.55 0.12 160); }
.av-2 { --av: oklch(0.62 0.12 60); }
.av-3 { --av: oklch(0.56 0.15 355); }
.av-4 { --av: oklch(0.55 0.12 295); }

.meet-name {
  font-size: 11px;
  color: var(--muted);
}

.meet-bar {
  position: absolute;
  left: 50%;
  bottom: 14px;
  transform: translateX(-50%);
  display: flex;
  gap: 8px;
  padding: 6px 8px;
  border-radius: 999px;
  background: color-mix(in oklch, var(--bg) 72%, transparent);
  backdrop-filter: blur(10px);
  -webkit-backdrop-filter: blur(10px);
  border: 1px solid var(--border-soft);
}

.meet-btn {
  width: 30px;
  height: 30px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  color: var(--muted);
  background: color-mix(in oklch, var(--fg) 6%, transparent);
}

.meet-btn svg {
  width: 14px;
  height: 14px;
}

.meet-btn.is-leave {
  background: color-mix(in oklch, var(--danger) 12%, transparent);
  color: var(--danger);
}
```

Update the `.hero-h1` measure for the new copy (3 short sentences): `width:
800px; transform: translate(-50px, 3px);` — keep the `920px` relax rule as-is.

The existing `@media (max-width: 1180px)` `.marvis-panel` rule keeps working
(panel is now inside `.meet`; change `margin: 0 var(--space-4) var(--space-4)`
stays valid since it's in normal flow there). The `@media (max-width: 920px)`
`.editor-body`/`.editor-side` rules are deleted; add:

```css
@media (max-width: 640px) {
  .meet-body {
    min-height: 0;
  }
}
```

- [ ] **Step 3: Verify + commit**

```bash
cd apps/web && bun run lint && bun run check-types
git add apps/web/components/hero.tsx apps/web/app/globals.css
git commit -m "web: hero meeting scene — Listen card over a call window, broadened copy"
```

---

### Task 3: Features — the three real pillars

**Files:**

- Modify: `apps/web/components/features.tsx`
- Modify: `apps/web/components/icons.tsx` (add `CaptionsIcon`; delete `KeyIcon`,
  `CommandIcon` — orphaned here)

- [ ] **Step 1: Add `CaptionsIcon` to `icons.tsx` (feature-mark style, 1.6
      stroke)**

```tsx
export const CaptionsIcon = (props: IconProps) => (
  <svg
    viewBox='0 0 24 24'
    fill='none'
    stroke='currentColor'
    strokeWidth='1.6'
    strokeLinecap='round'
    strokeLinejoin='round'
    aria-hidden='true'
    {...props}>
    <rect
      x='3'
      y='5'
      width='18'
      height='14'
      rx='2'
    />
    <path d='M7 15h4M15 15h2M7 11h2M13 11h4' />
  </svg>
);
```

- [ ] **Step 2: Rewrite `features.tsx`**

```tsx
import { CaptionsIcon, LockIcon, MonitorIcon } from './icons';
import { sectionStack } from './styles';

export const Features = () => (
  <section
    className='section'
    id='features'
    data-od-id='features'>
    <div
      className='container stack'
      style={sectionStack}>
      <div style={{ maxWidth: '40ch' }}>
        <p className='eyebrow'>What it does</p>
        <h2>Two jobs, done quietly — over everything you open.</h2>
      </div>
      <div className='grid-3'>
        <div
          className='feature card-flat'
          data-od-id='feature-screen-aware'>
          <div className='feature-mark'>
            <MonitorIcon />
          </div>
          <h3>Answers with your screen in context</h3>
          <p>
            Press ⌘⏎ and the latest screen frame rides along with your
            prompt — “explain this”, “what changed”, “compare these”. The
            answer streams back as markdown into the overlay.
          </p>
          <span className='meta'>
            frames live in a 60-second memory ring — never on disk
          </span>
        </div>
        <div
          className='feature card-flat'
          data-od-id='feature-listen'>
          <div className='feature-mark'>
            <CaptionsIcon />
          </div>
          <h3>Meetings, transcribed as they happen</h3>
          <p>
            Mic plus system audio become a speaker-labeled transcript while
            the call runs — pause and resume anytime, filter by voice, copy
            the whole thing. Every five turns, a rolling TLDR lands with
            follow-up questions you can ask in one click.
          </p>
          <span className='meta'>
            deepgram · whisper.cpp · sherpa — your engine, fully local if you
            want
          </span>
        </div>
        <div
          className='feature card-flat'
          data-od-id='feature-private'>
          <div className='feature-mark'>
            <LockIcon />
          </div>
          <h3>Private by design</h3>
          <p>
            No account, no sync, no Marvis servers. Keys live in{' '}
            <span className='num'>~/.marvis</span> masked to{' '}
            <span className='num'>…last4</span>, and calls go straight to the
            provider you pick — or to Ollama, fully on-device.
          </p>
          <span className='meta'>free &amp; open source · MIT</span>
        </div>
      </div>
    </div>
  </section>
);
```

- [ ] **Step 3: Delete `KeyIcon` + `CommandIcon`** from `icons.tsx` (orphaned by
      this rewrite).

- [ ] **Step 4: Verify + commit**

```bash
cd apps/web && bun run lint && bun run check-types
git add apps/web/components/features.tsx apps/web/components/icons.tsx
git commit -m "web: features = Ask / Listen / private-by-design pillars"
```

---

### Task 4: Privacy — honest file tree

**Files:**

- Modify: `apps/web/components/privacy.tsx` (FILE_TREE comment strings only)

- [ ] **Step 1: Update `FILE_TREE`**

```tsx
const FILE_TREE: ReactNode[] = [
  '~/.marvis/',
  <Fragment key='keys'>
    ├── keys.json{' '}
    <span className='dim'>
      0600 · provider keys, masked as …last4 in the UI
    </span>
  </Fragment>,
  <Fragment key='config'>
    ├── config.toml{' '}
    <span className='dim'>
      0644 · providers, models, hotkeys, window position
    </span>
  </Fragment>,
  <Fragment key='db'>
    ├── marvis.db{' '}
    <span className='dim'>
      0600 · SQLite — sessions, messages, transcripts, summaries
    </span>
  </Fragment>,
  <Fragment key='models'>
    └── models/whisper/{' '}
    <span className='dim'>ggml models — tiny · base · small, on demand</span>
  </Fragment>,
];
```

Body copy and stats stand (0 accounts · 60s ring · 3 files under `~/.marvis` —
models are downloads, not app writes).

- [ ] **Step 2: Verify + commit**

```bash
cd apps/web && bun run lint && bun run check-types
git add apps/web/components/privacy.tsx
git commit -m "web: privacy file tree reflects shipped models dir + transcript tables"
```

---

### Task 5: Hotkeys — the real binding set

**Files:**

- Modify: `apps/web/components/hotkeys.tsx`

- [ ] **Step 1: Rewrite**

Two tables in the right column — the five rebindable globals plus the fixed
in-bar keys (per `src-tauri/src/hotkey.rs` `Action` + defaults).
`ds-table`/`kbd`/`card` styles already exist.

```tsx
import type { ReactNode } from 'react';
import { leadTop, splitGrid, surfaceBg } from './styles';

const GLOBAL: [ReactNode, string][] = [
  ['Show / hide the input', '⌘ ⌥ Space'],
  ['Toggle screen capture', '⌘ ⌥ R'],
  ['Start a Listen', '⌘ ⌥ T'],
  ['Session history', '⌘ ⌥ H'],
  ['Lock bar position', '⌘ ⇧ L'],
];

const IN_BAR: [ReactNode, string][] = [
  ['Send', 'Enter'],
  ['New line', '⇧ Enter'],
  ['Send with screen frame', '⌘ ⏎'],
  ['Settings', '⌘ ,'],
];

export const Hotkeys = () => (
  <section
    className='section'
    id='hotkeys'
    data-od-id='hotkeys'
    style={surfaceBg}>
    <div
      className='container grid-2'
      style={splitGrid}>
      <div className='stack'>
        <div style={{ maxWidth: '34ch' }}>
          <p className='eyebrow'>Always in reach</p>
          <h2>Five chords, all rebindable.</h2>
          <p
            className='lead'
            style={{ ...leadTop, fontSize: '17px' }}>
            Click a binding in Settings → Hotkeys and press a new one, or edit{' '}
            <span className='num'>[hotkeys]</span> in{' '}
            <span className='num'>~/.marvis/config.toml</span>. Inside the
            bar, four keys stay fixed. The bar itself drags anywhere and
            remembers; Settings → Bar snaps it to a work-area edge.
          </p>
        </div>
        <div
          className='card'
          data-od-id='deeplink-card'
          style={{ boxShadow: 'none' }}>
          <p
            className='eyebrow'
            style={{ color: 'var(--muted)', marginBottom: '10px' }}>
            Deep links
          </p>
          <p
            className='num'
            style={{ fontSize: '14px', margin: 0, color: 'var(--fg-2)' }}>
            marvis://ask?text=what+changed+here
          </p>
          <p
            style={{
              margin: '10px 0 0',
              color: 'var(--muted)',
              fontSize: '14px',
            }}>
            Focuses the bar and fires an Ask from any app — a browser, a
            script, a launcher. Any other <span className='num'>marvis://</span>{' '}
            link just surfaces the overlay.
          </p>
        </div>
      </div>
      <div
        className='stack'
        style={{ gap: '28px' }}>
        <div>
          <p
            className='meta'
            style={{
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
              marginBottom: '8px',
            }}>
            Global — rebindable
          </p>
          <table
            className='ds-table'
            data-od-id='hotkey-table-global'>
            <thead>
              <tr>
                <th>Action</th>
                <th>Shortcut</th>
              </tr>
            </thead>
            <tbody>
              {GLOBAL.map(([action, keys]) => (
                <tr key={keys}>
                  <td>{action}</td>
                  <td>
                    <span className='kbd'>{keys}</span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div>
          <p
            className='meta'
            style={{
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
              marginBottom: '8px',
            }}>
            In the bar — fixed
          </p>
          <table
            className='ds-table'
            data-od-id='hotkey-table-bar'>
            <thead>
              <tr>
                <th>Action</th>
                <th>Shortcut</th>
              </tr>
            </thead>
            <tbody>
              {IN_BAR.map(([action, keys]) => (
                <tr key={keys}>
                  <td>{action}</td>
                  <td>
                    <span className='kbd'>{keys}</span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  </section>
);
```

- [ ] **Step 2: Verify + commit**

```bash
cd apps/web && bun run lint && bun run check-types
git add apps/web/components/hotkeys.tsx
git commit -m "web: hotkeys section lists the real 5 global + 4 in-bar bindings"
```

---

### Task 6: Providers — full catalog + failover chain + STT

**Files:**

- Modify: `apps/web/components/providers.tsx`

- [ ] **Step 1: Rewrite the left column**

```tsx
import { leadTop, splitGrid } from './styles';

export const Providers = () => (
  <section
    className='section'
    id='providers'
    data-od-id='providers'>
    <div
      className='container grid-2'
      style={splitGrid}>
      <div className='stack'>
        <div style={{ maxWidth: '34ch' }}>
          <p className='eyebrow'>Your model, your call</p>
          <h2>Bring your own everything.</h2>
          <p
            className='lead'
            style={{ ...leadTop, fontSize: '17px' }}>
            API keys talk straight to the providers you pick — no Marvis
            proxy in the middle. Order them into a failover chain: a failed
            call hands off to the next configured provider.
          </p>
        </div>
        <div
          className='row'
          style={{ flexWrap: 'wrap', gap: '10px' }}>
          <span className='tag'>OpenAI</span>
          <span className='tag'>Anthropic</span>
          <span className='tag'>Gemini</span>
          <span className='tag'>OpenRouter</span>
          <span className='tag'>Ollama · local</span>
          <span className='tag'>OpenAI-compatible</span>
        </div>
        <div>
          <p
            className='meta'
            style={{
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
              marginBottom: '10px',
            }}>
            Transcription
          </p>
          <div
            className='row'
            style={{ flexWrap: 'wrap', gap: '10px' }}>
            <span className='tag'>Deepgram · streaming</span>
            <span className='tag'>whisper.cpp · local</span>
            <span className='tag'>sherpa · local</span>
          </div>
        </div>
        <p
          className='meta'
          style={{ margin: 0 }}>
          Keys live in <span className='num'>keys.json</span>, shown masked —{' '}
          <span className='num'>…last4</span>. An optional vision provider
          reads each frame first, so any text model can still answer about
          your screen.
        </p>
      </div>
      <div data-od-id='platform-list'>
        <article
          className='log-row'
          data-od-id='platform-macos'>
          <h3>macOS</h3>
          <span className='meta meta-desc'>
            Apple Silicon & Intel · ScreenCaptureKit
          </span>
          <span className='pull'>
            <span className='pill pill-green'>Available now</span>
          </span>
        </article>
        <article
          className='log-row'
          data-od-id='platform-windows'>
          <h3>Windows</h3>
          <span className='meta meta-desc'>Same Rust core, native capture</span>
          <span className='pull'>
            <span className='tag'>In development</span>
          </span>
        </article>
        <article
          className='log-row'
          data-od-id='platform-linux'>
          <h3>Linux</h3>
          <span className='meta meta-desc'>Same Rust core, native capture</span>
          <span className='pull'>
            <span className='tag'>In development</span>
          </span>
        </article>
      </div>
    </div>
  </section>
);
```

- [ ] **Step 2: Verify + commit**

```bash
cd apps/web && bun run lint && bun run check-types
git add apps/web/components/providers.tsx
git commit -m "web: providers — full LLM catalog, failover chain, STT engines, vision"
```

---

### Task 7: Interface — current views, honest Listen

**Files:**

- Modify: `apps/web/components/interface-section.tsx`
- Modify: `apps/web/app/globals.css` (drop `.mv-stage-center` margin tweak if
  needed — no; only text/dims change)

- [ ] **Step 1: Update section head + gate copy + ghost window**

- `h2` → `One bar. Cards when you need them.`; `lead` →
  `Everything floats above your work — a capsule at rest, an input for asks, and
  cards for chat, listen, and history in one grown window. These are the app's
  real views, recreated at actual size. The toggle swaps in its dark theme.`
- `ghost-win` `.gw-url` text → `Design sync — agenda` (generic doc, not github).
- `GATES` descriptions:
  - mini → `The bar idles as a 104px capsule: iris, screen-capture toggle, and
    the Listen recorder. Click it or just start typing — it morphs open.`
  - main → `The default gate. Type — or dictate with the mic — and ⌘⏎ sends the
    latest screen frame with your question.`
  - permission — unchanged.

- [ ] **Step 2: Replace the `.grid-2` Listen row with a full-width shot**

```tsx
        <figure
          className='shot'
          data-od-id='shot-listen'>
          <div className='shot-pad'>
            <p className='eyebrow'>Listen</p>
            <h3>The transcript writes itself.</h3>
            <p>
              Mic and system audio become a speaker-labeled document while
              the call runs — pause and resume, filter by voice, copy it all.
              Every five turns a TLDR lands with follow-ups you can fire
              straight into the session&rsquo;s own chat. Finished meetings
              stay readable from History.
            </p>
          </div>
          <div className='mv-stage-center'>
            <MvListen />
          </div>
          <figcaption className='shot-cap'>
            <span>listen card · 600px</span>
            <span>one window, grown — chat · listen · history share the band</span>
            <span>rolling summary every 5 turns</span>
          </figcaption>
        </figure>
```

(Removes the old `grid-2` block, `mutedBody` import if orphaned, and the "Phase
2" copy.)

- [ ] **Step 3: Verify + commit**

```bash
cd apps/web && bun run lint && bun run check-types
git add apps/web/components/interface-section.tsx
git commit -m "web: interface section — Listen ships, real card width, generic ghost doc"
```

---

### Task 8: Metadata + CTA copy sweep

**Files:**

- Modify: `apps/web/app/layout.tsx` (`SITE.description`)
- Modify: `apps/web/app/opengraph-image.alt.txt`,
  `apps/web/app/twitter-image.alt.txt`
- Modify: `apps/web/components/download-cta.tsx` (h2)

- [ ] **Step 1: Apply copy**

`layout.tsx`:

```tsx
  description:
    'Marvis is a private AI assistant for macOS. A floating bar sees your screen with permission, transcribes your meetings, and answers through the providers you choose — your keys and screen data never leave your device.',
```

Both `*.alt.txt` files → `Marvis — private AI for your screen and your
meetings`.

`download-cta.tsx` h2 → `Your screen. Your meetings. Your machine.` (lead
unchanged).

- [ ] **Step 2: Verify + commit**

```bash
cd apps/web && bun run lint && bun run check-types
git add apps/web/app/layout.tsx apps/web/app/opengraph-image.alt.txt apps/web/app/twitter-image.alt.txt apps/web/components/download-cta.tsx
git commit -m "web: metadata + CTA copy reflect screen-ask + meeting-listen scope"
```

---

### Task 9: `DESIGN.md` sync + final verification

**Files:**

- Modify: `DESIGN.md` (§6, §7, §9, §10)

- [ ] **Step 1: Update stale sections**

- §6 geometry table: Listen row → `Listen card (.mv-listen) | 600px | chat ·
  listen · history share one grown card band`; bar-state bullets: mini = iris +
  capture toggle + Listen recorder; main mic = dictation (live). Replace the
  "Listen panel" paragraph with a description of the transcript card (header +
  LISTENING badge, speaker filter, `m:ss` blocks with interim caret, pinned
  TLDR).
- §6 token block: add `--mv-accent` + `--mv-speaker-1..4` (light values; dark
  swap values listed alongside).
- §7: hero scene → meeting window (`.meet` tiles + toolbar) with the Listen card
  floating; `.shot-listen` → full-width `.shot` (600px card no longer needs the
  docked-geometry caveat); ghost window = generic agenda doc.
- §9: speaker colors (`You` = deep slate accent, them = speaker-1..4), STT
  catalog (Deepgram nova-2 · whisper.cpp tiny/base/small · sherpa sense-voice),
  the five rebindable hotkeys + four fixed in-bar keys.
- §10: section order — `hero` = live overlay over a meeting window; note
  `assets/marvis-landing.html` predates this pass and `apps/web` is the live
  implementation.

Keep `| --- | --- |` table style.

- [ ] **Step 2: Full verification**

```bash
cd apps/web && bun run lint && bun run check-types && bun run build
```

Expected: clean lint, zero type errors, build succeeds.

- [ ] **Step 3: Visual checklist** (dev server `bun run dev` → :4010)

- Hero: meeting tiles + toolbar under floating bar + Listen card; `≤1180px`
  panel docks below; `≤640px` tiles stay 2×2.
- Interface: light/dark toggle restyles transcript card; speaker chips colored;
  no "Phase 2" text anywhere.
- `grep -ri "phase 2\|editor\|github.com/marvis" apps/web/components
  apps/web/app` → no stale hits (except imports of `PreLines` in `privacy.tsx`,
  still used).

- [ ] **Step 4: Commit**

```bash
git add DESIGN.md
git commit -m "docs: DESIGN.md sync — meeting hero, Listen transcript card, real hotkeys/STT"
```

---

## Self-review notes

- Spec coverage: hero ✓ (T2), mv rebuild ✓ (T1),
  features/privacy/hotkeys/providers/interface ✓ (T3–T7), metadata ✓ (T8),
  DESIGN.md ✓ (T9), nav untouched per spec decision.
- `PreLines` survives — `privacy.tsx` still uses it; hero drops its import.
- Orphan cleanup: `CameraIcon` (T1), `KeyIcon`/`CommandIcon` (T3). `LockIcon`
  was already unused; this plan puts it to use rather than deleting it.
