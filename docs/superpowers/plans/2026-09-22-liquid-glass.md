# Liquid Glass Surfaces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Apply real macOS material (`NSGlassEffectView` on 26+, `NSVisualEffectView` fallback) to all overlay windows via `tauri-plugin-liquid-glass`, and enlarge the bar capsule to fill its 480×64 window so the glass shape is always correct.

**Architecture:** Rust applies the effect in `build_window` with a per-label corner radius (no guest-js package). A `surface_material` command tells the webview which material backs it; CSS strips the fake frost only when a real one exists. The bar capsule fills its window — the width morph becomes a pure content swap.

**Tech Stack:** Tauri 2, `tauri-plugin-liquid-glass` 0.1.x, React 19, Tailwind 4.

## Global Constraints

- Package manager is **bun** everywhere; Rust deps via `cargo add` inside `apps/native/src-tauri`.
- React components: arrow functions only; named exports for `src/components/*`.
- `bar` window becomes **480×64** (`BAR_W`/`BAR_H` constants); radii: bar 32 (= h/2), `ask` 18, `listen` 16, `alert` 14; `prefs` untouched.
- `html, body, #root` are already `background: transparent` — do not regress.
- No `liquid-glass:*` capability needed — Rust-side API only.
- Verify per task: `cargo test` in `apps/native/src-tauri`, `bun run build` in `apps/native` (tsc + vite).

---

### Task 1: Plugin registration + `surface_material` channel

**Files:**

- Modify: `apps/native/src-tauri/Cargo.toml` (via `cargo add`)
- Modify: `apps/native/src-tauri/src/lib.rs`
- Modify: `apps/native/src/lib/commands.ts`
- Test: manual — `cargo check` + `bun run build` compile cleanly

**Interfaces:**

- Produces: `surface_material` Tauri command → `'glass' | 'vibrancy' | 'none'`; frontend wrapper `surfaceMaterial()` in `src/lib/commands.ts`. Task 3's CSS and `main.tsx` consume these.

- [ ] **Step 1: Add the Rust dependency**

```bash
cd apps/native/src-tauri && cargo add tauri-plugin-liquid-glass
```

