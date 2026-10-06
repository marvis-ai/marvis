# AGENTS

You're the topest full-stack web / application engineer, especially in NextJS
framework, tauri 2, vitejs and related severless deployment. You will follow the
principles below

## Web (Vite & NextJS) Principles

### 1. Package Management: Use bun

- **Always** use `bun` for package management
- Install: `bun install <package>`
- Run scripts: `bun run <script>`
- Dev dependencies: `bun add -D <package>`
- Production dependencies: `bun add <package>`

### 2. Component Syntax: Arrow Functions Only

- **Always** use arrow function syntax for React components, hooks, and context
  providers
- ✅ `const MyComponent = () => { ... }`
- ❌ `function MyComponent() { ... }`
- Scoped to authored code — vendored shadcn files in
  `packages/ui/src/components/ui/` keep upstream style

### 3. Export Pattern: Named Exports Only (Non-UI Components)

- **Always** use named exports for components in `src/components/*`
- ✅ `export const MyComponent = () => { ... }`
- ❌ `export default MyComponent`

### 4. Code Reuse: Refactor Repeated Components

- Extract repeated UI patterns into reusable components
- Create shared components in `src/components/` directories
- Use composition over duplication

### 5. Discovery: Find from Current Project First

- Search existing codebase before creating new patterns
- Check `src/components/`, hooks, and utilities for existing solutions

### 6. Markdown Documentation: Compact Table Style

- **Always** use compact table format with spaces after opening pipes
- ✅ `| --- | --- |` (space after `|`)
- ❌ `|---|---|` (no space)

### 7. Page Files: Keep as Server Components

- `page.tsx` files **must remain Server Components** by default — do **not** add
  `"use client"` to a page file
- Move client-side interactivity into dedicated child components (with `"use
  client"`) and import them into the page

### 8. Shared UI components

- Use `packages/ui` for shared UI components, it's Shadcn's standard UI
  components
- Shared different in apps/

### 9. Icons: `Icon`-Suffixed lucide-react Names Only

- **Always** use the `Icon`-suffixed lucide-react alias when exporting or
  importing icons — never the bare name
- ✅ `export { XIcon, InfoIcon } from "lucide-react"` / `import { XIcon } from
  '@marvis/ui'`
- ❌ `export { X, Info } from "lucide-react"` / `import { X } from '@marvis/ui'`
- Apps import icons through the `@marvis/ui` barrel (`packages/ui/src/index.ts`)
  — keep its export list suffixed too

## Project Structure

<!-- BEGIN:nextjs-agent-rules -->

## This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may
all differ from your training data. Read the relevant guide in
`node_modules/next/dist/docs/` (resolved from this file's directory; in
monorepos the `next` package may not be visible from the repo root) before
writing any code. Heed deprecation notices.

This block is written and re-added by `next dev` — verify at
`node_modules/next/dist/server/lib/generate-agent-files.js`. Removing it from a
diff only re-creates the uncommitted change; committing it with your work keeps
the tree clean.

<!-- END:nextjs-agent-rules -->

## Tauri 2 — `apps/native`

The desktop app pairs a Vite/React webview (`src/`) with a Rust backend
(`src-tauri/`). The command surface lives in `src-tauri/src/lib.rs`; everything
below is the existing contract — extend it, don't fork it.

### 10. IPC Boundary: `lib/commands.ts` + `lib/events.ts` Only

- **Never** call `invoke()` or `listen()` inside components, views, or hooks
- Every command gets a typed wrapper in `src/lib/commands.ts`; every event name
  an `EV_*` constant plus payload type in `src/lib/events.ts`
- ✅ `dictationStart()` / `useTauriEvent(EV_DICTATION_STATE, cb)`
- ❌ `invoke('dictation_start', ...)` / `listen('dictation:state', ...)` in a
  component
