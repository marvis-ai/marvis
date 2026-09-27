# Bar Context Menu + Unified Menus Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: execute inline in this
> session. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Right-click menu on the bar's idle capsule + identical menu on
the tray icon; four new global rebindable hotkeys; persisted position
lock.

**Architecture:** One `menus.rs` module owns all `menu.*` item ids and a
single `build(app) -> Menu` used for both `tray.set_menu` and
`WebviewWindow::popup_menu`. One global `app.on_menu_event` dispatcher
in lib.rs handles all ids (menu events broadcast to every listener).
New `hotkey::Action` variants reuse the config-driven `[hotkeys]`
registration.

**Spec:** `docs/superpowers/specs/2026-09-27-bar-context-menu-design.md`

## Global Constraints

- Rust commands/handlers only touch state through `AppState`; never hold
  `pool`/`config`/`capture`/`hotkeys` locks across `tray.set_menu`
  (it blocks on the main thread).
- Arrow-function components, named exports (project rules).
- Event names in `src/lib/events.ts` mirror the Rust emit constants.

---

### Task 1: config — `window.bar_locked` + four new hotkey defaults

**Files:**

- Modify: `apps/native/src-tauri/src/config.rs`

- [ ] **Step 1:** `WindowPrefs` gains `bar_locked`:

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowPrefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar_y: Option<f64>,
    /// `true` freezes the bar's position — pointer drags are suppressed
    /// webview-side while programmatic moves (snap/recenter) still work.
    #[serde(skip_serializing_if = "is_false")]
    pub bar_locked: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}
```

- [ ] **Step 2:** `default_hotkeys()` becomes five bindings (update the
  doc comment — no longer "the one configurable binding"):

```rust
pub fn default_hotkeys() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("toggle_input".into(), "Cmd+Alt+Space".into()),
        ("toggle_capture".into(), "Cmd+Alt+R".into()),
        ("start_listen".into(), "Cmd+Alt+T".into()),
        ("show_history".into(), "Cmd+Alt+H".into()),
        ("toggle_lock".into(), "Cmd+Shift+L".into()),
    ])
}
```

- [ ] **Step 3:** `cargo test -p Marvis config` — expect failures in
  `hotkey.rs` tests next task (they assert one binding).

### Task 2: hotkey actions

**Files:**

- Modify: `apps/native/src-tauri/src/hotkey.rs`

- [ ] **Step 1:** Extend `Action` + `action_for`:

```rust
pub enum Action {
    ToggleInput,
    /// `toggle_capture` — start/stop ambient screen recording (`Cmd+Alt+R`).
    ToggleCapture,
    /// `start_listen` — begin a meeting Listen session (`Cmd+Alt+T`);
    /// start-only — a live session makes it a no-op webview-side.
    StartListen,
    /// `show_history` — open the card on the session list (`Cmd+Alt+H`).
    ShowHistory,
    /// `toggle_lock` — freeze/unfreeze the bar's position (`Cmd+Shift+L`).
    ToggleLock,
}
```

```rust
"toggle_input" => Some(Action::ToggleInput),
"toggle_capture" => Some(Action::ToggleCapture),
"start_listen" => Some(Action::StartListen),
"show_history" => Some(Action::ShowHistory),
"toggle_lock" => Some(Action::ToggleLock),
_ => None,
```

- [ ] **Step 2:** Fix tests: `every_config_default_parses` asserts
  `binds.len() == 5`; `action_for_maps_config_names` asserts the four new
  names (plus existing `None` cases); `bindings_returns_the_single_toggle_pair`
  → `bindings_returns_the_default_set` asserting all five pairs (BTreeMap
  order: `show_history`, `start_listen`, `toggle_capture`,
  `toggle_input`, `toggle_lock`).

- [ ] **Step 3:** `cargo test -p Marvis hotkey` — all pass.

### Task 3: `menus.rs` — shared menu module

**Files:**

- Create: `apps/native/src-tauri/src/menus.rs`
- Modify: `apps/native/src-tauri/src/lib.rs` (`mod menus;`)

```rust
//! `menus.rs` — the ONE menu shared by the tray icon and the bar's
//! idle-state right-click popup.
//!
//! Every item lives under the `menu.*` id namespace, dispatched by the
//! single global `on_menu_event` listener registered in lib.rs
//! (`menu_dispatch`) — menu events broadcast to EVERY listener, so the
//! prefix match is what keeps dispatch single-fire.
//!
//! The menu is rebuilt rather than mutated: `build` reads live state
//! (capture running, listen live, nearest edge, `window.bar_locked`,
//! `cfg.hotkeys` for the accelerator labels) so a popup is always
//! current, and `refresh_tray_menu` swaps the tray's copy at each
//! state-change funnel.

