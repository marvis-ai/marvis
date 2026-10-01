# Marvis — Design System

Design documentation for the Marvis landing page (`apps/web`; the earlier
static mock `assets/marvis-landing.html` predates it — see §10) and its
brand assets (`assets/`). The visual language derives from the Notion design
system — warm neutrals, whisper borders, compressed display typography —
re-tuned for Marvis with a slate-blue accent and a wordmark in Galada.

> Marvis is an always-available desktop assistant: a small translucent bar that
> floats above your workspace, sees your screen (with permission), and streams
> answers into an overlay — with keys, history, and screen data staying on
> your machine.

---

## 1. Brand assets

### The mark — `assets/marvis-mark.svg`

A single dark rounded square holding the product's essence: an eye over lines
of text. It is the whole logo — no enclosing tile, no capsule.

| Element | Geometry (48 viewBox) | Fill |
| --- | --- | --- |
| Tile | `rect 3,3 42×42, rx 10` | `--fg-2` / `#31302e` (warm dark) |
| Iris | `circle 24,17.5 r 7.5` | `--accent` / `#78a1bb` |
| Pupil | `circle 24,17.5 r 2.9` | `--bg` / white |
| Text line 1 | `rect 14,30 20×2.6, rx 1.3` | `--surface` / `#f6f5f4` |
| Text line 2 | `rect 17.5,34.8 13×2.6, rx 1.3` | `--meta` / `#a39e98` |

The mark doubles as the favicon (`<link rel="icon" href="assets/marvis-mark.svg">`).

### The lockup — `assets/marvis-logo.svg`

Horizontal lockup: the 48px mark + the word "Marvis" in **Galada**. The Galada
font is embedded as a woff2 `@font-face` data URI inside the SVG so it renders
correctly even when the file is used as an `<img>` (external font links don't
load in that context). Fallback: `'Segoe Script', cursive`.

### The app icon — `assets/marvis-icon.svg` + `marvis-icon-1024.png`

The same mark scaled to 1024: warm-dark rounded square (subtle vertical
gradient + inner top highlight, macOS squircle proportions) with the iris and
text bars floating directly on it. Rendered PNG is produced through the
system's native renderer — the frosted-gradient fill and drop shadow are
dropped by ImageMagick's SVG path, so don't rasterize with `convert`.

### The wordmark — Galada usage rules

Galada is used for the "Marvis" wordmark **only** — never for UI text.

- Header: 30px, weight 400, letter-spacing 0 (never track a connected script),
  `transform: translateY(4.5px)` — optically centers the glyph mass on the
  30px mark; script faces can't be metric-centered because the em box reserves
  room for swashes.
- Footer: 17px, `translateY(2.5px)` (same proportional nudge on the 20px mark).
- SVG lockup: baseline y = 43 against the 48px mark (center y = 28).

---

## 2. Color

All colors live in `:root` as tokens. No raw hex outside the token block.

### Surfaces

| Token | Value | Role |
| --- | --- | --- |
| `--bg` | `#ffffff` | Page canvas, card fill |
| `--surface` / `--surface-warm` | `#f6f5f4` | Warm white — section alternation, subtle fills |

### Foreground

| Token | Value | Role |
| --- | --- | --- |
| `--fg` | `rgba(0,0,0,0.95)` | Primary text — near-black, never pure black |
| `--fg-2` | `#31302e` | Warm dark — headings, dark-section background |
| `--muted` | `#615d59` | Secondary text (~5.5:1 on white) |
| `--meta` | `#a39e98` | Captions, placeholders, quietest tier |

### Borders & elevation

| Token | Value | Role |
| --- | --- | --- |
| `--border` | `rgba(0,0,0,0.1)` | Whisper border — the only border weight |
| `--border-soft` | `rgba(0,0,0,0.06)` | Subtler row dividers |
| `--elev-raised` | 4-layer shadow, max opacity 0.04 | Card depth — felt, not seen |

### Accent — Marvis slate

`--accent: #78a1bb` is the single saturated color on the page. Because it is a
mid-light slate (~2.8:1 on white), it cannot carry white text or small colored
text — the derivative tokens exist for exactly that reason:

| Token | Value | Role |
| --- | --- | --- |
| `--accent` | `#78a1bb` | Fills: primary buttons, pills, iris, tints |
| `--accent-on` | `rgba(0,0,0,0.85)` | Text **on** accent fills (white fails contrast) |
| `--accent-hover` | `color-mix(oklab, accent, black 15%)` | Hover fill |
| `--accent-active` | `color-mix(oklab, accent, black 24%)` | Pressed fill |
| `--accent-text` | `color-mix(oklch, accent, black 34%)` | Accent-colored text on white (≥4.5:1) |
| `--focus-ring` | `0 0 0 3px rgba(120,161,187,0.45)` | Focus halo, slate hue |

**Rule:** color-mix derivatives only — never a second hue. The accent appears
at most twice per screen (eyebrow + primary CTA is the default budget).

### Semantic

`--success: #1aae39` · `--warn: #dd5b00` · `--danger: #dc2626`

### Dark section

`.section-dark` inverts on `--fg-2`. All its inner colors are `color-mix`
derivatives of `--bg` over `--fg-2` — no second palette:

- lead text: `--bg 66%` over `--fg-2`
- eyebrow: `--accent 55%` over `--bg` (lightened slate for the dark ground)
- card fill: `--bg 6%` over `--fg-2`, border `--bg 12%`
- stat borders: `--bg 16%`

---

## 3. Typography

Four font roles — three loaded webfonts plus the system mono stack.

| Role | Stack | Used for |
| --- | --- | --- |
| Display & body | `"NotionInter", "Inter", -apple-system, …` | All page text |
| Wordmark | `"Galada"` (Google Fonts) | "Marvis" wordmark only |
| Product UI | `"Outfit"` | Recreated overlay views (`.mv-*`) — matches the app's real font |
| Mono | `ui-monospace, "SF Mono", …` | Numerals, code, captions, file tree, hotkey chips |

### Scale

| Token | Size | Usage | Tracking |
| --- | --- | --- | --- |
| `--text-4xl` | 64px | Hero display (clamped `40px→64px` at `5.4vw`) | `-0.033em` |
| `--text-3xl` | 48px | Section headings (clamped `30px→48px` at `4vw`) | `-0.031em` |
| `--text-2xl` | 40px | Sub-heading large | — |
| `--text-xl` | 26px | Sub-heading | — |
| `--text-lg` | 20px | `.lead` — section intros, weight 600, muted | `-0.006em` |
| `--text-base` | 16px | Body | 0 |
| `--text-sm` | 14px | Captions, footer | — |
| `--text-xs` | 12px | Badges, meta, eyebrows | `+0.04–0.08em` |

Rules inherited from the Notion system:

- Compression scales with size: `-0.033em` at display relaxing to 0 at body.
- Four weights: 400 read / 500 interact / 600 emphasize / 700 announce.
- Line-height tightens upward: `1.5` body → `1.0` display.
- `h1–h4` use `text-wrap: balance`; `p` uses `text-wrap: pretty`.
- `.eyebrow`: `--text-xs`, 600, `0.08em` tracking, uppercase, `--accent-text`.

---

## 4. Layout

- `--container-max: 1200px`; gutters `24 / 16 / 12px` (desktop/tablet/phone).
- Section rhythm `--section-y-*: 80 / 48 / 32px`; consecutive sections divided
  by the whisper border; `scroll-margin-top: 72px` for anchored nav.
- White and warm-white sections alternate; one dark section (`#privacy`)
  provides the sole deep contrast.
- Hero is centered (`max-width 700px`) with the h1 held to an 800px measure
  via `translate(-50px,3px)` — preserve if editing.
- Grids: `.grid-2`, `.grid-3` collapse to 1 column ≤920px.
- Breakpoints: `1180` (hero Listen card docks below the meeting window),
  `920` (grids stack, hero measure relaxes), `760` (nav links hide),
  `640` (meeting tiles shrink, log-rows collapse, shot stage reflows),
  `480` (phone gutters).

---

## 5. Components

### Buttons (`.btn`)

`padding 9px 18px · radius 4px · 15px/600 · -0.005em`. Transitions on
transform + background + border-color + color, 150ms `--ease-standard`.

- `.btn-primary`: `--accent` fill, `--accent-on` text (dark — see §2), hover
  `--accent-hover`.
- `.btn-secondary`: `fg 5%` translucent fill → `fg 9%` on hover.
- `.btn-ghost`: transparent → `--accent-text` + underline on hover.
- `.btn-arrow`: trailing `→` nudges `translateX(2px)` on hover.
- `:active` → `scale(0.98)`.

### Pills & tags

