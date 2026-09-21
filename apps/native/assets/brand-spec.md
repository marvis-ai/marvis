# Brand spec — Marvis desktop redesign

Extracted from `marvis/design.md` + `packages/ui/src/index.css` (the app's real tokens).

## Tokens

- `--bg` `oklch(0.968 0.003 95)` — warm white canvas (#f6f5f4)
- `--surface` `oklch(1 0 0)` — card / panel fill
- `--fg` `oklch(0.148 0.004 228.8)` — primary text (app's real foreground)
- `--muted` `oklch(0.56 0.021 213.5)` — secondary text (app's muted-foreground)
- `--border` `oklch(0.925 0.005 214.3)` — hairline
- `--accent` `oklch(0.53 0.08 237)` — Marvis deep slate (#3a7294; the #78a1bb hue family deepened per user direction — stricter, techier)

Derived: `--primary` = deep slate accent (buttons take `#3a7294` fills per user direction), `--primary-fg oklch(0.97 0.008 237)` (near-white label, ~5.2:1 on slate), `--destructive oklch(0.577 0.245 27.3)`, `--accent-text` = accent darkened ~34% for text-on-white, `--accent-soft` = accent 14%.

## Fonts

- UI / body: **Outfit** (the app's real UI font)
- Wordmark: **Galada** — "Marvis" wordmark only (embedded in `assets/marvis-logo.svg`)
- Mono: `ui-monospace, SF Mono` — numerals, code, kbd chips, captions

## Visual rules

1. Warm neutrals + whisper borders; the only saturated hue is slate, used ≤2×/screen.
2. Product buttons are deep-slate `--primary` pills (`#3a7294` fill, near-white labels, darker on hover); the same slate carries state (iris, listening, active, focus ring).
3. Overlay chrome = frosted: `card 80–90% + backdrop-blur`, pill bar r=9999, panels r=18.
4. Real geometry: bar 353×47 at 21px under the menubar; chat panel 600px drops 8px under the bar.
5. Motion: 150/200ms `cubic-bezier(0.2,0,0,1)`; ambient = bar breath (~7s); reduced-motion kills it.
6. Content honesty: real providers (OpenAI/Anthropic/Gemini/Ollama), real hotkeys, real `~/.marvis` files — nothing invented.
