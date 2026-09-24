# Marvis — Desktop Design System

Design specification for the Marvis macOS overlay assistant, as implemented in
`assets/index.html`, `assets/marvis-floating-window.html`, and `assets/marvis-settings-window.html`.
Tokens are extracted from the app's real UI package (`packages/ui`) and the
landing page; the accent was deepened to `#3a7294` per direction.

Companion file: `assets/brand-spec.md` (token extraction notes).

---

## 1. Design principles

1. **Quiet until needed.** Marvis lives as a small breathing capsule over the
   desktop. It expands only when addressed — never decorates, never nags.
2. **One hue.** Warm neutrals carry the entire interface; deep slate is the
   only saturated color and marks exactly one thing: *Marvis is alive*
   (listening, streaming, focused) or *this is the primary action*.
3. **Frosted, floating chrome.** Overlay surfaces are translucent white/dark
   glass with hairline borders and soft, deep shadows — native to macOS.
4. **Real over plausible.** Real providers, real hotkeys, real `~/.marvis`
   files, real geometry. Nothing invented for looks.
5. **Motion is state.** Every animation communicates a state change —
   breathing = idle presence, ring = listening, morph = mode switch. No idle
   wobble, no decoration-only motion.

---

## 2. Color tokens

All colors are OKLch. Derive variants with `color-mix` — never hand-pick hex.

### Light (default)

| Token | Value | Approx | Use |
| --- | --- | --- | --- |
| `--bg` | `oklch(0.968 0.003 95)` | `#f6f5f4` | Warm-white canvas |
| `--surface` | `oklch(1 0 0)` | `#ffffff` | Card / panel / window fill |
| `--fg` | `oklch(0.148 0.004 228.8)` | — | Primary text |
| `--muted` | `oklch(0.56 0.021 213.5)` | — | Secondary text, captions |
| `--border` | `oklch(0.925 0.005 214.3)` | — | Hairline borders |
| `--accent` | `oklch(0.53 0.08 237)` | `#3a7294` | Deep slate — the Marvis hue |
| `--primary` | = `--accent` | `#3a7294` | Primary button fill |
| `--primary-fg` | `oklch(0.97 0.008 237)` | near-white | Label on slate (~5.2:1) |
| `--destructive` | `oklch(0.577 0.245 27.3)` | — | Danger only |
| `--fg-2` | `oklch(0.28 0.008 60)` | — | Kbd chips, deep neutral fill |
| `--accent-soft` | `accent 14%` in oklch | — | Tints: chips, selected rows |
| `--accent-text` | `accent 62% + black` | — | Slate as *text* on light bg |
| `--fg-soft` | `fg 5%` | — | Ghost fills, icon-btn hover |
| `--input-well` | `fg 4%` | — | Inset field wells |
| `--focus-ring` | `0 0 0 3px accent 45%` | — | `:focus-visible` on everything |

### Dark (`is-dark` scope on the stage/window root)

| Token | Value |
| --- | --- |
| `--bg` | `oklch(0.185 0.012 237)` |
| `--surface` | `oklch(0.245 0.016 237)` |
| `--fg` | `oklch(0.93 0.006 230)` |
| `--muted` | `oklch(0.70 0.02 230)` |
| `--border` | `oklch(0.375 0.016 235)` |
| `--fg-2` | `oklch(0.84 0.01 230)` |
| `--accent` | unchanged `#3a7294` |
| `--accent-soft` | `accent 26%` (stronger on dark) |
| `--accent-text` | `accent 55% + white` (lightened for contrast) |
| `--fg-soft` | `fg 8%` |
| `--input-well` | `fg 7%` |

Dark-mode gotchas: error text mixes toward white (`destructive 75% + white`),
scrim sheets use a black base, and the stage root must declare
`color: var(--fg)` so inherited scene text actually flips.

---

## 3. Typography

| Role | Stack | Notes |
| --- | --- | --- |
| UI / body | `'Outfit', -apple-system, system-ui, sans-serif` | The app's real UI font. Base 15px / 1.55 |
| Wordmark | `Galada` | "Marvis" only — embedded in `assets/marvis-logo.svg`. Never for UI text |
| Mono | `ui-monospace, 'SF Mono', Menlo, monospace` | Numerals, kbd chips, eyebrows, captions, key paths, hex labels |

Patterns:

- `.eyebrow` — mono 12px, uppercase, `0.08em` tracking, `--accent-text`.
- `.num` — mono + `tabular-nums` for all counters/stats.
- `.meta` — mono 12.5px muted for stage captions and metadata.
- Overlay surfaces run smaller: bar input 12.5px, panel body 12.5–13px,
  mini-panel 11.5px. Small type is mono or Outfit — never Galada.

---

## 4. Shape, material, depth

- `--radius: 10px` (cards, inputs, sheets) · `--radius-lg: 18px` (panels,
  windows) · `999px` (pill bar, buttons, chips).
- **Frosted overlay:** `surface 80–92%` fill + `backdrop-filter: blur(14px)`
  - 1px `--border`. Heavier chrome (floating panels) at 90–92%; scene chrome
  (menubar, dock) at 55–62%.