- `.pill`: `--accent 8%` tint bg, `--accent-text` text, `radius-pill`,
  `4px 10px`, 12px/600, `0.04em`.
- `.pill-green`: success-tinted variant (platform "Available now").
- `.tag`: bordered neutral chip — used for provider names and hero meta
  ("Free & open source · MIT", "No account", "No cloud sync").

### Cards

`.card`: white, whisper border, `radius-lg` 12px, `elev-raised`, 28px padding.
`.card-flat` strips chrome for feature cells. On `.section-dark`, cards become
`bg 6%`-over-`fg-2` translucent panels.

### Feature cell (`.feature`)

36px bordered icon box (`feature-mark`, 18px stroke icon) → `h3` → 15px muted
body → optional `.meta` footnote.

### Stat (`.stat`, dark section)

Top hairline + huge mono numeral `clamp(34px,4vw,48px)` in `--bg` +
14px label at `bg 64%`. Real figures only: `0` accounts · `60s` ring buffer ·
`3` files under `~/.marvis`.

### Table (`.ds-table`) & `.kbd`

15px rows, 13px uppercase headers, whisper row dividers, hover row tint.
`.kbd` renders hotkeys as mono chips (fg-2 text, `fg 5%` fill, whisper border).

### File tree (`.filetree`)

Mono `pre` block inside the `~/.marvis` card; `.dim` comments at `bg 48%`.

### Log rows (`.log-row`, platforms)

`140px | 1fr | auto` grid: name / mono description / status pill. Collapses
to `1fr | auto` ≤640px with the description hidden.

---

## 6. The `.mv-*` overlay-UI system — recreated product views

The hero scene and `#interface` section recreate the real app views 1:1 in DOM
(sources: `apps/native/src/views/Bar.tsx` plus
`apps/native/src/components/{ChatSection,ListenSection}.tsx` and
`components/listen/*` in the Marvis repo). They are honest recreations, not
screenshots — the page copy says so.

**Scope tokens** (declared on `.mv`, light shadcn-style OKLch):

```css
--mv-fg:        oklch(0.148 0.004 228.8)    foreground
--mv-card:      oklch(1 0 0)                panel fill
--mv-muted:     oklch(0.963 0.002 197.1)    muted fill
--mv-muted-fg:  oklch(0.56 0.021 213.5)     muted text
--mv-border:    oklch(0.925 0.005 214.3)    hairline
--mv-input:     oklch(0.925 0.005 214.3)    input wells
--mv-primary:   oklch(0.218 0.008 223.9)    dark primary (real app button)
--mv-primary-fg:oklch(0.987 0.002 197.1)
--mv-accent:    oklch(0.53 0.08 237)        deep slate — "You" voice + badges
--mv-speaker-1: oklch(0.55 0.12 160)        teal-green
--mv-speaker-2: oklch(0.62 0.12 60)         amber
--mv-speaker-3: oklch(0.56 0.15 355)        rose
--mv-speaker-4: oklch(0.55 0.12 295)        violet
```

Font inside `.mv` is **Outfit 14px** — the app's real UI font — kept isolated
from the page's NotionInter chrome. `--mv-accent` and `--mv-speaker-1..4` are
the app's own tokens verbatim (`apps/native/src/index.css`): the accent is the
deep slate `#3a7294`, and the speaker hues hold lightness/chroma equal at
spaced hues so no diarized voice reads as "the important one".

### Geometry (real window sizes)

| View | Size | Notes |
| --- | --- | --- |
| Bar (`.mv-bar`) | **172×64 idle → 600×64 grown** | 9999px pill, `card 80%` + `blur(14px)`, floats 21px below work-area top; rests as a **172px capsule** (`is-mini`), morphs open on click or type-to-wake |
| Ask (`.mv-panel`) | **600px** | `radius 18px`, `card 90%` frosted, drops 8px under the bar |
| Listen card (`.mv-listen`) | **600px** | chat · listen · history share one grown card band |

### Bar states (`.mv-bar-inner`)

Faithful to `Bar.tsx` — the real app has two gates plus a resting state:

- `mini` — the 172px resting capsule: iris + screen-capture toggle +
  Listen recorder (`MicAudioLinesIcon` starts a meeting Listen) +
  history (`HistoryIcon` opens the session list).
- `main` — iris + `Ask Marvis…` input + dictation mic (live — speaks into
  the field) + settings gear.
- `needs_permission` — shield icon + "Screen recording needed" +
  primary `Grant` + link `Open settings`.