- Subscribe through `useTauriEvent` only — a raw `listen()` leaks its backend
  listener under StrictMode remounts (tauri-apps/tauri#15799) and every event
  then double-fires

### 11. JS API Imports: Scoped v2 Paths

- ✅ `import { invoke } from '@tauri-apps/api/core'`, `import { getCurrentWindow
  } from '@tauri-apps/api/window'`
- ❌ `import { invoke } from '@tauri-apps/api/tauri'` — the v1 flat path is gone
  in v2
- Plugin features come from their own `@tauri-apps/plugin-*` packages
  (`@tauri-apps/plugin-opener`), not the core api

### 12. Event Contract: `domain:action` Names + Resync Commands

- Event names are `domain:action` kebab-case (`ask:chunk`, `bar:toggle-input`) —
  declared as `EV_*` constants in `events.ts` AND `const EV_*: &str` in Rust;
  change both sides together
- Every live event stream pairs with a `*_status` / `*_current` command the view
  invokes on mount — an emit can land before the webview's listener registers,
  so resync never trusts the stream alone
- `app.emit_to(label, ...)` targets one window; `app.emit(...)` broadcasts —
  pick deliberately, and always fire-and-forget (`let _ =`): a dead webview must
  never stall a transition

### 13. Commands: Guarded, Idempotent, `Result<T, String>`

- Rust params are snake_case, JS arg keys camelCase — the bridge converts
  (`withScreen` → `with_screen`); command names are invoked snake_case exactly
  as declared
- Errors are `Result<T, String>` via `.map_err(|e| e.to_string())` — the string
  is user-surfaceable (it lands in the alert toast), so no internal paths, keys,
  or panic text
- Mutating commands guard themselves server-side (the `Gate::Main` checks) — a
  crafted invoke must not bypass what the UI hides; make start/stop pairs
  idempotent
- Register in `tauri::generate_handler!` and mirror the change in `commands.ts`
  in the same edit

### 14. State & Locking: `AppState`, `parking_lot` vs `tokio` Mutex

- Shared state lives on `AppState`, installed once via `app.manage(...)` in
  `setup` and reached through `State<'_, AppState>` / `app.state::<AppState>()`
  — no globals, no per-command `manage` re-registration
- `parking_lot::Mutex` for synchronous guards; `tokio::sync::Mutex` only when
  the guard must be held across `.await` (command futures are `Send`)
- **Never** hold a `parking_lot`/`std` guard across `.await` — clone the data
  out, drop the guard, then await
- **Never** hold a state lock across `run_on_main_thread`, `set_menu`, or other
  calls that dispatch to the main thread — main-thread window handlers take the
  same locks (ABBA deadlock); main-thread callbacks use `try_lock` and drop on
  contention
- Multi-lock critical sections declare their lock order on the field (see
  `gate_transition`) — follow it

### 15. Threads: `tauri::async_runtime` + Main-Thread Hops

- Blocking OS calls (permission prompts, ScreenCaptureKit, AVFoundation) run on
  `tauri::async_runtime::spawn_blocking` — never on the async executor, never on
  the main thread
- Main-thread-only Cocoa/AppKit APIs hop via `app.run_on_main_thread(...)`
- Spawned tasks and dispatch closures hold an `AppHandle` clone and re-resolve
  `app.state::<AppState>()` per use — they must survive gate transitions and
  webview reloads

### 16. Windows: Programmatic Only

- `app.windows` in `tauri.conf.json` stays `[]` — windows are built by
  `windows/` (the `WindowPool`) via `WebviewWindowBuilder` and referenced
  through label constants (`windows::BAR_LABEL`), never string literals
- Window geometry/material changes go through the pool — the bar IS its native
  surface under liquid glass, so the webview never resizes itself directly

### 17. Capabilities: `capabilities/` ACL, Least Privilege

- Tauri 2 replaced the v1 allowlist with capability files — add the narrowest
  `plugin:allow-*` permission to `capabilities/default.json`; scope `windows:`
  to labels unless genuinely global
- `gen/schemas/` is regenerated by the build — never edit it

### 18. Platform Code: `#[cfg]`-Gated Modules

- Per-OS implementations split into `mod.rs` + `{macos,windows,linux}.rs` (see
  `permissions/`, `capture/`) — command signatures stay uniform across OSes with
  defensive bodies where the platform can't support the feature
- OS-only deps go under `[target.'cfg(target_os = "...")'.dependencies]` in
  `Cargo.toml`; OS-only plugins behind `#[cfg]` in `run()`
- macOS consent strings live in `src-tauri/Info.plist`; private-API usage stays
  behind the declared `macos-private-api` feature

### 19. Dev Loop & Sidecars: `bun run build:dev`

- Run the desktop app with `bun run build:dev` (root or `apps/native`) — it
  stages the `whisper-cli` sidecar via `src-tauri/scripts/build-marvis.sh
  --dev-stage` before `tauri dev`; plain `bun run dev`/`vite` gives a webview
  with no backend
- Vite is pinned to port 1420 (`devUrl`) — a "port already in use" failure means
  a stray dev process; kill it, don't change the port
- Sidecars ship via `bundle.externalBin` — they're staged beside the executable
  (`Contents/MacOS`), not under `Resources`; resolve them through
  `std::env::current_exe()` like `bundled_whisper`
- Rust verification: `cargo test` in `src-tauri` — the `lib.rs` test module
  asserts the command contract from source; update it when the surface changes

### 20. Webview Imports: `@/` Alias, Never `../../`

- Inside `apps/native/src` (the webview), cross-directory imports use the `@/`
  alias (`tsconfig.json` maps `@/*` → `./src/*`) — never climb out with `../../`
- ✅ `import { sherpaStatus } from '@/lib/commands'` / `import { cn } from '@/lib/classes'`
- ❌ `import { sherpaStatus } from '../../lib/commands'`
- Same-directory sibling imports stay `./` (`./types`, `./AuxModelCard`)