- **Shadows are deep and soft, never hard:** panels use
  `0 18px 40px -16px fg 34%`; the settings window
  `0 30px 70px -24px fg 40%` + a 1px surface inset ring.
- Hairlines do the separation work — 1px `--border` or `fg 5–12%` mixes. No
  thick rules, no grey boxes inside grey boxes.

---

## 5. Motion

| Token | Value | Use |
| --- | --- | --- |
| `--ease` | `cubic-bezier(0.2, 0, 0, 1)` | Everywhere — no other ease |
| `--motion-fast` | 150ms | Hover, color, icon swaps |
| `--motion-base` | 200ms | Width morphs, panel open/close, transforms |

Signature motions:

- **Breath** — the floating capsule bobs ±3px toward its docked screen edge on
  a 7s cycle (`mv-breath-{top,bottom,left,right}`). The iris breathes in sync
  (`iris-breath`). This is the only perpetual motion.
- **Listening** — iris pulses on 1.6s (`iris-listen`), bar gains a slate ring
  (`accent 55%` border + `accent 16%` halo), waveform bars animate 1.1s.
- **Capsule ⇄ input morph** — bar width 130⇄431px on `--motion-base`; the
  iris layer collapses (scale .3, fade) while a `←` arrow spins in from
  `rotate(-90°) scale(.4)` with a 55ms stagger. Requires persistent DOM —
  never re-render `innerHTML` mid-morph.
- **Panels** — fade + translate toward the bar's dock direction, 200ms,
  `visibility` delayed so closed panels aren't focusable.
- **Caret** — 7×13px block, 1.05s `steps(1)` blink while streaming.
- `prefers-reduced-motion` kills breath, morphs, waveforms, and the caret.

---

## 6. Surfaces & components

### Floating window (`assets/marvis-floating-window.html`)

Real geometry: the bar's default spot is the middle of the work area;
snapped edges keep a 12px margin; panels drop 8px from the bar.

**2026-09-21 — bar enlarged +25%.** Window 353×47 → 441×59; input pill
345×39 → 431×49; capsule 104 → 130px wide. Sizes below are the new values.

| State | Size | Contents |
| --- | --- | --- |
| Capsule (idle) | 130×49, pill | iris · screenshot · mic |
| Input status | 431×49, pill | iris/back · input · camera · mic |
| Locked / permission gates | 431×49 | icon + label + primary action |
| Rail (left/right dock) | 59×130 | same controls, vertical |

- **Capsule → input:** click iris or start typing. Iris morphs into `←`
  (back). ⏎ sends; Esc or `←` collapses. Draft survives collapse.
- **Chatbox** — 600px max-width frosted panel (r=18) extending from the bar
  along the dock axis. Header shows question (`q-live` while listening),
  gear opens the mini panel; body streams markdown with caret + chips;
  composer row hosts input on rails.
- **Listening** — bar gets the slate ring, mic turns `--accent-text`, panel
  shows the live transcript (`who` labels mono; latest speaker accent-tinted)
  then streams the answer.
- **Settings entry** — the gear on the input bar, `Cmd+,`, and the tray
  item all open the decorated preferences window (below); there is no
  mini panel beside the chatbox.
- **Docks** — the bar snaps to a work-area edge (`top | bottom | left |
  right`). Top/bottom horizontal, left/right vertical rail; panels extend
  inward and breath direction follows the edge. An open card always pins
  its bar row to the card's bottom edge (chat/listen rides above it).
- **Dark** — `is-dark` class on the stage remaps all tokens (§2).

### Settings window (`assets/marvis-settings-window.html`)

- macOS frame: liquid-glass **Sidebar** material with a transparent
  titlebar (hidden title, traffic lights overlaying content — the
  macOS Settings.app look). The window floats above other apps only
  while focused; on blur it returns to normal level, while the bar
  stays always-on-top. **Sidebar** (settings mode: General / Bar /
  Recording / Providers / Hotkeys / Privacy & data / About, mono
  credit footer `@2026 Marvis AI made with 💗`), main column of
  `pref-row`s. "Re-run setup" lives in About → Setup.
- **Onboarding = step-by-step wizard, no sidebar.** A 4-segment progress
  track across the window top (`ob-seg` fills `--primary`) + mono counter
  `2 / 4 · screen access`; steps centered in a ~480px column with Back /
  primary actions. Mode switch resets to step 1. **The bar stays hidden
  for the whole wizard** — it appears only after `onboarding_done` writes,
  and a re-run hides it again until the window closes.
- **BYOK** — provider picker pills (OpenAI / Anthropic / Gemini /
  **OpenRouter** / Ollama / **OpenAI-compatible**). OpenRouter is a
  first-class pinned endpoint (`openrouter.ai/api/v1`, `sk-or-…` key,
  `X-Title: Marvis`); Compatible adds *Provider name* + *Base URL*
  fields above the key field; validation requires `http(s)://`, key optional
  for local endpoints; model becomes free-text. A saved provider is
  promoted to the top of the failover order. Keys persist in
  `~/.marvis/keys.json` (plaintext, mode 0600 — the UI only ever sees the
  `…last4` mask).