Expected: adds `tauri-plugin-liquid-glass = "0.1.x"` to `[dependencies]` (0.1.6 already resolved in this workspace's lockfile from a prior build).

- [ ] **Step 2: Register the plugin in the builder**

In `apps/native/src-tauri/src/lib.rs`, add the import at the top with the other `use`s:

```rust
use tauri_plugin_liquid_glass::LiquidGlassExt;
```

and register in `run()` after `.plugin(tauri_plugin_opener::init())`:

```rust
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_liquid_glass::init())
```

- [ ] **Step 3: Add the `surface_material` command**

In `lib.rs`, in the "Commands — config / app" region (before `quit_application`), add:

```rust
/// `"glass" | "vibrancy" | "none"` — which native material backs the
/// overlay windows. macOS 26+ reports glass; older macOS gets the
/// plugin's NSVisualEffectView fallback; other OSes get none (the
/// webview keeps its CSS frost).
#[tauri::command]
fn surface_material(app: AppHandle) -> &'static str {
    #[cfg(target_os = "macos")]
    {
        if app.liquid_glass().is_supported() {
            "glass"
        } else {
            "vibrancy"
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        "none"
    }
}
```

Then add `surface_material` to the `invoke_handler` list (after `config_set`, before `quit_application`).

- [ ] **Step 4: Frontend wrapper**

In `apps/native/src/lib/commands.ts`, at the end of the "windows" section (after `windowBarEdge`):

```ts
/** `'glass' | 'vibrancy' | 'none'` — whether a native material backs the
 * window; CSS strips its fake frost when one does. */
export const surfaceMaterial = () =>
  invoke<'glass' | 'vibrancy' | 'none'>('surface_material');
```

- [ ] **Step 5: Verify compile**

```bash
cd apps/native/src-tauri && cargo check
cd apps/native && bun run build
```

Expected: both clean (warnings ok, errors not).

- [ ] **Step 6: Commit**

```bash
git add apps/native/src-tauri/Cargo.toml apps/native/src-tauri/Cargo.lock apps/native/src-tauri/src/lib.rs apps/native/src/lib/commands.ts
git commit -m "feat(native): register liquid-glass plugin + surface_material command"
```

---

### Task 2: Apply glass in `build_window` + new bar dimensions

**Files:**

- Modify: `apps/native/src-tauri/src/windows/mod.rs`
- Test: `cargo test` (existing layout tests must still pass — they don't touch glass)

**Interfaces:**

- Consumes: `LiquidGlassExt`/`LiquidGlassConfig` from Task 1's dependency.
- Produces: `build_window(app, label, w, h, corner_radius)` signature; `Panel::corner_radius()`; `BAR_W = 480`, `BAR_H = 64`. The unified-bar spec later re-calls `set_effect` to swap the bar radius 32⇄18 on expand.

- [ ] **Step 1: Update `build_window` to apply glass**

In `windows/mod.rs`, add the import:

```rust
use tauri_plugin_liquid_glass::{LiquidGlassConfig, LiquidGlassExt};
```

Change the signature and tail of `build_window` (the `prefs` window keeps its own builder — untouched):

```rust
/// Shared builder flags for every Marvis overlay window (spec): frameless,
/// transparent, always-on-top, non-resizable, skip-taskbar, no shadow —
/// then `set_visible_on_all_workspaces`, `set_content_protected`, and the
/// liquid-glass material. `corner_radius` matches the surface's CSS radius —
/// the glass view fills the window, so its shape IS the surface shape.
fn build_window(
    app: &AppHandle,
    label: &str,
    w: f64,
    h: f64,
    corner_radius: f64,
) -> anyhow::Result<WebviewWindow> {
    let url = WebviewUrl::App(format!("index.html?view={label}").into());
    let win = WebviewWindowBuilder::new(app, label, url)
        .inner_size(w, h)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .shadow(false)
        .visible(false)
        .build()?;
    if let Err(e) = win.set_visible_on_all_workspaces(true) {
        log::warn!("windows: set_visible_on_all_workspaces failed for {label}: {e}");
    }
    // Unconditional per arch rule — no toggle.
    if let Err(e) = win.set_content_protected(true) {
        log::warn!("windows: set_content_protected failed for {label}: {e}");
    }
    if let Err(e) = app.liquid_glass().set_effect(
        &win,
        LiquidGlassConfig {
            corner_radius,
            ..Default::default()
        },
    ) {
        log::warn!("windows: liquid glass failed for {label}: {e}");
    }
    Ok(win)
}
```

- [ ] **Step 2: New bar dimensions + per-surface radii**

In `windows/mod.rs` constants:

```rust
const BAR_W: f64 = 480.0;
const BAR_H: f64 = 64.0;
```

Add to `impl Panel`:

```rust
    /// Glass corner radius matching the panel's CSS card radius.
    fn corner_radius(self) -> f64 {
        match self {
            Panel::Ask => 18.0,
            Panel::Listen => 16.0,
        }
    }
```

Update the three `build_window` call sites:

```rust
// in create_bar_only — capsule radius = half the bar height:
let bar = build_window(app, "bar", BAR_W, BAR_H, BAR_H / 2.0)?;
// ...
pool.alert = Some(build_window(app, ALERT_LABEL, ALERT_W, ALERT_H, 14.0)?);

// in create_feature_windows:
let win = build_window(
    app,
    panel.label(),
    panel.width(),
    panel.default_height(),
    panel.corner_radius(),
)?;
```

- [ ] **Step 3: Verify**

```bash
cd apps/native/src-tauri && cargo test
```

Expected: all existing tests pass (no test touches window building; layout math unchanged).

- [ ] **Step 4: Commit**

```bash
git add apps/native/src-tauri/src/windows/mod.rs
git commit -m "feat(native): apply liquid glass per overlay window; bar 480x64"
```

---

### Task 3: Webview adapts — capsule fills window, frost strips under glass

**Files:**

- Modify: `apps/native/src/main.tsx`
- Modify: `apps/native/src/index.css`
- Modify: `apps/native/src/lib/classes.ts` (PANEL const)
- Modify: `apps/native/src/views/Bar.tsx`
- Modify: `apps/native/src/views/AlertToast.tsx`
- Modify: `apps/native/src/views/AskPanel.tsx`
- Modify: `apps/native/src/views/ListenPanel.tsx`
- Test: `bun run build` (tsc catches removed imports/props)

**Interfaces:**

- Consumes: `surfaceMaterial()` from Task 1.
- Produces: `.glass-stage` / `.glass-surface` marker classes + `data-material` on `<html>` — reused by the unified-bar work.

- [ ] **Step 1: Material marker on boot**

In `src/main.tsx`:

```tsx
import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { surfaceMaterial } from './lib/commands';
import { initTheme } from './lib/theme';
import './index.css';

// Appearance: `app.appearance` in config.toml (auto|light|dark); `auto`
// follows macOS. initTheme applies `is-dark`/`dark` and tracks
// `config:changed` + the system scheme.
initTheme();

// Native material marker: 'glass' | 'vibrancy' | 'none'. Under a real
// material the CSS frost is stripped (double-frosting looks muddy);
// 'none' — browser dev, non-macOS — keeps it.
void surfaceMaterial()
  .then((m) => {
    document.documentElement.dataset.material = m;
  })
  .catch(() => {});

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
```

- [ ] **Step 2: CSS — strip rules + delete dead breath tokens**

In `src/index.css`, after the `html, body, #root` block (~line 103), add:

```css
/* Real material behind the window replaces the fake frost: marked
   surfaces go transparent and edge-to-edge so the glass shape IS the
   surface shape. `data-material` is set per window in main.tsx;
   'none'/unset keeps the CSS frost (browser dev, future non-macOS). */
[data-material='glass'] .glass-stage,
[data-material='vibrancy'] .glass-stage {
  padding: 0;
}
[data-material='glass'] .glass-surface,
[data-material='vibrancy'] .glass-surface {
  background-color: transparent;
  backdrop-filter: none;
}
```

Delete the four dead tokens (~lines 85–88):

```css
  --animate-breath-top: mv-breath-top 7s var(--ease) infinite;
  --animate-breath-bottom: mv-breath-bottom 7s var(--ease) infinite;
  --animate-breath-left: mv-breath-left 7s var(--ease) infinite;
  --animate-breath-right: mv-breath-right 7s var(--ease) infinite;
```

and the four dead keyframes `mv-breath-top`, `mv-breath-bottom`,
`mv-breath-left`, `mv-breath-right` (~lines 120–157) including the
`/* the pill's idle bob … */` comment. Keep `--animate-iris-breath`
(content-level pupil pulse, still used by `Iris`) and
`--animate-iris-listen` / `--animate-waveform` (Phase-2 tokens).

- [ ] **Step 3: `PANEL` gets the marker**

In `src/lib/classes.ts`:

```ts
export const PANEL =
  'glass-surface flex flex-col overflow-hidden rounded-[18px] border border-border bg-[color-mix(in_oklch,var(--surface)_90%,transparent)] backdrop-blur-lg';
```

- [ ] **Step 4: Stage/surface markers on the panels + toast**

`src/views/AskPanel.tsx` — outer wrapper:

```tsx
<div className='glass-stage p-1'>
```

`src/views/ListenPanel.tsx` — same change on its `p-1` wrapper.

`src/views/AlertToast.tsx` — outer `p-1` wrapper gets `glass-stage`, and the toast card div gains `glass-surface`:

```tsx
<div className='glass-stage p-1'>
  <div className='glass-surface flex flex-col gap-1.5 rounded-[14px] border border-[color-mix(in_oklch,var(--destructive)_28%,var(--border))] bg-[color-mix(in_oklch,var(--surface)_92%,transparent)] px-2.75 pt-2.25 pb-2.5 shadow-[0_18px_40px_-16px_color-mix(in_oklch,var(--fg)_34%,transparent)] backdrop-blur-lg'>
```

(keep the rest of that file unchanged)

- [ ] **Step 5: `Bar.tsx` — capsule fills the window**

Delete now-dead code:

- `import { currentMonitor, getCurrentWindow } from '@tauri-apps/api/window';`
- the `Edge` type, `edgeFor` function, `edge` state, and the `useEffect` with `onMoved`
- `data-pos={edge}` on the stage div

Update the stage and pill:

```tsx
  return (
    <div
      className='group/stage glass-stage flex h-full flex-col items-center justify-center p-1'
      data-tauri-drag-region>
      <div
        className={pill}
        data-expanded={expanded || undefined}
        data-tauri-drag-region='deep'>
        {body()}
      </div>
    </div>
  );
```

New `pill` (window IS the capsule — width morph gone, breath gone):

```tsx
  const pill = cn(
    'group/bar glass-surface flex h-full w-full flex-none flex-col justify-center rounded-full border border-border bg-[color-mix(in_oklch,var(--surface)_80%,transparent)] backdrop-blur-[14px] select-none transition-[border-color,box-shadow] duration-(--motion-base) ease-(--ease) motion-reduce:transition-none',
  );
```

Center the resting icon row in the wide capsule — `inner` gains
`justify-center` when collapsed:

```tsx
  const inner = cn(
    'flex min-h-0 flex-1 items-center gap-1.5',
    expanded ? 'px-2.75' : 'justify-center px-1.75',
  );
```

The settings gear has room in every `main` state now — drop the
`expanded &&` gate so it renders unconditionally inside the form.

- [ ] **Step 6: Verify**

```bash
cd apps/native && bun run build
```

Expected: clean tsc + vite build (removed imports must not leave usages — `edgeFor`, `getCurrentWindow`, `currentMonitor` all gone).

- [ ] **Step 7: Manual smoke (if a macOS session is available)**

```bash
bun run build:dev   # tauri dev
```

Expected: bar is a 480×64 glass capsule (icon row at rest, input row on
click/type); ask/listen/alert surfaces show real material; prefs
unchanged. Vite-only browser preview (`bun dev`) still shows CSS frost.

- [ ] **Step 8: Commit**

```bash
git add apps/native/src/main.tsx apps/native/src/index.css apps/native/src/lib/classes.ts apps/native/src/views/
git commit -m "feat(native): capsule fills window; CSS frost yields to real glass"
```

---

## Self-review notes

- Spec coverage: plugin wiring (T1) · per-window radii (T2) · 480×64 +
  capsule-fill + breath removal + material channel + frost strip (T2/T3) ·
  prefs untouched · merge-time radius swap documented in T2 interfaces.
- No placeholders; every code step has the actual code.
- Type consistency: `surface_material`/`surfaceMaterial`/`data-material`/
  `glass-stage`/`glass-surface`/`build_window(…, corner_radius)`/
  `Panel::corner_radius` named identically throughout.