use tauri::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu,
};
use tauri::{AppHandle, Manager, Wry};

use crate::capture::MacosCapture;
use crate::windows::Dir;
use crate::AppState;

pub const MENU_ASK: &str = "menu.ask";
pub const MENU_CAPTURE: &str = "menu.capture";
pub const MENU_LISTEN: &str = "menu.listen";
pub const MENU_HISTORY: &str = "menu.history";
pub const MENU_POSITION: &str = "menu.position";
pub const MENU_POS_TOP: &str = "menu.pos.top";
pub const MENU_POS_BOTTOM: &str = "menu.pos.bottom";
pub const MENU_POS_LEFT: &str = "menu.pos.left";
pub const MENU_POS_RIGHT: &str = "menu.pos.right";
pub const MENU_POS_CENTER: &str = "menu.pos.center";
pub const MENU_LOCK: &str = "menu.lock";
pub const MENU_SETTINGS: &str = "menu.settings";
pub const MENU_QUIT: &str = "menu.quit";

/// Build the shared menu against live state. The pill rect is refreshed
/// from the live window first — a just-finished drag must check the
/// edge it actually landed on (same read `window_bar_edge` makes).
pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let state = app.state::<AppState>();
    let (hotkeys, locked) = {
        let cfg = state.config.lock();
        (cfg.hotkeys.clone(), cfg.window.bar_locked)
    };
    let accel = |action: &str| hotkeys.get(action).cloned();
    let capture_running = state
        .capture
        .lock()
        .as_ref()
        .is_some_and(MacosCapture::is_running);
    let listen_live = state.listen.status().is_listening();
    let edge = {
        let mut pool = state.pool.lock();
        pool.refresh_bar_rect();
        pool.bar_edge()
    };

    let ask = MenuItem::with_id(app, MENU_ASK, "Start Conversation", true, accel("toggle_input"))?;
    let capture = MenuItem::with_id(
        app,
        MENU_CAPTURE,
        if capture_running {
            "Stop Screen Recording"
        } else {
            "Start Screen Recording"
        },
        true,
        accel("toggle_capture"),
    )?;
    // Start-only per spec — a live session disables the item rather
    // than toggling it off.
    let listen = MenuItem::with_id(
        app,
        MENU_LISTEN,
        "Start Listening",
        !listen_live,
        accel("start_listen"),
    )?;
    let history = MenuItem::with_id(app, MENU_HISTORY, "History", true, accel("show_history"))?;
    let sep1 = PredefinedMenuItem::separator(app)?;

    let mut edge_items: Vec<CheckMenuItem<Wry>> = Vec::new();
    for (id, label, dir) in [
        (MENU_POS_TOP, "Top", Dir::Up),
        (MENU_POS_BOTTOM, "Bottom", Dir::Down),
        (MENU_POS_LEFT, "Left", Dir::Left),
        (MENU_POS_RIGHT, "Right", Dir::Right),
    ] {
        edge_items.push(CheckMenuItem::with_id(
            app,
            id,
            label,
            true,
            edge == dir,
            None::<&str>,
        )?);
    }
    let pos_sep = PredefinedMenuItem::separator(app)?;
    let center = MenuItem::with_id(app, MENU_POS_CENTER, "Center", true, None::<&str>)?;
    let position = Submenu::with_id(app, MENU_POSITION, "Position", true)?;
    {
        let mut items: Vec<&dyn IsMenuItem<Wry>> =
            edge_items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
        items.push(&pos_sep);
        items.push(&center);
        position.append_items(&items)?;
    }

    let lock = CheckMenuItem::with_id(
        app,
        MENU_LOCK,
        "Lock Bar Position",
        true,
        locked,
        accel("toggle_lock"),
    )?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    // Same display-only accelerator the old tray item carried — the real
    // `Cmd+,` binding is the bar webview's keydown handler.
    let settings = MenuItem::with_id(app, MENU_SETTINGS, "Settings", true, Some("CmdOrCtrl+,"))?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit Marvis", true, None::<&str>)?;

    Menu::with_items(
        app,
        &[
            &ask, &capture, &listen, &history, &sep1, &position, &lock,
            &sep2, &settings, &quit,
        ],
    )
}
```

- [ ] **Step:** `cargo check -p Marvis` after lib.rs `mod menus;` —
  errors only from not-yet-wired dispatchers; fix next task.

### Task 4: lib.rs — dispatch, helpers, refresh funnels, command

**Files:**

- Modify: `apps/native/src-tauri/src/lib.rs`

- [ ] **Step 1:** New event consts near `EV_BAR_TOGGLE_INPUT`:

```rust
/// Rust→bar events the shared menu + `start_listen`/`show_history`
/// hotkeys fire. The webview owns both surfaces — dispatch only emits.
const EV_BAR_START_LISTEN: &str = "bar:start-listen";
const EV_BAR_SHOW_HISTORY: &str = "bar:show-history";
```

- [ ] **Step 2:** Replace `tray_menu_dispatch` with `menu_dispatch`:

```rust
/// Map `menu.*` item ids onto the same actions the bar buttons,
/// commands, and hotkeys use. Registered once via `app.on_menu_event`
/// in `setup` — menu events broadcast to every listener, so this must
/// be the ONLY handler matching `menu.*` ids.
fn menu_dispatch()
    -> impl Fn(&AppHandle, tauri::menu::MenuEvent) + Send + Sync + 'static
{
    move |app, event| match event.id().as_ref() {
        menus::MENU_ASK => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_TOGGLE_INPUT, ());
        }
        menus::MENU_CAPTURE => toggle_capture(app),
        menus::MENU_LISTEN => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_START_LISTEN, ());
        }
        menus::MENU_HISTORY => {
            let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_SHOW_HISTORY, ());
        }
        menus::MENU_POS_TOP => snap_edge_and_refresh(app, windows::Dir::Up),
        menus::MENU_POS_BOTTOM => snap_edge_and_refresh(app, windows::Dir::Down),
        menus::MENU_POS_LEFT => snap_edge_and_refresh(app, windows::Dir::Left),
        menus::MENU_POS_RIGHT => snap_edge_and_refresh(app, windows::Dir::Right),
        menus::MENU_POS_CENTER => {
            app.state::<AppState>().pool.lock().recenter_bar();
            refresh_tray_menu(app);
        }
        menus::MENU_LOCK => {
            let locked = app.state::<AppState>().config.lock().window.bar_locked;
            set_bar_locked(app, !locked);
        }
        menus::MENU_SETTINGS => show_settings(app),
        menus::MENU_QUIT => app.exit(0),
        _ => {}
    }
}
```

- [ ] **Step 3:** New helpers (place near `start_capture`/`stop_capture`):

```rust
/// The `menu.capture` item + `toggle_capture` hotkey shared boundary:
/// stop when live, else the `capture_start` command's gate-checked
/// start (the check and `start_capture` share `gate_transition` so a
/// racing leave-transition can't interleave).
fn toggle_capture(app: &AppHandle) {
    let state = app.state::<AppState>();
    let running = state
        .capture
        .lock()
        .as_ref()
        .is_some_and(MacosCapture::is_running);
    if running {
        stop_capture(app);
        return;
    }
    let _transition = state.gate_transition.lock();
    if *state.gate.lock() != Gate::Main {
        log::warn!("toggle_capture dropped while gate != Main");
        return;
    }
    start_capture(app);
}