- **Provider rows are the failover chain** — Settings → Providers lists
  them in priority order. The grip at the row's left edge drags to
  re-rank (`providers.order`); the switch on the right toggles a provider
  out of the chain (`providers.disabled`) without deleting its key, and
  only enables when the provider is usable (key/endpoint/model present).
  Disabled rows dim but stay editable. A "primary" tag marks the provider
  that would answer right now — the topmost enabled, configured one.
- **Bar tab** — the four-edge snap picker (`top / bottom / left / right`)
  animates the bar to that work-area edge. The bar also drags anywhere
  with the pointer, and every move persists `window.bar_x/y` after a
  400 ms debounce — position is remembered with no "remember" row.
- **Accent** — General tab row: a native color well writes `app.accent`
  (`#rrggbb`, `#3a7294` default); theme.ts applies it to `--accent` and
  every token downstream (`--primary`, soft/text variants, focus ring)
  derives live. The "one hue" rule holds — the user just picks which hue.
- **Hotkeys** — one global, rebindable chord (`toggle_input`
  `Cmd+Alt+Space`) lives in `[hotkeys]`; it emits `bar:toggle-input` to
  the webview, which morphs capsule ⇄ input pill only — the card never
  opens from it. Clicking the binding captures
  the next chord (modifier required, Esc cancels) and writes it through
  `config_set`, which delta-swaps the registered set. The rest are fixed
  keys inside the bar: `Cmd+,` opens settings while the bar is active,
  `Enter` sends / `Shift+Enter` adds a line / `Cmd+Enter` sends with the
  current screenshot at the input. No fixed
  move/scroll/click-through/display shortcuts — the bar moves by drag
  or the Bar-tab picker.
- Appearance control (Auto / Light / Dark segmented) lives in General;
  `is-dark` on the stage flips the whole scene.

### Stage chrome (both screens + launcher)

- Menubar: frosted `bg 88%` + blur(12px), crumb left, meta right.
- Ghost editor: `--surface` window, sidebar + code lines as `fg-soft` bones,
  `.tint` lines in `--accent-soft`, one `ghost-err` line in `--destructive`.
- Dock: frosted strip, 38px tiles (`t1–t4` fg gradients), Marvis tile uses
  the real `assets/marvis-icon.svg`, running dot under it.
- Demo switchers under the stage (prototype-only affordance): mono 11px
  segmented groups — `gate`, `dock`, `appearance`.

### Buttons & controls

- `.btn-primary` / `BTN_PRIMARY` — pill, `--primary` fill, `--primary-fg`
  label. Hover `88% + black 12%`; **active `74% + black 26%`** (a deeper
  slate dip, plus `scale(.97)` on bar buttons). Disabled = only state allowed
  to lose contrast.
- `.btn-outline` / `BTN_OUTLINE` — transparent + `--border`; hover deepens
  border to `fg 30%`.
- `.btn-link` / `BTN_LINK` — muted text; hover → `--accent-text` +
  underline.
- `ICON_BTN` — 26px round ghost; hover `fg-soft` fill + `fg` icon.
- Inputs — `--input-well` fill, `--border`, r=8, mono value; focus →
  `accent` border + `--focus-ring`.
- Segmented controls / switches — `--primary` fill for the on-segment.

---

## 7. Interaction & contrast rules

- Hover moves the **background** (±0.06–0.12 L or a `color-mix` step); never
  fade foreground toward muted. Slate buttons darken on hover/press.
- Light text on `#3a7294` ≈ 5.2:1 — the reason the accent was deepened from
  `#78a1bb` (white on that only reaches ~2.9:1).
- Text-form accent on light surfaces is `--accent-text` (62% + black), on
  dark `accent 55% + white`.
- Every focusable element shows `--focus-ring`. Focus is never `outline: none`
  without it.
- One primary action per surface state. Secondary actions are outline/ghost/link.

---

## 8. Assets

| File | Use |
| --- | --- |
| `assets/marvis-logo.svg` | Galada wordmark + iris lockup — onboarding welcome, launcher |
| `assets/marvis-icon.svg` | App icon — dock tile |
| `assets/marvis-mark.svg` | Iris mark alone — favicon-scale uses |

Never redraw or substitute the iris/wordmark; reference the SVGs.

---

## 9. Don'ts

- No second hue. No gradients on surfaces, no purple wash, no emoji icons.
- No `#78a1bb` — it's retired in favor of `#3a7294`.
- No white labels on the slate (use `--primary-fg` ink-light).
- No sidebar in onboarding; no wizard steps inside the settings sidebar mode.
- No `innerHTML` re-render of the bar mid-interaction (breaks the morph);
  drive state with `data-*` attributes (the markup is Tailwind utilities —
  shared bundles live in `src/lib/classes.ts`).
- No `white-space: nowrap` on oversized type; no remote images, no invented
  providers/hotkeys/metrics.
