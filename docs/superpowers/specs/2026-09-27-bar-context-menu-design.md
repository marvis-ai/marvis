# Bar context menu + shared menus — design

A right-click context menu on the bar's idle capsule, and the same menu
on the tray icon. Both surfaces share ONE menu definition — the tray's
"Show / Hide" item is removed, so the two menus are identical and a
single builder serves both.

## Menu contents

```
Start Conversation          ⌘⌥Space
Start/Stop Screen Recording ⌘⌥R
Start Listening             ⌘⌥T
History                     ⌘⌥H
───────────────────────────
Position ▸  ✓Top / Bottom / Left / Right
            ─ Center
Lock Bar Position           ⇧⌘L
───────────────────────────
Settings                    ⌘,
Quit Marvis
```

- The recording item's label flips **Start ⇄ Stop Screen Recording**
  off the live capture state; the Position submenu checks the bar's
  nearest work-area edge (`bar_edge()`); "Lock Bar Position" is a
  check item tracking `window.bar_locked`.
- "Start Listening" is **start-only** (user decision): disabled while a
  session is live.
- Menu accelerators are display-only — they mirror the configured
  `[hotkeys]` bindings so rebinding updates the labels. The real chords
  are global-shortcut registrations (below).
- "Start Conversation" reuses the existing `bar:toggle-input` emit.

## Architecture

### `src-tauri/src/menus.rs` (new module)

- `pub const MENU_*` item ids — a `menu.*` namespace (`menu.ask`,
  `menu.capture`, `menu.listen`, `menu.history`, `menu.pos.top|bottom|
  left|right|center`, `menu.lock`, `menu.settings`, `menu.quit`). The
  old `tray.*` ids migrate here (internal strings — no persistence or
  webview contract).
- `pub fn build(app: &AppHandle) -> tauri::Result<Menu>` — reads live
  state (`capture.is_running`, `listen.status().is_listening`,
  `pool.bar_edge()`, `cfg.window.bar_locked`, `cfg.hotkeys`) and
  returns the fresh `Menu`.

### Dispatch — one global listener

`MenuEvent`s broadcast to every registered listener (verified in the
runtime: global listeners + all window listeners fire per event), so a
single `app.on_menu_event` registered in `setup` is the only
dispatcher. `menu_dispatch()` in lib.rs matches `menu.*` ids and maps
to the same actions the commands/hotkeys use:

| id | action |
| --- | --- |
| `menu.ask` | emit `bar:toggle-input` to the bar webview |
| `menu.capture` | `toggle_capture(app)` — stop, or gate-checked start (`gate_transition` + `Gate::Main`, same as the `capture_start` command) |
| `menu.listen` / `menu.history` | emit `bar:start-listen` / `bar:show-history` |
| `menu.pos.*` | `pool.snap_edge(dir)` / `pool.recenter_bar()` |
| `menu.lock` | `set_bar_locked(app, !locked)` |
| `menu.settings` / `menu.quit` | `show_settings` / `app.exit(0)` |

`tray::init` drops its `on_menu` param — its menu events flow to the
same global listener; no double-dispatch because only that one handler
matches `menu.*` ids.

### Popup trigger

`WebviewWindow::popup_menu` shows the menu at the cursor. The bar
webview fires `contextmenu` → `invoke('bar_context_menu')` → Rust
builds + pops the menu. Gated to idle only: `gate === 'main' &&
!showInputRow && !cardOpen` (the permission/boot cards and any expanded
surface get no menu).

### Live tray menu

Tray menus are static once built, so `refresh_tray_menu(app)` rebuilds
via `app.tray_by_id("main").set_menu(...)` at the state-change funnels:

- `emit_capture_state` — recording label
- `emit_listen_state` — listen item enabled state
- `persist_bar_position`, `snap_edge`, `recenter_bar` dispatch — edge check
- `set_bar_locked` — lock check
- `swap_hotkeys` — accelerator labels

`refresh_tray_menu` must never be called while holding `pool`/`config`/
`capture`/`hotkeys` locks — `set_menu` blocks on the main thread, and
the main thread's window-event handlers take `pool` (Resized). All
funnel points already run lock-free.

## New global hotkeys

Four actions join `toggle_input` in `default_hotkeys()`, `hotkey::
Action`, `action_for`, and `hotkey_dispatch` — making them real global
chords that are rebindable in Settings → Hotkeys through the existing
`config_set`/`swap_hotkeys` path (user decision):

| action | default | dispatch |
| --- | --- | --- |
| `toggle_capture` | `Cmd+Alt+R` | `toggle_capture(app)` |
| `start_listen` | `Cmd+Alt+T` | emit `bar:start-listen` |
| `show_history` | `Cmd+Alt+H` | emit `bar:show-history` |
| `toggle_lock` | `Cmd+Shift+L` | `set_bar_locked(app, !current)` |

HotkeysTab `ACTIONS` gains the four rows and the section copy updates
(no longer "the only global chord").

## Lock — `window.bar_locked` (persisted)

"Lock" freezes the bar's position (user decision). `WindowPrefs` gains
`bar_locked: bool` (`skip_serializing_if` a `is_false` helper so the
file stays clean); `config_set` accepts `window.bar_locked` (bool).
`set_bar_locked(app, locked)` writes the config, emits `config:changed`,
and refreshes the tray menu.

The bar webview tracks `locked` from `configGet()` + `config:changed`
and suppresses dragging with one line on the stage root:
`onMouseDownCapture={e => locked && e.stopPropagation()}`. Tauri's
drag handler is a document-level *bubble-phase* `mousedown` listener
(verified in `window/scripts/drag.js`), so stopping propagation covers
every drag region in the window — pill, card chrome, gate cards — with
no prop threading. Clicks and context menus are unaffected (separate
events; default actions aren't cancelled). Programmatic moves (snap,
recenter, reclamp) still work while locked.

## Bar.tsx changes

- `onContextMenu` on the stage div → `barContextMenu()` invoke,
  `preventDefault`, gated to idle.
- `locked` state + the mousedown-capture suppression above.
- Extract `pressMic`'s listen-start tail into `startListenSession()`
  (open card + `listenStart` + error raise). New `bar:start-listen`
  handler: gate `main`, no-op while `listening`/`paused`, stop live
  dictation first (same contract as the mic press), then start.
- `bar:show-history` handler: gate `main`, `setPinned('history')` +
  `windowSetChatOpen(true)`.

## commands.ts / events.ts

- `barContextMenu = () => invoke<void>('bar_context_menu')` — new
  command in `generate_handler!`.
- `WindowPrefs` gets `bar_locked?: boolean`.
- New event consts: `EV_BAR_START_LISTEN`, `EV_BAR_SHOW_HISTORY`.

## Tests

- `hotkey.rs`: the "single binding" assertions become five;
  `every_config_default_parses` keeps covering the new defaults.
- `config.rs`: `window.bar_locked` round-trips through `config_set`'s
  writable-key path (covered by the match arm test if present).

## Files

- NEW `apps/native/src-tauri/src/menus.rs`
- `src-tauri/src/{lib.rs, tray.rs, hotkey.rs, config.rs}`
- `src/views/Bar.tsx`, `src/lib/{commands.ts, events.ts}`,
  `src/components/prefs/HotkeysTab.tsx`