### Ask panel

Header = the submitted question (12px/500, line-clamp-2) + X close; body =
14px markdown stream with `.mv-caret` block caret blinking at 1.05s steps.

### Listen card

`ListenSection.tsx` is the live capture surface inside the grown card — the
mock (`.mv-panel.mv-listen`, same 18px radius + `card 90%` frost as Ask)
recreates its four regions:

- Header — session title + mono sources/engine subline
  (`mic + system audio · deepgram nova-2`), a `.mv-badge` LISTENING pill
  (accent ring + pinging dot), pause + stop icon buttons.
- Filter row (`.mv-filter`) — `all` + speaker chips (`you` in
  `--mv-accent`, `speaker 1` in `--mv-speaker-1`), mono line/elapsed
  counters, copy-transcript button.
- Transcript (`.mv-tt`) — `m:ss` mono stamp + colored dot + name per
  block; the streaming interim tail renders muted with the block caret.
- TLDR (`.mv-tldr`) — pinned summary strip (`TLDR · topic`, two-line
  clamp) fed by the app's rolling summary every five closed turns.

### Dark variant (`[data-theme='dark'] .shots`)

The `.mv-*` system is fully tokenized, so dark mode is a pure token swap using
the app's real `.dark` scope from `packages/ui/src/index.css` — never an
invented palette (`--mv-accent` holds — the slate reads on both grounds):

```css
--mv-card:      oklch(0.218 0.008 223.9)    dark panel fill
--mv-fg:        oklch(0.987 0.002 197.1)
--mv-muted:     oklch(0.275 0.011 216.9)    muted fill
--mv-muted-fg:  oklch(0.723 0.014 214.4)
--mv-border:    oklch(1 0 0 / 10%)
--mv-input:     oklch(1 0 0 / 15%)          input wells
--mv-primary:   oklch(0.925 0.005 214.3)    inverts — light pill, dark text
--mv-primary-fg:oklch(0.218 0.008 223.9)
--mv-speaker-1: oklch(0.72 0.13 160)        speakers lighten — same hues
--mv-speaker-2: oklch(0.76 0.13 65)
--mv-speaker-3: oklch(0.74 0.15 355)
--mv-speaker-4: oklch(0.74 0.13 295)

```

Scene chrome follows in the same scope: `.shot` card → dark fill +
`white/10%` border, `.shot-desktop` → dark stage gradient, `.shot-menubar` /
`.ghost-win` / `.shot-dock` / `.mv-stage` → dark frosted variants, `.shot-cap`
/ `.gate-label` → `oklch(0.723)` mono captions. The section itself stays
light — the demos read as "the product running in dark mode," keeping the
page's light rhythm intact.

---

## 7. The `.shot-*` scene system (Interface section)

Framed "screenshots" built from the `.mv-*` views — plus the hero's `.meet`
scene, which borrows the same window chrome without the `.shot` figure card:

- `.scene` + `.meet` (hero) — a generic meeting window under the floating
  bar: three-dot chrome + meeting meta ("Design sync — 4 participants"), a
  2×2 grid of `.meet-tile` participants (avatar hues `.av-1..4` mirror the
  speaker palette), and a floating `.meet-bar` call toolbar (mic · camera ·
  share · leave). The Listen card (`.marvis-panel`) floats over it
  bottom-right; ≤1180px it docks in-flow inside `.meet` below
  `.meet-bar` — side margins `var(--space-4)`; ≤640px the
  tiles stay 2×2 (their min-height just relaxes).
- `.shot` — figure card: white, whisper border, `radius-lg`, `elev-raised`,
  `overflow hidden`.
- `.shot-desktop` — 580px stage, radial warm gradient (`fg-2 8%` over
  `surface`), containing: `.shot-menubar` (26px frosted macOS menu strip),
  `.ghost-win` (defocused agenda doc — `gw-url` title + `gw-lines` text
  rules — at `opacity .72`), `.shot-dock` (frosted dock; last tile is the
  real `assets/marvis-icon.svg`), `.shot-bar` (bar at `top:47px` = 21px
  below the menubar) and `.shot-ask` (`top:102px` = 8px below the bar) —
  both centered on the same axis.
- `.shot-cap` — mono 11px caption row carrying real dimensions.
- `.mv-stage` — diagonal warm gradient pad behind a single bar;
  `.mv-stage-center` for centered panels; `.gate-label` — mono caption naming
  the real state (`state: mini`, `gate: main`, …).
