# Marvis Native — Liquid Glass Surfaces

Date: 2026-09-22
Status: Approved design

## Goal

Apply real macOS material to the overlay windows via
[`tauri-plugin-liquid-glass`](https://github.com/hkandala/tauri-plugin-liquid-glass):
`NSGlassEffectView` on macOS 26+, `NSVisualEffectView` fallback below it, no-op
elsewhere. Lands BEFORE the unified-bar refactor so that work builds on glass
surfaces from the start.

## User decisions

| Question | Decision |
| --- | --- |
| Bar + glass conflict | The capsule ENLARGES to fill the window — the bar window is always exactly its surface, so glass is shaped correctly in every state |
| Application point | Rust side in `build_window` — no `tauri-plugin-liquid-glass-api` npm package |
| Variant / tint | `Regular` variant, no tint — real glass samples the desktop, so light/dark adapts for free |

## The constraint that shaped this

Glass covers the WHOLE window rect. The bar window is a fixed pill-sized
frame while the capsule inside it morphed 130⇄431 px — window-level glass
would render a fat slab around the tiny resting capsule. So the capsule
becomes the window:

| Mode | Window | Contents |
| --- | --- | --- |
| Rest / input / permission | 480×64 | One glass capsule — icon row ⇄ input row is a CONTENT swap, never a width morph |
| (unchanged) `ask` 600×h, `listen` 400×h, `alert` 340×100 | today | Cards already fill their windows |

The 480×64 size arrives now — it was already the approved idle size for the
unified-bar spec, so nothing is thrown away.

## Changes

### Rust

| File | Change |
| --- | --- |
| `src-tauri/Cargo.toml` | `cargo add tauri-plugin-liquid-glass` (0.1.x; 0.1.6 already resolved in this workspace) |
| `src-tauri/src/lib.rs` | `.plugin(tauri_plugin_liquid_glass::init())` before `setup`; new `surface_material` command registered in `invoke_handler` |
| `src-tauri/src/windows/mod.rs` | `build_window` takes a per-label `corner_radius`; after build, `app.liquid_glass().set_effect(&win, LiquidGlassConfig { corner_radius, ..default() })` — warn-and-continue on failure. `BAR_W` 441→480, `BAR_H` 59→64 |

| Window | Radius | Match |
| --- | --- | --- |
| `bar` | 32 | Full capsule (h/2) |
| `ask` | 18 | `radius-lg` |
| `listen` | 18 | `rounded-[18px]` (shared `PANEL`) |
| `alert` | 14 | `rounded-[14px]` |
| `prefs` | — | skipped — decorated opaque window, separate builder |

`surface_material()` returns `"glass"` (macOS 26+, `is_supported()`),
`"vibrancy"` (macOS <26 fallback), or `"none"` (other OS / failure).

### Webview

| File | Change |
| --- | --- |
| `src/lib/commands.ts` | `surfaceMaterial()` wrapper |
| `src/main.tsx` | On mount (every window): `surfaceMaterial()` → `document.documentElement.dataset.material`; invoke failure → `"none"` |
| `src/index.css` | `[data-material="glass" \| "vibrancy"]` strips the fake frost — surfaces marked `.glass-surface` lose `surface 80%` fill + `backdrop-blur`; hairline borders stay. `none`/unset = today's CSS frost (browser dev, future non-macOS) |
| `src/views/Bar.tsx` | Capsule fills the window (`w-full h-full`, window IS the capsule): `expanded` now controls only the CONTENT swap (icon row ⇄ input row + permission/boot-error states); settings gear renders in all `main` states (room now); `edgeFor` uses live window size, not hardcoded 441×59 |
| `src/views/AskPanel.tsx`, `AlertToast.tsx`, `ListenPanel.tsx` | Frosted container classes gain `.glass-surface` so the CSS override reaches them |

### Casualty: the breath

The ±3 px ambient bob (`animate-breath-*` + `iris-breath`) dies — a capsule
cannot bob inside the window that IS its shape. Animating the window itself
every 7 s fights `Moved`-persistence for nothing. Dropped; motion-is-state
says idle wobble shouldn't exist anyway.

### At the merge (next spec)

`set_chat_open` re-calls `set_effect` to swap the bar's `corner_radius`
32⇄18 (capsule⇄card) — one line in the unified-bar work, noted so it isn't
forgotten.

## Caveat

`NSGlassEffectView` is a private Apple API (`macOSPrivateApi` already
enabled). Fine for direct/OSS distribution; would be an App Store review
issue if that ever becomes a channel.

## Testing

- `cargo check`/`clippy` clean; `bun run check-types` for the webview.
- Manual: `bun build:dev` → bar shows real glass capsule; input swap keeps
  shape; alert/ask/listen surfaces glassy; prefs unchanged; vite-only browser
  preview still shows CSS frost (`data-material` absent → `none`).