/// The `window.bar_locked` write shared by the Lock item and the
/// `toggle_lock` hotkey — save + `config:changed` broadcast, then the
/// tray menu's check refreshes.
fn set_bar_locked(app: &AppHandle, locked: bool) {
    let state = app.state::<AppState>();
    let updated = {
        let mut cfg = state.config.lock();
        cfg.window.bar_locked = locked;
        if let Err(e) = config::save(&cfg) {
            log::warn!("persist bar_locked failed: {e}");
            return;
        }
        cfg.clone()
    };
    let _ = app.emit("config:changed", &updated);
    refresh_tray_menu(app);
}

/// The menu Position snap path shared by dispatch and
/// `window_snap_edge`.
fn snap_edge_and_refresh(app: &AppHandle, dir: windows::Dir) {
    app.state::<AppState>().pool.lock().snap_edge(dir);
    refresh_tray_menu(app);
}

/// Rebuild the tray's copy of the shared menu — labels/checks track
/// live state. NEVER call while holding `pool`/`config`/`capture`/
/// `hotkeys` locks: `set_menu` blocks on the main thread, and
/// main-thread window-event handlers take `pool` (Resized).
fn refresh_tray_menu(app: &AppHandle) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    match menus::build(app) {
        Ok(menu) => {
            if let Err(e) = tray.set_menu(Some(menu)) {
                log::warn!("tray menu refresh failed: {e}");
            }
        }
        Err(e) => log::warn!("tray menu rebuild failed: {e}"),
    }
}
```

- [ ] **Step 4:** `hotkey_dispatch` gains the four arms:

```rust
hotkey::Action::ToggleInput => {
    let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_TOGGLE_INPUT, ());
}
hotkey::Action::ToggleCapture => toggle_capture(&app),
hotkey::Action::StartListen => {
    let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_START_LISTEN, ());
}
hotkey::Action::ShowHistory => {
    let _ = app.emit_to(windows::BAR_LABEL, EV_BAR_SHOW_HISTORY, ());
}
hotkey::Action::ToggleLock => {
    let locked = app.state::<AppState>().config.lock().window.bar_locked;
    set_bar_locked(&app, !locked);
}
```

- [ ] **Step 5:** Refresh funnels:
  - `emit_capture_state` — append `refresh_tray_menu(app);`
  - `emit_listen_state` — append `refresh_tray_menu(app);`
  - `persist_bar_position` — append `refresh_tray_menu(app);` at the end
  - `swap_hotkeys` — after the `slot` match, append `refresh_tray_menu(app);`
  - `config_set` — after the `config:changed` emit, append
    `refresh_tray_menu(&app);` (covers `window.bar_locked` writes)
  - `window_snap_edge`/`window_recenter` commands — take `app: AppHandle`,
    call `snap_edge_and_refresh` / `pool.recenter_bar()` + `refresh_tray_menu`

- [ ] **Step 6:** `config_set` writable key:

```rust
"window.bar_locked" => {
    cfg.window.bar_locked = value
        .as_bool()
        .ok_or("window.bar_locked must be a bool")?;
}
```

- [ ] **Step 7:** New command + registration:

```rust
/// The idle capsule's right-click menu — the webview gates the invoke
/// to its idle surface; this builds the shared menu fresh (live
/// labels/checks) and pops it at the cursor.
#[tauri::command]
fn bar_context_menu(app: AppHandle) -> Result<(), String> {
    let Some(bar) = app.state::<AppState>().pool.lock().bar().cloned() else {
        return Ok(());
    };
    let menu = menus::build(&app).map_err(|e| e.to_string())?;
    bar.popup_menu(&menu).map_err(|e| e.to_string())
}
```

Add `bar_context_menu` to `generate_handler!`.

- [ ] **Step 8:** `setup`: `tray::init(handle)?`-style call becomes
  `tray::init(handle)` inside the existing warn-and-continue block, then
  `handle.on_menu_event(menu_dispatch());` right after.

- [ ] **Step 9:** `cargo check -p Marvis && cargo test -p Marvis`.

### Task 5: tray.rs — strip the old menu

**Files:**

- Modify: `apps/native/src-tauri/src/tray.rs`

- [ ] `init(app)` loses the `on_menu` param and `MenuItem`/`Menu` imports;
  `pub const MENU_*` move to `menus.rs` (delete here). Body:

```rust
pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let menu = crate::menus::build(app)?;
    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(ICON_BYTES)?)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Marvis")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .build(app)?;
    Ok(())
}
```

- [ ] `cargo check -p Marvis`.

### Task 6: Bar.tsx — contextmenu trigger, lock suppression, new event handlers

**Files:**

- Modify: `apps/native/src/views/Bar.tsx`
- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/lib/events.ts`