- the Listen figure (`data-od-id="shot-listen"`) — a full-width `.shot`:
  `.shot-pad` copy, then the 600px transcript card centered on
  `.mv-stage-center`. The card now shares the grown 600px band, so the old
  docked-geometry caveat is gone — it stays out of the composite shot
  simply so it reads at actual size.
- `.seg` / `.seg-btn` — Light|Dark segmented toggle, right-aligned in the
  section header (`data-od-id="interface-theme-toggle"`). `next-themes`
  writes `data-theme` on `<html>` and persists the choice to
  `localStorage["marvis-iface-theme"]` (default light, no system); every
  dark rule still scopes as `[data-theme='dark'] .shots …`, so only the
  demos re-theme. `is-on` gets a white pill + hairline ring; a 200ms
  `var(--motion-base)` transition eases the swap on surfaces, borders, and
  text.

Reflow ≤640px: stage grows to 640px, bar/panel offsets shift, ghost window
goes full-bleed.

---

## 8. Motion & interaction states

- `--motion-fast 150ms` / `--motion-base 200ms`,
  `--ease-standard cubic-bezier(0.2,0,0,1)`.
- `.marvis-bar` floats ±6px over 7s (the only ambient animation).
- `.mv-caret` blink, `.mv-spin` spinner, `.mv-badge` ping —
  `prefers-reduced-motion` stills all of it.
- Hover moves backgrounds (`accent → accent-hover`, `fg 5% → 9%`) or position —
  never lightens foreground text.
- Every focusable element gets `--focus-ring` on `:focus-visible`.

---

## 9. Content & provenance rules

- Every product claim on the page comes from the Marvis repo: 60s/120-frame/
  64MB ring buffer, `keys.json` plaintext at 0600 inside the 0700
  `~/.marvis` root, masked `…last4` display, `marvis://` deep links, real
  provider list (OpenAI/Anthropic/Gemini/OpenRouter/Ollama/OpenAI-compatible),
  real model names.
- Speaker colors: `You` renders in `--mv-accent` (deep slate); diarized
  others rotate `--mv-speaker-1..4` by `speaker_idx % 4` — the same mapping
  as `speakerColor` in `components/listen/model.ts`.
- STT catalog: Deepgram `nova-2` (hosted streaming), whisper.cpp
  `tiny`/`base`/`small` (`ggml-*.bin`, local), sherpa-onnx `sense-voice`
  (local) — from `VoiceSetup.tsx` + `voice_models.rs`/`sherpa_models.rs`.
- Hotkeys: five rebindable globals (`toggle_input` ⌘⌥Space ·
  `toggle_capture` ⌘⌥R · `start_listen` ⌘⌥T · `show_history` ⌘⌥H ·
  `toggle_lock` ⌘⇧L — `default_hotkeys()` in `config.rs`) plus four fixed
  in-bar keys (Enter send · ⇧Enter new line · ⌘⏎ send with screen frame ·
  ⌘, settings).
- Platform honesty: macOS = "Available now" (green pill); Windows and Linux =
  "In development" — no platform claim beyond that anywhere.
- No invented metrics, testimonials, or logos.

---

## 10. File inventory

| File | Role |
| --- | --- |
| `apps/web` | The live landing page (Next.js) — `components/mv.tsx` holds the `.mv-*` recreations, `app/globals.css` the tokens + scenes |
| `assets/marvis-landing.html` | Earlier self-contained static mock — predates this pass; kept as a design artifact, not the shipped page |
| `assets/marvis-mark.svg` | The mark (also favicon) |
| `assets/marvis-logo.svg` | Horizontal lockup, Galada embedded as data URI |
| `assets/marvis-icon.svg` | App icon source |
| `assets/marvis-icon-1024.png` | 1024px raster of the icon |
| `DESIGN.md` | This document |

Sections in order: `hero` (live overlay over a meeting window — `.meet`
tiles + toolbar, floating bar + Listen card) → `features` (3 cells) →
`privacy` (dark — file tree + stats) → `interface` (composite desktop
shot, resting capsule + 2 gate states, the Listen card at actual size,
light/dark toggle via `next-themes` on `localStorage["marvis-iface-theme"]`)
→ `hotkeys` (global + in-bar tables, deep links) → `providers` (LLM + STT
tags, platform log rows) → `download` (CTA) → footer.