- [ ] **Step 1 — commands.ts:**

```ts
/** `config.window.bar_locked` — `true` suppresses pointer drags. */
```

In `WindowPrefs` add `bar_locked?: boolean`. New export:

```ts
/** Pop the shared `menu.*` context menu at the cursor — the idle
 *  capsule's right-click. */
export const barContextMenu = () => invoke<void>('bar_context_menu');
```

- [ ] **Step 2 — events.ts:**

```ts
/** Emitted to the `bar` window only — the `start_listen` global hotkey
 * and the shared menu's Listen item (lib.rs dispatch). Start-only: a
 * live session makes it a no-op. */
export const EV_BAR_START_LISTEN = 'bar:start-listen';
/** `show_history` hotkey / `menu.history` — the card opens on the
 * session list. */
export const EV_BAR_SHOW_HISTORY = 'bar:show-history';
```

- [ ] **Step 3 — Bar.tsx state + handlers:**

```ts
const [locked, setLocked] = useState(false);
useEffect(() => {
  void configGet()
    .then((c) => setLocked(c.window.bar_locked ?? false))
    .catch(() => {});
}, []);
useTauriEvent<Config>(EV_CONFIG_CHANGED, (c) =>
  setLocked(c.window.bar_locked ?? false),
);
```

Extract `pressMic`'s listen-start tail into `startListenSession`
(identical body: `setListenWanted(true); setPinned('listen');
setListenViewing(null); windowSetChatOpen(true); listenStart()…finally
speechBusy=false`), call it from `pressMic`'s last branch, then:

```ts
// `bar:start-listen` (menu item + `start_listen` hotkey): start-only —
// a live session makes it a no-op, mirroring the disabled menu item.
// A live dictation owns the mic — stop it first (the mic button's
// "stop the other mode first" contract).
useTauriEvent(EV_BAR_START_LISTEN, () => {
  if (gate !== 'main' || speechBusy.current) {
    return;
  }
  if (listenState === 'listening' || listenState === 'paused') {
    return;
  }
  speechBusy.current = true;
  const stopping = dictation.stopIfActive();
  if (stopping !== null) {
    void stopping.then(startListenSession);
    return;
  }
  startListenSession();
});

// `bar:show-history` (menu item + `show_history` hotkey): the card
// opens on the session list.
useTauriEvent(EV_BAR_SHOW_HISTORY, () => {
  if (gate !== 'main') {
    return;
  }
  setPinned('history');
  void windowSetChatOpen(true).catch(() => {});
});
```

- [ ] **Step 4 — stage root element** gains both handlers:

```tsx
onMouseDownCapture={(e) => {
  // Position lock: Tauri drags off a document-level bubble `mousedown`
  // listener (window/scripts/drag.js) — stopping propagation on capture
  // covers every drag region in the window (pill chrome, card header,
  // gate cards) with no prop threading. Clicks unaffected.
  if (locked && e.button === 0) {
    e.stopPropagation();
  }
}}
onContextMenu={(e) => {
  // Idle capsule only — input pills, cards, and gate rows get no menu.
  if (gate !== 'main' || showInputRow || cardOpen) {
    return;
  }
  e.preventDefault();
  void barContextMenu().catch(() => {});
}}
```

- [ ] **Step 5:** `bunx tsc --noEmit` (or the app's typecheck script) +
  `bun run lint` if present.

### Task 7: HotkeysTab — new rebindable rows

**Files:**

- Modify: `apps/native/src/components/prefs/HotkeysTab.tsx`

- [ ] `ACTIONS` gains the four ids (labels matching menu wording):

```ts
const ACTIONS: { id: string; label: string }[] = [
  { id: 'toggle_input', label: 'Start to ask Marvis' },
  { id: 'toggle_capture', label: 'Screen recording' },
  { id: 'start_listen', label: 'Start listening' },
  { id: 'show_history', label: 'Show history' },
  { id: 'toggle_lock', label: 'Lock bar position' },
];
```

- [ ] Update the header doc comment and the `SUB` copy ("only global
  chord" → plural).

- [ ] Typecheck + `cargo test -p Marvis` + `bun run build` (frontend).

### Task 8: Manual verification

`bun run build:dev` → right-click idle capsule shows the menu; each item
acts; ⌘⌥R/T/H and ⇧⌘L work globally; Lock blocks drags but not clicks or
snaps; tray menu mirrors; Settings → Hotkeys rebinds persist after
restart.
