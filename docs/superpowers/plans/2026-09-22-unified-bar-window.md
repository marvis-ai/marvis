# Unified Morphing Bar Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Merge the `ask` and `listen` panel windows into the one `bar` window — the pill expands in place into a chat/listen card (and back), with true multi-turn ask sessions.

**Architecture:** `WindowPool` keeps a single resizable bar window. `bar_rect` stays the canonical PILL rect (136⇄480×64 — the existing capsule⇄input window morph is unchanged); while the card is open, `expand_dir` + `chat_height` derive the live card rect, and `refresh_bar_rect` derives the pill back so persistence/edge math never see card geometry. All ask events re-target `emit_to("bar", …)`. The webview learns card-open from its own window height (`set_chat_open` emits nothing) and grow-direction from the window's y-delta. Ask becomes multi-turn: the trailing 20 persisted `ai_messages` ride along as text-only history.

**Tech Stack:** Tauri 2 (Rust backend, `apps/native/src-tauri`), React 19 + Tailwind v4 frontend (`apps/native/src`), bun scripts, cargo test/clippy.

## Global Constraints

- The bar stays `resizable(false)` — all resizes are programmatic, animated by `windows/movement.rs` over 180 ms (`ANIM_DUR`).
- Pill modes: `BAR_IDLE_W=136` ⇄ `BAR_W=480`, `BAR_H=64` — unchanged. Card modes: `600×(64 + content_h)`, clamps `[64+40, min(900, free space)]`; `chat_height` default 480.
- Ask event names unchanged (`ask:state`/`ask:chunk`/`ask:done`/`ask:error`); emit target becomes `"bar"`.
- The `alert` (340×100) and `prefs` (720×520, decorated) windows are unchanged.
- `Main` gate still requires `onboarding_done` && screen permission; ask commands keep their gate guards.
- Components in `src/components/*`: arrow functions + named exports; icons use `Icon`-suffixed names via `@marvis/ui`.
- Use `bun` for all package/script commands; `bun run build` in `apps/native` runs `tsc` (the typecheck) + `vite build`.
- History cap: last **20** persisted messages, text-only, user/assistant roles only; only the new user turn may carry the frame image.

## Interpretation notes (decisions made when the spec was loose)

| Spec point | Decision taken here |
| --- | --- |
| "Idle window size 480×64" vs. the live 136⇄480 capsule morph (commit `a4abcd7`) | **Keep the morph** (user-confirmed). "480×64" names the input-mode pill; expansion animates from the live `bar_rect` whatever its width. |
| How the webview learns card-open (no emit) | `window.innerHeight > 64` — a `resize` listener is the single open/close signal for every path (Cmd+/, ask send, `ask_close`). Grow direction: compare `outerPosition().y` to the last collapsed y (`y` fixed → down, decreased → up). |
| Mic button → listen mode, `capture:permission-needed` → collapse | Neither can use toggle semantics (mic on an open chat must *switch*, not close; the permission collapse must not cancel the in-flight text-only ask). New thin command `window_set_chat_open(open)` exposes the pool primitive. |
| "the observer now measures the WHOLE card" | Observer lives in `Bar.tsx` — the card element is the shell's. It measures `stage.scrollHeight` (card + frost padding self-corrects, as before) and reports total window height via `window_adjust_height`. |
| Glass shape vs. card shape | `set_chat_open` re-applies the liquid-glass `corner_radius`: `BAR_H/2` (capsule) ⇄ `18` (card, matches `PANEL`'s `rounded-[18px]`). Same warn-only detached-thread pattern as `build_window`. |
| `session_end_active` must not create-then-end a session | New read-only `Db::session_active_id(kind)` (the SELECT half of `session_get_or_create_active`, refactored out). |

## File map

| File | Action | Responsibility |
| --- | --- | --- |
| `src-tauri/src/windows/layout.rs` | Modify | Delete `panel_rects` + its tests; add pure `expand_dir_for` / `expanded_rect` / `derive_pill_rect` + card consts |
| `src-tauri/src/windows/mod.rs` | Modify | Slim `WindowPool` to bar+alert+prefs; `chat_open`/`chat_height`/`expand_dir`; `set_chat_open`/`toggle_chat`/`adjust_height(px)`/`target_rect` |
| `src-tauri/src/ask.rs` | Modify | Emit to `"bar"`; `kick`→`set_chat_open(true)`; `close(app,pool)`; multi-turn history in `send_chain` |
| `src-tauri/src/storage.rs` | Modify | Extract `session_active_id` from `session_get_or_create_active` |
| `src-tauri/src/lib.rs` | Modify | Commands: `window_toggle_all`→toggle_chat, `window_adjust_height(height)`, +`window_set_chat_open`, +`session_end_active`, +`ask_current`; `enter_main`/`leave_main`/dispatch rewires |
| `src-tauri/src/hotkey.rs` | Modify | `ToggleVisibility` doc comment only |
| `src/lib/commands.ts` | Modify | `windowAdjustHeight(height)`; +`windowSetChatOpen`, +`sessionEndActive`, +`askCurrent` |
| `src/lib/events.ts` | Modify | Comments only (`ask` window → `bar` window) |
| `src/components/ChatSection.tsx` | Create | Multi-turn conversation section (from `views/AskPanel.tsx`) |
| `src/components/ListenSection.tsx` | Create | Phase-2 stub section (from `views/ListenPanel.tsx`) |
| `src/views/Bar.tsx` | Modify | Becomes the shell: stage + card + BarRow + sections |
| `src/App.tsx` | Modify | Routes shrink to `bar`/`alert`/`prefs` |
| `src/views/AskPanel.tsx`, `src/views/ListenPanel.tsx` | Delete | Superseded by the card sections |

---

### Task 1: Expansion math in `windows/layout.rs`

Pure functions only — the whole expand/collapse/derive geometry, unit-testable without a runtime. Additive (the pool switches to them in Task 2); `panel_rects` and its tests stay until Task 2 deletes them with their only consumer.

**Files:**

- Modify: `apps/native/src-tauri/src/windows/layout.rs`

**Interfaces:**

- Produces (consumed by `windows/mod.rs` in Task 2):
  - `pub const EXPANDED_W: f64` (600), `pub const MAX_EXPANDED_H: f64` (900), `pub const MIN_CHAT_H: f64` (40)
  - `pub fn expand_dir_for(bar: Rect, work: Rect) -> Dir`
  - `pub fn expanded_rect(bar: Rect, dir: Dir, chat_h: f64, work: Rect) -> Rect`
  - `pub fn derive_pill_rect(card: Rect, pill_w: f64, pill_h: f64, dir: Dir) -> Rect`

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `layout.rs` (the existing `WORK` const is reused; `PILL` is new):

```rust
    /// The input-mode pill (136-capsule expands identically — only
    /// center-x and the anchored edge matter).
    const PILL: Rect = Rect {
        x: 100.0,
        y: 33.0,
        w: 480.0,
        h: 64.0,
    };

    #[test]
    fn expand_dir_grows_toward_the_larger_free_side() {
        // Top-docked (bar near work top): more room below → Down.
        assert_eq!(expand_dir_for(PILL, WORK), Dir::Down);
        // Bottom-docked: more room above → Up.
        let bottom = Rect {
            y: WORK.bottom() - PILL.h - 12.0,
            ..PILL
        };
        assert_eq!(expand_dir_for(bottom, WORK), Dir::Up);
        // Exact middle → Down (tie goes down).
        let mid = Rect {
            y: WORK.y + (WORK.h - PILL.h) / 2.0,
            ..PILL
        };
        assert_eq!(expand_dir_for(mid, WORK), Dir::Down);
    }

    #[test]
    fn expanded_rect_grow_down_anchors_top_and_recenters() {
        let r = expanded_rect(PILL, Dir::Down, 200.0, WORK);
        assert_eq!(r.y, PILL.y);
        assert_eq!(r.w, 600.0);
        assert_eq!(r.center_x(), PILL.center_x());
        assert_eq!(r.h, PILL.h + 200.0);
    }

    #[test]
    fn expanded_rect_grow_up_anchors_bottom() {
        // PILL.bottom()=97, WORK.y=25 → free above is 72 < the 104 floor,
        // so h floors at 104 and the anchored bottom edge still holds
        // (y = 97−104 = −7: the floor case may overflow the far edge —
        // spec fixes the anchor, not the far edge).
        let r = expanded_rect(PILL, Dir::Up, 200.0, WORK);
        assert_eq!(r.bottom(), PILL.bottom());
        assert_eq!(r.h, PILL.h + MIN_CHAT_H);
        assert_eq!(r.y, PILL.bottom() - (PILL.h + MIN_CHAT_H));
        assert_eq!(r.center_x(), PILL.center_x());
    }

    #[test]
    fn expanded_rect_clamps_height_to_900_and_free_space() {
        // A work area tall enough that the 900 ceiling actually binds
        // (WORK's 875 height caps free space at 867 first).
        let tall = Rect { x: 0.0, y: 0.0, w: 1440.0, h: 1200.0 };
        let pill = Rect { y: 33.0, ..PILL };
        let r = expanded_rect(pill, Dir::Down, 5000.0, tall);
        assert_eq!(r.h, 900.0);
        // Inside WORK, free space below the pill caps first: 900−33=867.
        let r = expanded_rect(PILL, Dir::Down, 5000.0, WORK);
        assert_eq!(r.h, WORK.bottom() - PILL.y);
        // Bar near the work bottom growing down: free space caps below 900.
        let low = Rect {
            y: WORK.bottom() - 300.0,
            ..PILL
        };
        let r = expanded_rect(low, Dir::Down, 5000.0, WORK);
        assert_eq!(r.h, 300.0);
        // …but never below the 104 floor even when free space is smaller.
        let floor = Rect {
            y: WORK.bottom() - 80.0,
            ..PILL
        };
        let r = expanded_rect(floor, Dir::Down, 5000.0, WORK);
        assert_eq!(r.h, PILL.h + MIN_CHAT_H);
    }

    #[test]
    fn expanded_rect_enforces_min_content_height() {
        let r = expanded_rect(PILL, Dir::Down, 0.0, WORK);
        assert_eq!(r.h, PILL.h + MIN_CHAT_H); // 64 + 40 = 104
    }

    #[test]
    fn expanded_rect_width_recenter_clamps_inside_work_area() {
        // Pill hugging the right edge: 600-wide recenter would overflow.
        let right = Rect {
            x: WORK.right() - PILL.w - 12.0,
            ..PILL
        };
        let r = expanded_rect(right, Dir::Down, 200.0, WORK);
        assert_eq!(r.right(), WORK.right());
        assert_eq!(r.w, 600.0);
    }

    #[test]
    fn derive_pill_rect_inverts_expanded_rect() {
        for dir in [Dir::Down, Dir::Up] {
            let card = expanded_rect(PILL, dir, 200.0, WORK);
            let pill = derive_pill_rect(card, PILL.w, PILL.h, dir);
            assert_eq!(pill, PILL, "dir {dir:?}");
        }
        // A dragged card derives a dragged pill (same center-x + anchor).
        let dragged = Rect {
            x: 500.0,
            y: 400.0,
            w: 600.0,
            h: 264.0,
        };
        let pill = derive_pill_rect(dragged, 136.0, 64.0, Dir::Up);
        assert_eq!(pill.x, dragged.center_x() - 68.0);
        assert_eq!(pill.bottom(), dragged.bottom());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd apps/native/src-tauri && cargo test windows::layout`
Expected: compile FAIL — `expand_dir_for`, `expanded_rect`, `derive_pill_rect`, `EXPANDED_W`/`MIN_CHAT_H` unresolved.

- [ ] **Step 3: Implement the functions + consts**

Add after `EDGE_MARGIN` in `layout.rs` (imports become `use super::{Dir, Rect};` — `Panel`, `BTreeMap`, `BTreeSet` are still used by `panel_rects` until Task 2, so keep them for now):

```rust
/// Expanded-card width (spec §Window model: chat/listen are 600 wide).
pub const EXPANDED_W: f64 = 600.0;
/// Total window height ceiling while expanded (spec: ≤ 900).
pub const MAX_EXPANDED_H: f64 = 900.0;
/// Minimum chat content height (spec: 40 → window ≥ pill + 40).
pub const MIN_CHAT_H: f64 = 40.0;

/// Which side the expanded card grows toward — the larger free side of
/// the bar's work area (top-docked grows down, bottom-docked grows up;
/// a tie goes down).
pub fn expand_dir_for(bar: Rect, work: Rect) -> Dir {
    if bar.y - work.y > work.bottom() - bar.bottom() {
        Dir::Up
    } else {
        Dir::Down
    }
}

/// The live window rect while expanded: `EXPANDED_W` wide, recentred on
/// the pill's center-x and clamped inside `work`; total height
/// `bar.h + chat_h` clamped to `[bar.h + MIN_CHAT_H, min(MAX_EXPANDED_H,
/// free space in `dir`)]`. The anchored edge is fixed — grow-down keeps
/// `bar.y`, grow-up keeps `bar.bottom()`.
pub fn expanded_rect(bar: Rect, dir: Dir, chat_h: f64, work: Rect) -> Rect {
    let min = bar.h + MIN_CHAT_H;
    let free = if dir == Dir::Up {
        bar.bottom() - work.y
    } else {
        work.bottom() - bar.y
    };
    let max = MAX_EXPANDED_H.min(free).max(min);
    let h = (bar.h + chat_h).clamp(min, max);
    let y = if dir == Dir::Up {
        bar.bottom() - h
    } else {
        bar.y
    };
    // Only the recentered x clamps inside `work` — a y-position clamp
    // would move the anchored edge (breaking `derive_pill_rect`'s
    // round-trip) whenever the floor height overflows the far edge.
    let x = if EXPANDED_W >= work.w {
        work.x
    } else {
        (bar.center_x() - EXPANDED_W / 2.0).clamp(work.x, work.right() - EXPANDED_W)
    };
    Rect { x, y, w: EXPANDED_W, h }
}

/// The pill rect implied by the live expanded window — the inverse of
/// [`expanded_rect`]: `pill_w`×`pill_h`, centred on the card's center-x,
/// pinned to the anchored edge (`dir`). Lets `refresh_bar_rect` recover
/// the canonical pill while a card is up so persistence/edge math never
/// see card geometry.
pub fn derive_pill_rect(card: Rect, pill_w: f64, pill_h: f64, dir: Dir) -> Rect {
    Rect {
        x: card.center_x() - pill_w / 2.0,
        y: if dir == Dir::Up {
            card.bottom() - pill_h
        } else {
            card.y
        },
        w: pill_w,
        h: pill_h,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd apps/native/src-tauri && cargo test windows::layout`
Expected: PASS — all existing tests plus the 7 new ones.

- [ ] **Step 5: Commit**

```bash
git add apps/native/src-tauri/src/windows/layout.rs
git commit -m "feat(native): expand/derive rect math for the unified card"
```

---

### Task 2: `WindowPool` slims to one window + all Rust rewires

The atomic cutover: `Panel`, panels, stacking, and `toggle_all` die; `chat_open`/`chat_height`/`expand_dir` arrive; every mutator animates to `target_rect()`. `lib.rs` and `ask.rs` rewire in the same commit — `Panel` is referenced there, so splitting would break the build.

**Files:**

- Modify: `apps/native/src-tauri/src/windows/layout.rs` (delete `panel_rects`, its 4 tests, `Panel`/`BTreeMap`/`BTreeSet` imports)
- Modify: `apps/native/src-tauri/src/windows/mod.rs`
- Modify: `apps/native/src-tauri/src/ask.rs` (plumbing only — multi-turn is Task 5)
- Modify: `apps/native/src-tauri/src/lib.rs`
- Modify: `apps/native/src-tauri/src/hotkey.rs` (one doc comment)

**Interfaces:**

- Consumes: Task 1's `expand_dir_for`, `expanded_rect`, `derive_pill_rect`, `EXPANDED_W`, `MIN_CHAT_H` (via `layout::` paths).
- Produces:
  - `WindowPool::set_chat_open(&mut self, app: &AppHandle, open: bool)` — open is a no-op off-`Main`; close always proceeds.
  - `WindowPool::toggle_chat(&mut self, app: &AppHandle)`
  - `WindowPool::adjust_height(&mut self, px: f64)` — `px` = desired TOTAL window height, expanded-only.
  - `AppState::gate_is_main(&self) -> bool` (pub(crate)).
  - `windows::BAR_LABEL: &str` = `"bar"`.
  - `AskService::close(&self, app: &AppHandle, pool: &Mutex<WindowPool>)`.
  - Commands: `window_set_chat_open(open: bool)` (new), `window_adjust_height(height: f64)` (`name` dropped), `window_toggle_all()` (now takes `AppHandle`).

- [ ] **Step 1: `layout.rs` — delete `panel_rects` and its tests**

Remove `pub fn panel_rects` entirely, the four tests that call it (`ask_is_centered_under_bar`, `listen_offsets_left_of_ask_when_both_visible`, `listen_is_centered_when_ask_hidden`, `both_visible_do_not_overlap`), the `vis()` helper, and the `BAR` const if left unused (the `snap_edge`/`clamp` tests still use `BAR` and `WORK` — check: `snap_right_lands_twelve_px_from_edge`, `snap_left_up_down_use_named_edge`, and the clamp tests all use `BAR`/`WORK`, so keep both). Imports collapse to `use super::{Dir, Rect};` — drop `BTreeMap`, `BTreeSet`, `Panel`.

- [ ] **Step 2: `mod.rs` — struct, consts, deleted symbols**

Delete the `Panel` enum and its whole `impl` block (lines ~58-115). Delete `MIN_PANEL_H`. Add next to the other consts:

```rust
/// Bar window label — the ask event target now that the chat card lives
/// inside the bar window (`emit_to(BAR_LABEL, "ask:*", …)`).
pub const BAR_LABEL: &str = "bar";
/// Card content height until the webview's first `window_adjust_height`
/// report (spec default).
const CHAT_DEFAULT_H: f64 = 480.0;
/// The expanded card's corner radius — matches the CSS card's
/// `rounded-[18px]`; the glass shape follows it while a card is up.
const CARD_RADIUS: f64 = 18.0;
```

`use layout::{clamp_to_work_area, panel_rects};` becomes:

```rust
use layout::{clamp_to_work_area, derive_pill_rect, expand_dir_for, expanded_rect};
```

Drop `use std::collections::{BTreeMap, BTreeSet};` (keep `AtomicU64`/`Ordering` — `BAR_MOVE_GEN` needs them).

The struct:

```rust
/// Owns the bar window (which now hosts chat/listen as expanded card
/// modes), the alert toast, and the prefs window.
///
/// `bar_rect` is the canonical PILL rect — source of truth for
/// position, edge detection, and `Moved`-debounce persistence. While
/// the card is open the live window IS the card; `expand_dir` +
/// `chat_height` derive the card rect from the pill, and
/// `refresh_bar_rect` derives the pill back, so nothing downstream ever
/// records card geometry.
pub struct WindowPool {
    bar: Option<WebviewWindow>,
    /// Alert toast — built with the bar (it never joins the gate's
    /// lifecycle; it must exist before `Main` to report failures).
    alert: Option<WebviewWindow>,
    /// The decorated preferences window (settings + onboarding). Built
    /// lazily on first `show_prefs` — it is NOT part of `Main`.
    prefs: Option<WebviewWindow>,
    /// The mode `prefs` was last opened in (`"settings"|"onboarding"`) —
    /// the `prefs_mode` command returns it so a `prefs:mode` emit that
    /// raced a still-loading webview isn't lost.
    prefs_mode: String,
    /// Whether the unified card (chat or listen mode) is open.
    chat_open: bool,
    /// Last reported card CONTENT height — window = `BAR_H + this`.
    chat_height: f64,
    /// Which way the card grows — computed at expand time from free
    /// space (`Up` toward the top edge, `Down` toward the bottom).
    expand_dir: Dir,
    /// Last known pill rect (logical). Refreshed from the live window
    /// (deriving the pill when expanded) so user drags aren't lost.
    bar_rect: Rect,
}
```

Init both constructors (`create_bar_only`, `new_empty`) with `chat_open: false, chat_height: CHAT_DEFAULT_H, expand_dir: Dir::Down` — and drop `panels`/`visible`/`remembered`/`heights` from both.

Delete these methods outright: `create_feature_windows`, `panel_window`, `is_visible`, `show`, `hide`, `toggle_all`, `layout_targets`, `height_of`, `restack`. Delete the now-unused `Panel` import in the `use` block.

- [ ] **Step 3: `mod.rs` — new + rewritten methods**

```rust
    /// The window's animated destination: the derived card rect while
    /// expanded, the pill rect otherwise. Every mutator operates on
    /// `bar_rect` (the canonical pill) and animates to THIS — an
    /// expanded card moves as a unit under snaps/recenters/reclamps.
    fn target_rect(&self) -> Rect {
        if self.chat_open {
            expanded_rect(
                self.bar_rect,
                self.expand_dir,
                self.chat_height,
                self.bar_work_area(),
            )
        } else {
            self.bar_rect
        }
    }

    /// `window_set_chat_open` / `Cmd+/` / ask pre-flight: animate the bar
    /// window collapsed ⇄ expanded per §Expansion. Emits nothing — the
    /// webview learns the mode from `ask:*` or its own action (window
    /// height). OPEN is a no-op off-`Main` (mirrors panels not existing
    /// pre-`Main`); CLOSE always proceeds — `leave_main` collapses after
    /// the gate has already flipped.
    pub fn set_chat_open(&mut self, app: &AppHandle, open: bool) {
        if open == self.chat_open {
            return;
        }
        if open && !app.state::<crate::AppState>().gate_is_main() {
            log::warn!("windows::set_chat_open: open dropped while gate != Main");
            return;
        }
        let Some(bar) = self.bar.clone() else { return };
        // Refresh BEFORE the flag flips: collapsed it reads the pill
        // directly; expanded it derives the pill from the (possibly
        // dragged) card so collapse lands where the user left it.
        self.refresh_bar_rect();
        if open {
            let work = self.bar_work_area();
            self.expand_dir = expand_dir_for(self.bar_rect, work);
            self.chat_open = true;
            let target = self.target_rect();
            movement::animate(&bar, target, ANIM_DUR);
            set_glass_radius(app, &bar, CARD_RADIUS);
        } else {
            self.chat_open = false;
            movement::animate(&bar, self.bar_rect, ANIM_DUR);
            set_glass_radius(app, &bar, BAR_H / 2.0);
        }
    }

    /// `Cmd+/`/`window_toggle_all`/tray Toggle: open ⇄ close the card.
    pub fn toggle_chat(&mut self, app: &AppHandle) {
        self.set_chat_open(app, !self.chat_open);
    }

    /// `window_adjust_height(px)`: `px` is the desired TOTAL window
    /// height (the frontend measures the whole card). Expanded-only —
    /// ignored when the card is closed. `expanded_rect` clamps to
    /// `[BAR_H + 40, min(900, free space)]` keeping the anchored edge
    /// fixed; the clamped result is recorded for later expands.
    pub fn adjust_height(&mut self, px: f64) {
        if !px.is_finite() {
            log::warn!("windows::adjust_height: non-finite height {px}");
            return;
        }
        if !self.chat_open {
            log::warn!("windows::adjust_height: ignored while the card is closed");
            return;
        }
        self.refresh_bar_rect();
        let target = expanded_rect(
            self.bar_rect,
            self.expand_dir,
            px - BAR_H,
            self.bar_work_area(),
        );
        self.chat_height = target.h - BAR_H;
        if let Some(bar) = &self.bar {
            movement::animate(bar, target, ANIM_DUR);
        }
    }
```

`set_bar_expanded`: keep the body, change the tail to animate `self.target_rect()` and drop `self.restack()`:

```rust
        if let Some(bar) = &self.bar {
            movement::animate(bar, self.target_rect(), ANIM_DUR);
        }
```

`snap_edge`: same tail — `movement::animate(bar, self.target_rect(), ANIM_DUR)`, drop `self.restack()`.

`recenter_bar`: same — animate `self.target_rect()`, drop `self.restack()`.

`reclamp`: replace `set_rect(&bar, clamped); self.restack();` with:

```rust
        if clamped != self.bar_rect {
            self.bar_rect = clamped;
            let target = self.target_rect();
            set_rect(&bar, target);
        }
```

`show_alert`: the anchor becomes `target_rect()` — under the expanded card when open, under the pill otherwise (for grow-up they coincide):

```rust
        self.reclamp();
        let anchor = self.target_rect();
        let target = clamp_to_work_area(
            Rect {
                x: anchor.center_x() - ALERT_W / 2.0,
                y: anchor.bottom() + layout::PANEL_PAD,
                w: ALERT_W,
                h: ALERT_H,
            },
            self.bar_work_area(),
        );
```

`refresh_bar_rect` derives the pill while expanded:

```rust
    /// Pull the live bar rect from the OS so user drags stay
    /// authoritative. While the card is open the live rect IS the card —
    /// derive the canonical pill back via `expand_dir` (keeping the
    /// stored pill width) so persistence/edge math never see card
    /// geometry.
    pub(crate) fn refresh_bar_rect(&mut self) {
        let Some(bar) = &self.bar else { return };
        let Some(r) = window_rect(bar) else { return };
        self.bar_rect = if self.chat_open {
            derive_pill_rect(r, self.bar_rect.w, BAR_H, self.expand_dir)
        } else {
            r
        };
    }
```

Add the free helper near `build_window` (same detached-thread rationale — `set_effect` blocks on the main queue and callers hold the pool lock):

```rust
/// Re-apply the liquid-glass corner radius on a live window — capsule
/// (`BAR_H/2`) ⇄ card (`CARD_RADIUS`). Same warn-only detached-thread
/// pattern as `build_window`: the call blocks on the main queue and the
/// pool lock must not be held across the wait.
fn set_glass_radius(app: &AppHandle, win: &WebviewWindow, corner_radius: f64) {
    let app = app.clone();
    let win = win.clone();
    std::thread::spawn(move || {
        if let Err(e) = app.liquid_glass().set_effect(
            &win,
            LiquidGlassConfig {
                corner_radius,
                ..Default::default()
            },
        ) {
            log::warn!("windows: liquid glass radius update failed: {e}");
        }
    });
}
```

Also update `create_bar_only`'s `build_window(app, "bar", …)` to use `BAR_LABEL` for the label (consistent with `ALERT_LABEL`/`PREFS_LABEL` style).

- [ ] **Step 4: `ask.rs` — emit target + pool calls (plumbing only)**

- `use crate::windows::{Panel, WindowPool};` → `use crate::windows::{BAR_LABEL, WindowPool};`
- In `kick`: `deps.pool.lock().show(Panel::Ask);` → `deps.pool.lock().set_chat_open(app, true);` (keep the "show first so pre-flight errors render" comment, updated to "expand first").
- In the emit closure: `app.emit_to("ask", name, payload)` → `app.emit_to(BAR_LABEL, name, payload)`.
- In `pre_spawn_error`: all three `app.emit_to("ask", …)` → `app.emit_to(BAR_LABEL, …)`.
- `close` signature and body:

```rust
    /// `ask_close`: cancel the in-flight stream (its `select!` arm emits
    /// the final `ask:state{idle}`), mint a fresh token for the next run,
    /// reset state, collapse the card. Emits nothing itself.
    ///
    /// Order matters: replace the token BEFORE flipping state to Idle —
    /// a `send` gated on `Idle` must never clone the cancelled token.
    pub fn close(&self, app: &AppHandle, pool: &Mutex<WindowPool>) {
        self.cancel.lock().cancel();
        *self.cancel.lock() = CancellationToken::new();
        *self.state.lock() = AskState::Idle;
        pool.lock().set_chat_open(app, false);
    }
```

- Module doc: `emit_to` lines now say "emitted to the `bar` window"; `Panel::Ask` references become "the card". Light touch — fix only lines that name the ask window/panel.

- [ ] **Step 5: `lib.rs` — rewires + `window_set_chat_open`**

- `use windows::{Panel, WindowPool};` → `use windows::WindowPool;`
- `AppState` gains:

```rust
    /// The gate read `WindowPool::set_chat_open` needs — opening the
    /// card is `Main`-only.
    pub(crate) fn gate_is_main(&self) -> bool {
        *self.gate.lock() == Gate::Main
    }
```

- `enter_main`: delete the `create_feature_windows` block (bar always exists — only `swap_hotkeys` + capture remain). Update the doc comment: "`Main` entry: full hotkey set + capture."
- `leave_main`: `state.ask.close(&state.pool);` → `state.ask.close(app, &state.pool);`; delete the `for panel in Panel::ALL { pool.hide(panel) }` block; add `state.pool.lock().hide_alert();` before `swap_hotkeys(app, false);`.
- `hotkey_dispatch`: `hotkey::Action::ToggleVisibility => state.pool.lock().toggle_all(),` → `state.pool.lock().toggle_chat(&app),`. Fix the stale comment on `NextStep`/`ScreenOnly` ("the ask panel doesn't even exist" → "the card can't open").
- `tray_menu_dispatch`: `pool.lock().toggle_all()` → `pool.lock().toggle_chat(app)`. Update its doc comment (`window_toggle_all` wording stays).
- Commands:

```rust
/// `Cmd+/` behaviour as a command: collapse/expand the unified card.
#[tauri::command]
fn window_toggle_all(app: AppHandle) {
    app.state::<AppState>().pool.lock().toggle_chat(&app);
}
```

```rust
/// `height` is the desired TOTAL window height (the frontend measures
/// the whole card) — the pool clamps and animates, anchored edge fixed.
/// Expanded-only.
#[tauri::command]
fn window_adjust_height(state: State<'_, AppState>, height: f64) {
    state.pool.lock().adjust_height(height);
}
```

```rust
/// Direct card open/close — the mic button's listen mode and the
/// `capture:permission-needed` collapse use it (toggle semantics would
/// close an open card when the user only wants to switch modes, and
/// `ask_close` would cancel an in-flight text-only ask).
#[tauri::command]
fn window_set_chat_open(app: AppHandle, open: bool) {
    app.state::<AppState>()
        .pool
        .lock()
        .set_chat_open(&app, open);
}
```

```rust
/// Cancel the in-flight stream and collapse the card.
#[tauri::command]
fn ask_close(app: AppHandle) {
    let state = app.state::<AppState>();
    state.ask.close(&app, &state.pool);
}
```

- `invoke_handler`: add `window_set_chat_open` (after `window_toggle_all`).

- [ ] **Step 6: `hotkey.rs` — doc comment**

`ToggleVisibility` variant comment `"toggle_visibility" — hide all panels / restore the remembered set.` → `"toggle_visibility" — collapse/expand the unified card (Cmd+/).`

- [ ] **Step 7: Run the full suite + clippy**

Run: `cd apps/native/src-tauri && cargo test`
Expected: PASS — layout expansion tests green; ask chain tests unchanged green (the emit target lives inside `send_chain`'s `emit` seam, untested by name); no `panel_rects` tests remain.

Run: `cd apps/native/src-tauri && cargo clippy`
Expected: no warnings (watch for: unused imports after deletions — `Panel`, `BTreeMap`, `BTreeSet`, `LogicalSize` may still be needed by `adjust_height`'s old body — it isn't anymore, but `set_rect` uses it; verify).

- [ ] **Step 8: Commit**

```bash
git add apps/native/src-tauri/src/windows/ apps/native/src-tauri/src/ask.rs apps/native/src-tauri/src/lib.rs apps/native/src-tauri/src/hotkey.rs
git commit -m "refactor(native): one morphing bar window — chat/listen become card modes"
```

---

### Task 3: `session_end_active` command

Powers ChatSection's "New chat" — ends the active `ask` session so the next send starts fresh. Needs a read-only active-session lookup (refactored out of `session_get_or_create_active`) so the command doesn't create-then-end a junk row.

**Files:**

- Modify: `apps/native/src-tauri/src/storage.rs`
- Modify: `apps/native/src-tauri/src/lib.rs`

**Interfaces:**

- Produces: `Db::session_active_id(&self, kind: &str) -> anyhow::Result<Option<i64>>`; command `session_end_active(kind: String) -> Result<bool, String>` (`true` = a session was ended).

- [ ] **Step 1: Write the failing test**

Append to `mod tests` in `storage.rs`:

```rust
    #[test]
    fn session_active_id_reads_without_creating() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        // None before any session — and must not create one.
        assert_eq!(db.session_active_id("ask").unwrap(), None);
        assert!(db.session_list().unwrap().is_empty());
        let sid = db.session_get_or_create_active("ask").unwrap();
        assert_eq!(db.session_active_id("ask").unwrap(), Some(sid));
        db.session_end(sid).unwrap();
        assert_eq!(db.session_active_id("ask").unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/native/src-tauri && cargo test storage::tests::session_active_id`
Expected: compile FAIL — `session_active_id` doesn't exist.

- [ ] **Step 3: Implement**

Refactor `session_get_or_create_active` — the SELECT half becomes the new method:

```rust
    /// The most recently active open (`ended_at IS NULL`) session of
    /// `kind`, or `None` — the read half of
    /// `session_get_or_create_active` for callers that must not create
    /// (e.g. `session_end_active`).
    pub fn session_active_id(&self, kind: &str) -> anyhow::Result<Option<i64>> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT id FROM sessions
                 WHERE type = ?1 AND ended_at IS NULL
                 ORDER BY last_active_at DESC, id DESC
                 LIMIT 1",
                [kind],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The most recently active open (`ended_at IS NULL`) session of `kind`,
    /// or a fresh row when none exists.
    pub fn session_get_or_create_active(&self, kind: &str) -> anyhow::Result<i64> {
        if let Some(id) = self.session_active_id(kind)? {
            return Ok(id);
        }
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO sessions (type, title, started_at, ended_at, last_active_at)
             VALUES (?1, NULL, ?2, NULL, ?2)",
            params![kind, now()],
        )?;
        Ok(conn.last_insert_rowid())
    }
```

- [ ] **Step 4: Add the command in `lib.rs` and register it**

In the sessions commands section:

```rust
/// "New chat": end the active session of `kind` (`"ask"`) so the next
/// send starts a fresh conversation. `true` when one was ended, `false`
/// when none was open (no junk row created).
#[tauri::command]
fn session_end_active(state: State<'_, AppState>, kind: String) -> Result<bool, String> {
    match state.db.session_active_id(&kind).map_err(|e| e.to_string())? {
        Some(id) => {
            state.db.session_end(id).map_err(|e| e.to_string())?;
            Ok(true)
        }
        None => Ok(false),
    }
}
```

Add `session_end_active` to `invoke_handler` after `session_delete`.

- [ ] **Step 5: Run tests**

Run: `cd apps/native/src-tauri && cargo test storage`
Expected: PASS — new test green, `get_or_create_active_reuses_then_recreates` still green through the refactor.

- [ ] **Step 6: Commit**

```bash
git add apps/native/src-tauri/src/storage.rs apps/native/src-tauri/src/lib.rs
git commit -m "feat(native): session_end_active command for New chat"
```

---

### Task 4: `ask_current` command

A re-expanded chat resyncs an in-flight run — the persisted session covers completed turns; this reports the live tail (`state`, `question`, `response`).

**Files:**

- Modify: `apps/native/src-tauri/src/ask.rs`
- Modify: `apps/native/src-tauri/src/lib.rs`

**Interfaces:**

- Produces: `AskState::as_str(&self) -> &'static str`; command `ask_current() -> {"state": "idle"|"loading"|"streaming", "question": string, "response": string, "error": {...}|null}` (serde_json::Value) — the `error` key was added by the final-review fix (cold-open `ask:error` resync).

- [ ] **Step 1: Write the failing test**

Append to `mod tests` in `ask.rs`:

```rust
    #[test]
    fn ask_state_as_str_matches_the_wire_names() {
        assert_eq!(AskState::Idle.as_str(), "idle");
        assert_eq!(AskState::Loading.as_str(), "loading");
        assert_eq!(AskState::Streaming.as_str(), "streaming");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/native/src-tauri && cargo test ask::tests::ask_state_as_str`
Expected: compile FAIL — `as_str` doesn't exist on `AskState`.

- [ ] **Step 3: Implement**

In `ask.rs`, on `AskState`:

```rust
impl AskState {
    /// The `ask:state` / `ask_current` wire value.
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Loading => "loading",
            Self::Streaming => "streaming",
        }
    }
}
```

`as_str` is module-private but the enum is `pub` — make it `pub(crate) fn as_str` so `lib.rs` can call it. Remove the three `#[allow(dead_code)]` attributes on `state()`, `current_response()`, `current_question()` (they gain a real consumer now).

In `lib.rs` (ask commands section, after `ask_send_screen_only`):

```rust
/// `{"state": "idle"|"loading"|"streaming", "question": ..., "response":
/// ...}` — the live tail a re-expanded chat resyncs from (the persisted
/// session already carries every completed turn).
#[tauri::command]
fn ask_current(state: State<'_, AppState>) -> serde_json::Value {
    json!({
        "state": state.ask.state().as_str(),
        "question": state.ask.current_question(),
        "response": state.ask.current_response(),
    })
}
```

Add `ask_current` to `invoke_handler` after `ask_send_screen_only`.

- [ ] **Step 4: Run tests + clippy**

Run: `cd apps/native/src-tauri && cargo test ask:: && cargo clippy`
Expected: PASS, no warnings (no more dead-code allows on the getters).

- [ ] **Step 5: Commit**

```bash
git add apps/native/src-tauri/src/ask.rs apps/native/src-tauri/src/lib.rs
git commit -m "feat(native): ask_current resync command"
```

---

### Task 5: Multi-turn `send_chain`

The LLM now receives the trailing persisted turns as context. History is read BEFORE the new user row persists, capped to the last 20, text-only; only the new user turn may carry the frame.

**Files:**

- Modify: `apps/native/src-tauri/src/ask.rs`

**Interfaces:**

- Consumes: `Db::ai_messages_for`, `ChatMessage::text`, `ChatMessage::user_with_image`, `Role::{User, Assistant}`.
- Produces: `send_chain`'s provider-facing `msgs` = `[system] + history(≤20) + [user(+frame)]`; helper `load_history(db, session_id) -> Vec<ChatMessage>`.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `ask.rs`:

```rust
    #[tokio::test]
    async fn send_chain_sends_prior_turns_as_text_only_history() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("ask").unwrap();
        db.ai_message_add(sid, "user", "first q").unwrap();
        db.ai_message_add(sid, "assistant", "first a").unwrap();
        let provider = MockProvider::new(vec![Behavior::Tokens(vec!["ok".into()])]);
        let calls = provider.calls();
        let (_events, emit) = recorder();
        let frame = fake_frame();
        let cancel = CancellationToken::new();

        send_chain(
            vec![candidate("openai", provider)],
            &db,
            &emit,
            "follow-up",
            Some(&frame),
            &cancel,
        )
        .await
        .unwrap();

        let calls = calls.lock();
        let msgs = &calls[0];
        // [system] + 2 history rows + new user turn.
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, Role::System);
        // History rides along text-only, oldest first, roles preserved.
        assert_eq!(msgs[1].role, Role::User);
        assert_eq!(msgs[1].content, vec![ContentPart::Text("first q".into())]);
        assert_eq!(msgs[2].role, Role::Assistant);
        assert_eq!(msgs[2].content, vec![ContentPart::Text("first a".into())]);
        assert!(!has_image(&msgs[1]));
        assert!(!has_image(&msgs[2]));
        // Only the NEW user turn carries the frame.
        assert_eq!(msgs[3].role, Role::User);
        assert!(has_image(&msgs[3]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn send_chain_history_tail_is_capped_at_20() {
        let dir = tmp_dir();
        let db = Db::at(dir.join("marvis.db")).unwrap();
        let sid = db.session_get_or_create_active("ask").unwrap();
        for i in 0..12 {
            db.ai_message_add(sid, "user", &format!("u{i}")).unwrap();
            db.ai_message_add(sid, "assistant", &format!("a{i}")).unwrap();
        }
        let provider = MockProvider::new(vec![Behavior::Tokens(vec!["ok".into()])]);
        let calls = provider.calls();
        let (_events, emit) = recorder();
        let cancel = CancellationToken::new();

        send_chain(
            vec![candidate("openai", provider)],
            &db,
            &emit,
            "new q",
            None,
            &cancel,
        )
        .await
        .unwrap();

        let calls = calls.lock();
        let msgs = &calls[0];
        // system + 20-row tail + new user turn = 22; tail starts at u2.
        assert_eq!(msgs.len(), 22);
        assert_eq!(msgs[1].content, vec![ContentPart::Text("u2".into())]);
        assert_eq!(msgs[20].content, vec![ContentPart::Text("a11".into())]);
        assert_eq!(msgs[21].content, vec![ContentPart::Text("new q".into())]);
        let _ = std::fs::remove_dir_all(&dir);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd apps/native/src-tauri && cargo test ask::tests::send_chain`
Expected: FAIL — `send_chain_sends_prior_turns_as_text_only_history` gets `msgs.len() == 2` (no history yet); `send_chain_history_tail_is_capped_at_20` fails the same way.

- [ ] **Step 3: Implement**

Add the cap const near `SCREEN_ONLY_PROMPT`:

```rust
/// Context window: only the trailing N persisted `ai_messages` ride
/// along with each ask (spec: last 20, text-only).
const HISTORY_TAIL: usize = 20;
```

In `send_chain`, replace `let session_id = persist_user_message(db, text);` with:

```rust
    // Order matters: history is read BEFORE the new user row persists —
    // the new turn is appended separately so it can carry the frame.
    let session_id = open_ask_session(db);
    let history = load_history(db, session_id);
    persist_user_message(db, session_id, text);
```

Rewrite the persistence helpers and add `load_history`:

```rust
/// The active `ask` session id, or `None` when the lookup itself fails —
/// history and the assistant row then have nowhere to go.
fn open_ask_session(db: &Db) -> Option<i64> {
    match db.session_get_or_create_active("ask") {
        Ok(sid) => Some(sid),
        Err(e) => {
            log::warn!("ask: session_get_or_create_active failed: {e}");
            None
        }
    }
}

/// The trailing persisted turns as text-only `ChatMessage`s — at most
/// `HISTORY_TAIL` rows, user/assistant roles only (images were never
/// persisted, so history is text by construction).
fn load_history(db: &Db, session_id: Option<i64>) -> Vec<ChatMessage> {
    let Some(sid) = session_id else { return Vec::new() };
    let rows = match db.ai_messages_for(sid) {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!("ask: history load failed: {e}");
            return Vec::new();
        }
    };
    rows.iter()
        .skip(rows.len().saturating_sub(HISTORY_TAIL))
        .filter_map(|m| match m.role.as_str() {
            "user" => Some(ChatMessage::text(Role::User, m.content.clone())),
            "assistant" => Some(ChatMessage::text(Role::Assistant, m.content.clone())),
            _ => None,
        })
        .collect()
}

/// The new user row, next to its session. `None` session (lookup
/// failed) skips the write — the stream must not die on a storage
/// hiccup.
fn persist_user_message(db: &Db, session_id: Option<i64>, text: &str) {
    let Some(sid) = session_id else {
        return;
    };
    if let Err(e) = db.ai_message_add(sid, "user", text) {
        log::warn!("ask: failed to persist user message: {e}");
    }
}
```

`build_messages` gains the history slot:

```rust
/// `[system] + history + [user]` — history rows are text-only; only the
/// new user turn pairs `text` with the frame's JPEG when one was
/// captured (and drops it on the multimodal retry).
fn build_messages(history: &[ChatMessage], text: &str, frame: Option<&Frame>) -> Vec<ChatMessage> {
    let mut msgs = Vec::with_capacity(history.len() + 2);
    msgs.push(ChatMessage::text(Role::System, system_prompt("")));
    msgs.extend(history.iter().cloned());
    msgs.push(match frame {
        Some(f) => ChatMessage::user_with_image(text, f.jpeg.clone()),
        None => ChatMessage::text(Role::User, text),
    });
    msgs
}
```

`stream_candidate` takes the history through so the retry rebuilds without the image:

```rust
async fn stream_candidate(
    provider: &dyn Provider,
    emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync),
    history: &[ChatMessage],
    text: &str,
    frame: Option<&Frame>,
    cancel: &CancellationToken,
) -> CandidateOutcome {
    let mut streaming = false;
    let mut msgs = build_messages(history, text, frame);
    let mut retried = false;
    loop {
        match stream_once(provider, &msgs, emit, cancel, &mut streaming).await {
            StreamOutcome::Done(full) => return CandidateOutcome::Done(full),
            StreamOutcome::Cancelled => return CandidateOutcome::Cancelled,
            StreamOutcome::Failed(e) => {
                // Vision-incapable model gets ONE retry without the frame.
                if !retried && frame.is_some() && e.is_multimodal() {
                    retried = true;
                    msgs = build_messages(history, text, None);
                    continue;
                }
                return CandidateOutcome::Failed(e);
            }
        }
    }
}
```

And the call site in `send_chain`: `stream_candidate(&*cand.provider, emit, &history, text, frame, cancel)`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd apps/native/src-tauri && cargo test ask::`
Expected: PASS — the two new tests green; all existing chain tests still green (empty history keeps `msgs[1]` = the user turn, so `calls[0][1]` assertions hold).

- [ ] **Step 5: Commit**

```bash
git add apps/native/src-tauri/src/ask.rs
git commit -m "feat(native): multi-turn ask — last 20 persisted messages as context"
```

---

### Task 6: `commands.ts` + `events.ts`

Typed wrappers for the new/changed commands; the one stale call site (`AskPanel`, deleted in Task 8) gets a one-line touch so `tsc` stays green between tasks.

**Files:**

- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/lib/events.ts`
- Modify: `apps/native/src/views/AskPanel.tsx` (one line — the file dies in Task 8)

**Interfaces:**

- Consumes: commands `session_end_active`, `ask_current`, `window_set_chat_open`, `window_adjust_height(height)`.
- Produces: `windowSetChatOpen(open)`, `sessionEndActive(kind)`, `askCurrent()`, `AskCurrent` type, `windowAdjustHeight(height)`.

- [ ] **Step 1: `commands.ts`**

In the ask section, after `askClose`:

```ts
/** `ask_current` return — the in-flight run's resync payload. */
export interface AskCurrent {
  state: 'idle' | 'loading' | 'streaming';
  question: string;
  response: string;
}

/** The live ask tail — a re-expanded chat resyncs from this. */
export const askCurrent = () => invoke<AskCurrent>('ask_current');
```

In the windows section, replace `windowAdjustHeight` and add `windowSetChatOpen`:

```ts
/** Reports the whole card's desired TOTAL window height — expanded
 * mode only; the backend clamps [104, min(900, free space)]. */
export const windowAdjustHeight = (height: number) =>
  invoke<void>('window_adjust_height', { height });

/** Direct card open/close — the mic button's listen mode and the
 * permission-needed collapse use it (toggle would close an open card
 * when the user only wants to switch modes). */
export const windowSetChatOpen = (open: boolean) =>
  invoke<void>('window_set_chat_open', { open });
```

(Delete the old two-arg `windowAdjustHeight` and its "Panels only" comment.)

In the sessions section, after `sessionDelete`:

```ts
/** End the active session of `kind` — ChatSection's "New chat". */
export const sessionEndActive = (kind: string) =>
  invoke<boolean>('session_end_active', { kind });
```

- [ ] **Step 2: `events.ts` comments**

`"Ask-window stream protocol (ask.rs), emitted to the ask window only."` → `"Ask stream protocol (ask.rs), emitted to the bar window only."` — same for the `EV_ASK_*` line. No code changes.

- [ ] **Step 3: Fix the stale `AskPanel` call**

`views/AskPanel.tsx` line ~118: `void windowAdjustHeight('ask', h).catch(() => {});` → `void windowAdjustHeight(h).catch(() => {});` (the file is deleted in Task 8 — this just keeps `tsc` green meanwhile).

- [ ] **Step 4: Verify**

Run: `cd apps/native && bun run build`
Expected: PASS (`tsc` clean + vite build).

- [ ] **Step 5: Commit**

```bash
git add apps/native/src/lib/commands.ts apps/native/src/lib/events.ts apps/native/src/views/AskPanel.tsx
git commit -m "feat(native): command wrappers for the unified card"
```

---

### Task 7: `ChatSection` + `ListenSection` components

The panel internals land as card sections. `ChatSection` becomes a true message list (history + live tail); `ListenSection` is the stub verbatim. Both are named-export arrow components in `src/components/` per project rules.

**Files:**

- Create: `apps/native/src/components/ChatSection.tsx`
- Create: `apps/native/src/components/ListenSection.tsx`

**Interfaces:**

- Consumes: `sessionList`, `sessionGet`, `sessionEndActive`, `askCurrent`, `askClose`, `modelGetSelected`, `windowShowSettings`; `EV_ASK_*` events; `AiMessage`/`Session`/`ModelSelection`/`AskCurrent` types.
- Produces: `export const ChatSection`, `export const ListenSection` — rendered by `Bar.tsx` (Task 8) inside the card.

- [ ] **Step 1: Create `src/components/ListenSection.tsx`**

```tsx
/**
 * The card's listen section — the Phase-2 waveform stub, unchanged
 * visually from the retired `?view=listen` panel: a dead waveform
 * animating would lie about being live, so it renders muted and still.
 */
import { CHIP, EMPTY } from '../lib/classes';

export const ListenSection = () => {
  return (
    <div className='flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-4 py-6'>
      <span
        className='flex h-4.5 items-center gap-0.75'
        aria-hidden>
        <i className='h-2 w-0.75 rounded-xs bg-muted' />
        <i className='h-3.5 w-0.75 rounded-xs bg-muted' />
        <i className='h-4.5 w-0.75 rounded-xs bg-muted' />
        <i className='h-3 w-0.75 rounded-xs bg-muted' />
        <i className='h-1.75 w-0.75 rounded-xs bg-muted' />
      </span>
      <p className={EMPTY}>Listen arrives in Phase 2</p>
      <span className={CHIP}>deepgram · stt</span>
    </div>
  );
};
```

- [ ] **Step 2: Create `src/components/ChatSection.tsx`**

```tsx
/**
 * The card's chat section (was `?view=ask`): a true multi-turn
 * conversation — the active `ask` session's persisted history renders on
 * mount, then the `ask:*` live protocol drives the in-flight run's tail.
 *
 * `ask:state{loading}` is a RUN boundary, not an append: it fires once
 * per run AND once per failover retry (and `ask_current` resyncs a live
 * run whose user row is already persisted). `applyLoading` folds all
 * three cases: retry of the current pair → drop the dead attempt's
 * partial text; resync over an existing user row → attach the live tail;
 * anything else → append the new pair.
 *
 * Height reporting moved OUT to Bar.tsx — the observer measures the
 * whole card (this section contributes height naturally) and reports
 * total window height via `window_adjust_height`.
 */
import { useEffect, useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { SettingsIcon, XIcon } from '@marvis/ui';
import {
  askClose,
  askCurrent,
  modelGetSelected,
  sessionEndActive,
  sessionGet,
  sessionList,
  windowShowSettings,
  type ModelSelection,
} from '../lib/commands';
import {
  EV_ASK_CHUNK,
  EV_ASK_DONE,
  EV_ASK_ERROR,
  EV_ASK_STATE,
  useTauriEvent,
} from '../lib/events';
import {
  ASK_MD,
  BTN_OUTLINE,
  BTN_SM,
  CHIP,
  EMPTY,
  ICON_BTN,
  PANEL_BODY,
  PANEL_HEAD,
  SPIN,
  cn,
} from '../lib/classes';

type AskPhase = 'loading' | 'streaming' | 'idle';

interface AskStatePayload {
  state: AskPhase;
  /** Present on `loading` — the submitted question (ask.rs). */
  question?: string;
}

interface ChatMsg {
  role: 'user' | 'assistant';
  content: string;
}

/** Distance from the bottom that still counts as pinned for autoscroll. */
const PIN_PX = 24;

/** Fold a `loading` boundary into the message list (see file doc). */
const applyLoading = (prev: ChatMsg[], q: string): ChatMsg[] => {
  const last = prev[prev.length - 1];
  if (
    last?.role === 'assistant' &&
    prev[prev.length - 2]?.role === 'user' &&
    prev[prev.length - 2].content === q
  ) {
    // Failover retry of the current run — drop the dead attempt's text.
    return [...prev.slice(0, -1), { role: 'assistant', content: '' }];
  }
  if (last?.role === 'user' && last.content === q) {
    // Resync: the persisted user row already rendered — attach the tail.
    return [...prev, { role: 'assistant', content: '' }];
  }
  return [
    ...prev,
    { role: 'user', content: q },
    { role: 'assistant', content: '' },
  ];
};

/** Set the tail assistant bubble's content (`done.full` / resync buffer). */
const setTail = (prev: ChatMsg[], content: string): ChatMsg[] => {
  const last = prev[prev.length - 1];
  if (last?.role !== 'assistant') {
    return [...prev, { role: 'assistant', content }];
  }
  return [...prev.slice(0, -1), { role: 'assistant', content }];
};

/** Append a streamed token to the tail assistant bubble. */
const appendTail = (prev: ChatMsg[], text: string): ChatMsg[] => {
  const last = prev[prev.length - 1];
  if (last?.role !== 'assistant') {
    return [...prev, { role: 'assistant', content: text }];
  }
  return [
    ...prev.slice(0, -1),
    { role: 'assistant', content: last.content + text },
  ];
};

export const ChatSection = () => {
  const [msgs, setMsgs] = useState<ChatMsg[]>([]);
  const [phase, setPhase] = useState<AskPhase>('idle');
  const [model, setModel] = useState<ModelSelection | null>(null);
  const [error, setError] = useState<{
    message: string;
    needsSetup: boolean;
  } | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  /** Autoscroll is on until the user scrolls away from the bottom. */
  const pinnedRef = useRef(true);

  // Mount resync: active `ask` session → persisted history; then
  // `ask_current` folds an in-flight run's tail on top. Best-effort —
  // live events drive the card even if the reads fail.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const sessions = await sessionList();
        const active = sessions.find(
          (s) => s.kind === 'ask' && s.ended_at === null,
        );
        if (active) {
          const rows = await sessionGet(active.id);
          if (!cancelled) {
            setMsgs(
              rows
                .filter((r) => r.role === 'user' || r.role === 'assistant')
                .map((r) => ({
                  role: r.role as ChatMsg['role'],
                  content: r.content,
                })),
            );
          }
        }
        const cur = await askCurrent();
        if (!cancelled && cur.state !== 'idle') {
          setMsgs((prev) =>
            setTail(applyLoading(prev, cur.question), cur.response),
          );
          setPhase(cur.state);
        }
      } catch {
        /* history is best-effort */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // The model chip is honest metadata — the active provider+model pair.
  useEffect(() => {
    void modelGetSelected()
      .then(setModel)
      .catch(() => {});
  }, []);

  useTauriEvent<AskStatePayload>(EV_ASK_STATE, (p) => {
    if (p.state === 'loading') {
      setError(null);
      pinnedRef.current = true;
      setMsgs((prev) => applyLoading(prev, p.question ?? ''));
    }
    setPhase(p.state);
  });
  useTauriEvent<{ text: string }>(EV_ASK_CHUNK, (p) => {
    setMsgs((prev) => appendTail(prev, p.text));
  });
  useTauriEvent<{ full: string; provider?: string; model?: string }>(
    EV_ASK_DONE,
    (p) => {
      // `full` is authoritative — covers a dropped/duplicated chunk;
      // `provider`/`model` report who ACTUALLY answered under failover.
      setMsgs((prev) => setTail(prev, p.full));
      if (p.provider && p.model) {
        setModel({ provider: p.provider, model: p.model });
      }
    },
  );
  useTauriEvent<{ message: string; needs_setup?: boolean }>(
    EV_ASK_ERROR,
    (p) => {
      setError({ message: p.message, needsSetup: p.needs_setup === true });
    },
  );

  // Repaint-per-token: stay glued to the bottom while pinned.
  useEffect(() => {
    const el = scrollRef.current;
    if (el && pinnedRef.current) {
      el.scrollTop = el.scrollHeight;
    }
  }, [msgs, phase]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (el) {
      pinnedRef.current =
        el.scrollHeight - el.scrollTop - el.clientHeight <= PIN_PX;
    }
  };

  const newChat = () => {
    void sessionEndActive('ask').catch(() => {});
    setMsgs([]);
    setError(null);
    setPhase('idle');
    pinnedRef.current = true;
  };

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      <header className={PANEL_HEAD}>
        <p className='min-w-0 flex-1 text-xs leading-normal font-[550] select-text'>
          Chat
        </p>
        <button
          type='button'
          className={cn(BTN_SM, BTN_OUTLINE)}
          onClick={newChat}>
          New chat
        </button>
        <button
          type='button'
          className={cn(ICON_BTN, '-mt-0.5 shrink-0')}
          title='Settings'
          aria-label='Settings'
          onClick={() => void windowShowSettings().catch(() => {})}>
          <SettingsIcon className='size-4' />
        </button>
        <button
          type='button'
          className={cn(ICON_BTN, '-mt-0.5 shrink-0')}
          title='Close'
          aria-label='Close'
          onClick={() => void askClose().catch(() => {})}>
          <XIcon className='size-4' />
        </button>
      </header>
      {error && (
        <div className='flex items-center gap-2 border-b border-border bg-[color-mix(in_oklch,var(--destructive)_9%,transparent)] px-3 py-2 text-xs text-destructive'>
          <span className='min-w-0 flex-1 wrap-break-word'>
            {error.message}
          </span>
          {error.needsSetup && (
            <button
              type='button'
              className={cn(BTN_SM, BTN_OUTLINE)}
              onClick={() => void windowShowSettings().catch(() => {})}>
              Open settings
            </button>
          )}
        </div>
      )}
      <div
        ref={scrollRef}
        onScroll={onScroll}
        className={PANEL_BODY}>
        {msgs.map((m, i) =>
          m.role === 'user' ? (
            <div
              key={i}
              className='mb-2 flex justify-end'>
              <p className='max-w-[85%] rounded-2xl rounded-br-sm bg-fg-soft px-3 py-1.5 text-[13px] leading-[1.5] wrap-break-word whitespace-pre-wrap select-text'>
                {m.content}
              </p>
            </div>
          ) : (
            m.content && (
              <div
                key={i}
                className={cn(ASK_MD, 'mb-2.5')}>
                <ReactMarkdown
                  remarkPlugins={[remarkGfm]}
                  disallowedElements={['img']}
                  components={{
                    a: ({ href, children }) => (
                      <a
                        href={href}
                        onClick={(e) => {
                          e.preventDefault();
                          if (href) void openUrl(href);
                        }}>
                        {children}
                      </a>
                    ),
                  }}>
                  {m.content}
                </ReactMarkdown>
              </div>
            )
          ),
        )}
        {phase === 'loading' && (
          <div className='flex items-center gap-2 py-0.5 text-xs text-muted-foreground'>
            <span className={SPIN} />
            Thinking…
          </div>
        )}
        {phase === 'streaming' && (
          <span className='ml-0.5 inline-block h-3.25 w-1.75 animate-caret bg-foreground align-[-2px] motion-reduce:animate-none' />
        )}
        {msgs.length === 0 && phase === 'idle' && !error && (
          <p className={EMPTY}>Ask Marvis — the conversation stays here.</p>
        )}
        {phase === 'idle' &&
          model &&
          msgs.some((m) => m.role === 'assistant' && m.content) && (
            <div className='mt-2.5 flex flex-wrap gap-1.5'>
              <span className={CHIP}>
                {model.model} · {model.provider}
              </span>
            </div>
          )}
      </div>
    </div>
  );
};
```

- [ ] **Step 3: Verify compile**

Run: `cd apps/native && bun run build`
Expected: PASS — both components compile standalone (unused exports are fine).

- [ ] **Step 4: Commit**

```bash
git add apps/native/src/components/ChatSection.tsx apps/native/src/components/ListenSection.tsx
git commit -m "feat(native): chat + listen sections for the unified card"
```

---

### Task 8: `Bar.tsx` becomes the shell + route/view cleanup

The bar window renders the pill modes (unchanged internals) OR the expanded card — `BarRow` pinned to the anchored edge plus `ChatSection`/`ListenSection`. Card state is read off the window itself: `innerHeight > BAR_H` = open; grow direction from the y-delta at expand time.

**Files:**

- Modify: `apps/native/src/views/Bar.tsx` (rewrite)
- Modify: `apps/native/src/App.tsx`
- Delete: `apps/native/src/views/AskPanel.tsx`, `apps/native/src/views/ListenPanel.tsx`

**Interfaces:**

- Consumes: `ChatSection`, `ListenSection` (Task 7); `windowSetChatOpen`, `windowAdjustHeight`, `askClose`, `askCurrent`-fed events (Tasks 6/4); `getCurrentWindow` from `@tauri-apps/api/window` (already a dependency via `@tauri-apps/api ^2`).
- Produces: nothing new consumed elsewhere — this is the leaf.

- [ ] **Step 1: Rewrite `src/views/Bar.tsx`**

```tsx
/**
 * The always-on-top bar (`?view=bar`) — the UNIFIED window. Two shapes:
 *
 *  - Pill modes (mini | input | permission): the 136⇄480×64 capsule⇄
 *    input morph — the capsule IS the window under liquid glass, so
 *    `expanded` reports to `window_set_bar_expanded` and Rust animates
 *    the width change. Unchanged mechanics.
 *  - Card modes (chat | listen): the same window grown to
 *    600×(64+content) — the bar row becomes the card's header, pinned
 *    to the anchored edge (`flex-col-reverse` when growing up puts the
 *    row at the bottom and it never visually jumps).
 *
 * `cardOpen` is read off the window itself: `innerHeight > BAR_H` means
 * Rust expanded us — `set_chat_open` emits nothing by design, so the
 * `resize` event is the single open/close signal for every path
 * (Cmd+/, ask send, `ask_close`, `window_set_chat_open`).
 *
 * `growDir` is detected at expand time: the anchored edge is fixed for
 * grow-down and rises for grow-up, so the first expanded y-read compared
 * to the last collapsed y gives the direction. While collapsed the
 * baseline refreshes on every `tauri://move`/`resize` tick.
 *
 * `data-tauri-drag-region` lives on the bar ROW only in card mode — a
 * stage-level region would intercept text selection in the scrollable
 * conversation. In pill mode it stays on the capsule chrome as before.
 *
 * Errors go to the `alert` window (`alertShow`) — the pill has no room.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { SubmitEvent } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  ArrowLeftIcon,
  CameraIcon,
  GripVerticalIcon,
  MicIcon,
  SettingsIcon,
  ShieldAlertIcon,
  ShineBorder,
  cn,
} from '@marvis/ui';
import {
  alertShow,
  askClose,
  askSend,
  askSendScreenOnly,
  permissionsOpenPrefs,
  permissionsRequestScreen,
  permissionsStatus,
  windowSetBarExpanded,
  windowSetChatOpen,
  windowShowSettings,
  type AppStatePayload,
  type Gate,
} from '../lib/commands';
import {
  EV_ASK_STATE,
  EV_APP_STATE,
  EV_CAPTURE_PERMISSION_NEEDED,
  useTauriEvent,
} from '../lib/events';
import { ChatSection } from '../components/ChatSection';
import { Iris } from '../components/Iris';
import { ListenSection } from '../components/ListenSection';
import { RetryCard } from '../components/RetryCard';
import {
  BTN_LINK,
  BTN_LINK_SM,
  BTN_PRIMARY,
  BTN_SM,
  ICON_BTN,
  PANEL,
} from '../lib/classes';

/** Mirrors `app_gate` in lib.rs so the first render doesn't wait on `app:state`. */
const gateFor = (screen: boolean): Gate =>
  screen ? 'main' : 'needs_permission';

/** Every bar error goes to the alert window — the pill has no room. */
const raise = (message: string) => void alertShow(message).catch(() => {});

/** The bar row's height — the pill's window height in pill modes and
 *  the card header's height in card modes (spec: 64). */
const BAR_H = 64;
/** Card-open read: any window taller than the pill is a card. */
const OPEN_EPS = 2;
/** Card's max-height so the reported height never exceeds Rust's 900
 *  cap — frost keeps the stage's `p-1` (8 px of chrome). */
const CARD_MAX = 900 - 8;
/** Reported-height deadband + invoke throttle (was AskPanel's). */
const HEIGHT_EPS = 4;
const HEIGHT_MS = 150;

/** Bar controls step up from the 26px overlay default (`ICON_BTN`) —
 *  the capsule is 64px tall, so buttons/icons scale ~1.3×; `fg-2` reads
 *  better than `muted` on glass. */
const BAR_BTN = cn(ICON_BTN, 'size-8.5 text-fg-2');

const grip = (
  <span
    className='-mx-0.75 grid w-4 max-w-0 flex-none cursor-grab place-items-center overflow-hidden text-muted-foreground opacity-0 transition-[max-width_var(--motion-base)_var(--ease),opacity_var(--motion-fast)_var(--ease),margin-inline_var(--motion-base)_var(--ease)] group-hover/bar:mx-0 group-hover/bar:max-w-4 group-hover/bar:opacity-100 active:cursor-grabbing motion-reduce:transition-none'
    data-tauri-drag-region='deep'
    title='Drag'
    aria-hidden='true'>
    <GripVerticalIcon className='size-3.75' />
  </span>
);

const Bar = () => {
  const [gate, setGate] = useState<Gate | null>(null);
  const [bootError, setBootError] = useState(false);
  const [busy, setBusy] = useState(false);
  const [text, setText] = useState('');
  const [open, setOpen] = useState(false);
  const [cardOpen, setCardOpen] = useState(
    () => window.innerHeight > BAR_H + OPEN_EPS,
  );
  const [growDir, setGrowDir] = useState<'up' | 'down'>('down');
  const [listenWanted, setListenWanted] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const cardRef = useRef<HTMLDivElement>(null);
  /** Last collapsed-mode outer y — the baseline the expand direction
   *  is detected against. */
  const collapsedY = useRef<number | null>(null);

  // Icon-row ⇄ input-row swap: the gate card and boot errors count as
  // expanded; `main` rests as the icon row until the iris opens it or
  // the user starts typing on the focused window.
  const expanded = bootError
    ? true
    : gate === 'main'
      ? open || text.length > 0
      : gate !== null;
  /** The row shows the input row in card modes regardless of `open` —
   *  it's the card header and the follow-up field. */
  const showInputRow = expanded || cardOpen;
  const section: 'chat' | 'listen' | null = !cardOpen
    ? null
    : listenWanted
      ? 'listen'
      : 'chat';

  const bootstrap = useCallback(async () => {
    try {
      const perms = await permissionsStatus();
      setGate(gateFor(perms.screen));
      setBootError(false);
    } catch {
      setBootError(true);
    }
  }, []);

  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);

  useTauriEvent<AppStatePayload>(EV_APP_STATE, (p) => setGate(p.gate));
  // Mid-session screen-permission revocation (ask.rs detects it when a
  // stale frame would have shipped): collapse the card — NOT `askClose`,
  // which would cancel the text-only fallback — and show the
  // permission card.
  useTauriEvent<{ permission: string }>(EV_CAPTURE_PERMISSION_NEEDED, () => {
    void windowSetChatOpen(false).catch(() => {});
    setGate('needs_permission');
  });
  // A send while in listen mode reasserts chat (`loading` = a run).
  useTauriEvent<{ state: string }>(EV_ASK_STATE, (p) => {
    if (p.state === 'loading') {
      setListenWanted(false);
    }
  });

  // Card open/close is learned from the window itself; grow direction
  // from the y-delta at expand time. While collapsed the baseline y
  // refreshes on move + resize ticks (a drag moves without resizing).
  useEffect(() => {
    const win = getCurrentWindow();
    let alive = true;
    const unMove = win.onMoved((e) => {
      if (window.innerHeight <= BAR_H + OPEN_EPS) {
        collapsedY.current = e.payload.y;
      }
    });
    const read = () => {
      const openNow = window.innerHeight > BAR_H + OPEN_EPS;
      void win
        .outerPosition()
        .then((p) => {
          if (!alive) {
            return;
          }
          setCardOpen((was) => {
            if (openNow && !was && collapsedY.current !== null) {
              setGrowDir(p.y < collapsedY.current - 0.5 ? 'up' : 'down');
            }
            return openNow;
          });
          if (!openNow) {
            collapsedY.current = p.y;
          }
        })
        .catch(() => setCardOpen(openNow));
    };
    window.addEventListener('resize', read);
    read();
    return () => {
      alive = false;
      window.removeEventListener('resize', read);
      void unMove.then((u) => u());
    };
  }, []);

  // Collapsing resets the section pick — the next open is chat.
  useEffect(() => {
    if (!cardOpen) {
      setListenWanted(false);
    }
  }, [cardOpen]);

  // The capsule IS the window under liquid glass — the pill⇄input morph
  // resizes it (idle 136 ⇄ 480). While the card is open the morph is
  // dormant: the width report is skipped so `bar_rect` (the canonical
  // pill) restores verbatim on collapse.
  useEffect(() => {
    if (!cardOpen) {
      void windowSetBarExpanded(expanded).catch(() => {});
    }
  }, [expanded, cardOpen]);

  // Report the card's desired TOTAL window height: leading + trailing
  // throttle, only on a real (>EPS) change — `adjust_height` animates
  // per call. The observer watches the CARD element (content-sized,
  // capped at CARD_MAX) — NOT the window-fixed `h-full` stage, whose
  // box only changes on real window resizes, so streaming content
  // growth/shrink actually triggers reports. `scrollHeight` reads the
  // uncapped content height (overflow counts); the backend clamps to
  // min(900, free). The stage's vertical padding is added back (frost
  // keeps `p-1`, glass strips it) so the report is total window height
  // under both materials.
  useEffect(() => {
    const el = cardRef.current;
    const stage = stageRef.current;
    if (!el || !cardOpen) {
      return;
    }
    let lastValue = -1;
    let lastSentAt = 0;
    let timer: number | undefined;
    const report = () => {
      const cs = stage ? getComputedStyle(stage) : null;
      const padY = cs
        ? parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom)
        : 0;
      const h = Math.min(
        Math.ceil(el.scrollHeight + (Number.isFinite(padY) ? padY : 0)),
        900,
      );
      if (Math.abs(h - lastValue) <= HEIGHT_EPS) {
        return;
      }
      const wait = HEIGHT_MS - (Date.now() - lastSentAt);
      if (wait <= 0) {
        lastValue = h;
        lastSentAt = Date.now();
        void windowAdjustHeight(h).catch(() => {});
      } else if (timer === undefined) {
        timer = window.setTimeout(() => {
          timer = undefined;
          report();
        }, wait);
      }
    };
    const observer = new ResizeObserver(report);
    observer.observe(el);
    report();
    return () => {
      observer.disconnect();
      window.clearTimeout(timer);
    };
  }, [cardOpen]);

  // Focus the field whenever the input row shows.
  useEffect(() => {
    if (showInputRow && gate === 'main') {
      inputRef.current?.focus();
    }
  }, [showInputRow, gate]);

  // Type-to-wake on the collapsed pill; Esc collapses input → capsule,
  // and collapses the card via `ask_close` (cancel + `set_chat_open`).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (cardOpen) {
          void askClose().catch(() => {});
          return;
        }
        setText('');
        setOpen(false);
        inputRef.current?.blur();
        return;
      }
      if (gate !== 'main' || open || cardOpen || e.metaKey || e.ctrlKey || e.altKey) {
        return;
      }
      if (e.key.length === 1) {
        setText(e.key);
        setOpen(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [gate, open, cardOpen]);

  const collapse = () => {
    setOpen(false);
    inputRef.current?.blur();
  };

  const grantScreen = async () => {
    if (busy) {
      return;
    }
    setBusy(true);
    try {
      await permissionsRequestScreen();
      await bootstrap();
    } catch {
      raise('Permission request failed');
    } finally {
      setBusy(false);
    }
  };

  // Submit = ask (a follow-up while the card is open). The backend
  // expands the window itself — no local collapse needed either way.
  const submitAsk = (e: SubmitEvent<HTMLFormElement>) => {
    e.preventDefault();
    const t = text.trim();
    if (!t) {
      return;
    }
    setText('');
    void askSend(t).catch(() => raise('Send failed'));
  };

  const rowCls = cn(
    'flex min-h-0 w-full flex-none items-center gap-1.5',
    cardOpen
      ? cn(
          'h-16 border-border px-2.75',
          // The divider sits between the row and the section — which
          // side depends on the grow direction (the row is bottom-
          // pinned under `flex-col-reverse` when growing up).
          growDir === 'up' ? 'border-t' : 'border-b',
        )
      : showInputRow
        ? 'px-2.75'
        : 'justify-center px-1.75',
  );

  const row = () => {
    if (bootError) {
      return (
        <div
          className={cn(rowCls, 'justify-center')}
          data-tauri-drag-region>
          {grip}
          <RetryCard onRetry={() => void bootstrap()} />
        </div>
      );
    }
    if (gate === 'needs_permission') {
      return (
        <div
          className={rowCls}
          data-tauri-drag-region>
          {grip}
          <ShieldAlertIcon
            className='size-4.5 flex-none text-muted-foreground'
            data-tauri-drag-region
          />
          <span
            className='min-w-0 flex-1 truncate text-xs text-muted-foreground'
            title='Marvis needs screen recording to see your screen'
            data-tauri-drag-region>
            Screen recording needed
          </span>
          <button
            type='button'
            className={cn(BTN_SM, BTN_PRIMARY)}
            onClick={() => void grantScreen()}
            disabled={busy}>
            Grant
          </button>
          <button
            type='button'
            className={cn(BTN_LINK_SM, BTN_LINK)}
            onClick={() => void permissionsOpenPrefs('Privacy_ScreenCapture')}>
            Open settings
          </button>
        </div>
      );
    }
    // `main` (and the null boot frame — the same capsule, inert until
    // the gate resolves).
    return (
      <form
        onSubmit={submitAsk}
        className={rowCls}
        data-tauri-drag-region>
        {grip}
        <button
          type='button'
          className={cn(BAR_BTN, 'relative')}
          aria-label={
            cardOpen ? 'Close chat' : open ? 'Back to capsule' : 'Ask Marvis'
          }
          onClick={() =>
            cardOpen
              ? void askClose().catch(() => {})
              : open
                ? collapse()
                : setOpen(true)
          }
          disabled={gate !== 'main'}>
          <Iris />
          <span className='pointer-events-none absolute inset-0 grid -rotate-90 scale-[0.4] place-items-center opacity-0 transition-[rotate_var(--motion-base)_var(--ease)_55ms,scale_var(--motion-base)_var(--ease)_55ms,opacity_var(--motion-fast)_var(--ease)_55ms] group-data-expanded/bar:rotate-none group-data-expanded/bar:scale-100 group-data-expanded/bar:opacity-100 motion-reduce:transition-none'>
            <ArrowLeftIcon className='size-5.5' />
          </span>
        </button>
        <input
          ref={inputRef}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onFocus={() => gate === 'main' && setOpen(true)}
          placeholder='Ask Marvis…'
          aria-label='Ask Marvis'
          className={cn(
            'min-w-0 flex-1 self-stretch border-0 bg-transparent text-[13.5px] text-foreground caret-accent outline-none select-text placeholder:text-muted-foreground focus-visible:shadow-none transition-[max-width_var(--motion-base)_var(--ease),opacity_var(--motion-fast)_var(--ease),margin-inline_var(--motion-base)_var(--ease)] motion-reduce:transition-none',
            showInputRow
              ? 'max-w-80'
              : 'pointer-events-none -mx-1.5 max-w-0 opacity-0',
          )}
        />
        <button
          type='button'
          className={BAR_BTN}
          aria-label='Ask about the screen'
          title='Ask about the screen'
          disabled={gate !== 'main'}
          onClick={() =>
            void askSendScreenOnly().catch(() => raise('Send failed'))
          }>
          <CameraIcon className='size-5' />
        </button>
        <button
          type='button'
          className={BAR_BTN}
          aria-label='Listen'
          title='Listen'
          disabled={gate !== 'main'}
          onClick={() => {
            setListenWanted(true);
            void windowSetChatOpen(true).catch(() => {});
          }}>
          <MicIcon className='size-5' />
        </button>
        {/* Only rendered in the input row — the idle capsule has no
            room for a fourth control (tray menu + Cmd+, reach it
            anyway). */}
        {showInputRow && (
          <button
            type='button'
            className={BAR_BTN}
            aria-label='Settings'
            title='Settings (⌘,)'
            disabled={gate !== 'main'}
            onClick={() => void windowShowSettings().catch(() => {})}>
            <SettingsIcon className='size-5' />
          </button>
        )}
      </form>
    );
  };

  return (
    <div
      ref={stageRef}
      className={cn(
        'group/stage glass-stage flex h-full flex-col p-1',
        growDir === 'up' ? 'justify-end' : 'justify-start',
      )}
      data-pos={growDir === 'up' ? 'bottom' : 'top'}
      data-dir={growDir}>
      <div
        ref={cardRef}
        className={cn(
          'group/bar glass-surface relative flex w-full flex-none select-none',
          cardOpen
            ? cn(PANEL, growDir === 'up' ? 'flex-col-reverse' : 'flex-col')
            : 'h-full flex-col justify-center rounded-full bg-[color-mix(in_oklch,var(--surface)_80%,transparent)] backdrop-blur-[14px] transition-[border-color,box-shadow] duration-(--motion-base) ease-(--ease) motion-reduce:transition-none',
        )}
        style={cardOpen ? { maxHeight: CARD_MAX } : undefined}
        data-expanded={showInputRow || undefined}
        data-tauri-drag-region={cardOpen ? undefined : 'deep'}>
        {row()}
        {section === 'chat' && <ChatSection />}
        {section === 'listen' && <ListenSection />}
        {/* Capsule shimmer — accent duotone follows light/dark via the
            tokens; masked to the border ring, pointer-events-none. */}
        <ShineBorder
          shineColor={['#A07CFE', '#FE8FB5', '#FFBE7B', 'var(--accent)']}
          borderWidth={1.8}
        />
      </div>
    </div>
  );
};

export default Bar;
```

Notes for the implementer (intentional behaviors — don't "fix" them):

- `submitAsk` no longer calls `collapse()` — the backend's `set_chat_open(true)` expands the window; a local 480→136→600 morph fight would be visible.
- The pill morph report is gated on `!cardOpen` so `bar_rect` stays the untouched pre-expand pill through the whole chat.
- `ShineBorder` is kept on the card too (it follows the border ring); flag for design review if it reads heavy on a tall card.
- One-frame `growDir` guess on the first expand tick is possible (`outerPosition` is async) — harmless mid-animation; don't add complexity to eliminate it.

- [ ] **Step 2: `App.tsx` — shrink routes**

```tsx
import Bar from './views/Bar';
import AlertToast from './views/AlertToast';
import Prefs from './views/Prefs';

/**
 * One webview bundle serves every window. `windows/mod.rs` builds each
 * `WebviewWindow` with `index.html?view=<label>` (`"bar"`, `"alert"`,
 * `"prefs"`) — the query picks the view; anything missing/unknown falls
 * back to Bar, the always-present unified window (chat/listen are its
 * card modes now, not separate windows).
 */
export default function App() {
  const view = new URLSearchParams(window.location.search).get('view');
  switch (view) {
    case 'alert':
      return <AlertToast />;
    case 'prefs':
      return <Prefs />;
    default:
      return <Bar />;
  }
}
```

- [ ] **Step 3: Delete the panel views**

```bash
rm apps/native/src/views/AskPanel.tsx apps/native/src/views/ListenPanel.tsx
```

- [ ] **Step 4: Verify build**

Run: `cd apps/native && bun run build && bun run test`
Expected: PASS — `tsc` clean (no unresolved `AskPanel`/`ListenPanel` imports, no unused `windowAdjustHeight` callers), vite build, `bun test --pass-with-no-tests` exits 0.

- [ ] **Step 5: Commit**

```bash
git add -A apps/native/src
git commit -m "feat(native): Bar becomes the unified shell — card modes replace panel windows"
```

---

### Task 9: Verification sweep

**Files:** none (verification only — fix-forward if something fails).

- [ ] **Step 1: Full Rust suite + lints**

Run: `cd apps/native/src-tauri && cargo test && cargo clippy`
Expected: all tests pass (layout expansion, ask chain incl. multi-turn, storage, lib, movement, hotkey); no clippy warnings.

- [ ] **Step 2: Frontend build + tests**

Run: `cd apps/native && bun run build && bun run test`
Expected: `tsc` clean, vite build succeeds, `bun test --pass-with-no-tests` exits 0.

(The spec lists `bun run check-types` — `apps/native` defines no such script; `bun run build` runs `tsc` first, which IS the typecheck. `check-types` only exists as a turbo task for packages that define it, e.g. `apps/web`.)

- [ ] **Step 3: Manual checklist (run `bun run build:dev` / `bun run tauri dev`)**

Verify each item and note results:

- [ ] Capsule ⇄ input morph unchanged (136⇄480, window resizes).
- [ ] Ask from the bar → window expands 480→600 downward (top-docked), bar row pinned at top, reply streams into the card.
- [ ] Follow-up question from the card's bar row → appends to the same conversation (multi-turn context).
- [ ] `Cmd+/` collapses → pill restores at the same anchor/width; `Cmd+/` again → card reopens with history intact (session resync).
- [ ] Bottom-docked bar (Settings → Bar → bottom): expand grows UP, bar row pinned at the window's bottom edge, no visual jump.
- [ ] "New chat" clears the conversation and starts a fresh session (next ask gets no prior context).
- [ ] Mic button → card opens in listen mode showing the Phase-2 stub; sending an ask returns it to chat mode.
- [ ] Esc in chat → collapse; Esc again on the pill → back to capsule.
- [ ] Alert toast anchors under the expanded card (trigger via e.g. a provider error).
- [ ] Permission-revocation path: revoke screen recording mid-session, ask → `capture:permission-needed` → card collapses + permission card shows (ask continues text-only in background).
- [ ] `apps/web` `.mv-*` mock views untouched (out of scope).

- [ ] **Step 4: Commit any fixes; otherwise done**

```bash
git status   # clean tree expected
```

---

## Self-review results

- **Spec coverage:** all `windows/mod.rs` deletions/additions (Task 2), `lib.rs` command changes incl. `session_end_active`/`ask_current` (Tasks 2–4), `ask.rs` emit target + `set_chat_open` calls (Task 2), multi-turn `send_chain` (Task 5), frontend `App.tsx`/`Bar.tsx`/`ChatSection`/`ListenSection`/deletions (Tasks 7–8), testing section (Tasks 1/3/4/5 + Task 9 sweep) — covered. Expansion math incl. clamps/anchoring: Task 1 + wired in Task 2. Alert anchoring: Task 2 (`show_alert` → `target_rect`). `resizable(false)`: untouched.
- **Deviations recorded** in the interpretation table: 136⇄480 morph kept (user-confirmed), `window_set_chat_open` added, height observer in `Bar.tsx`, glass radius swap, `session_active_id`, window-height as the open signal.
- **Type consistency:** `expanded_rect(bar, dir, chat_h, work)` signature identical across Tasks 1/2; `set_chat_open(app, open)` used by `toggle_chat`, `ask.rs`, `leave_main`, `window_set_chat_open` consistently; `close(&self, app, pool)` matches both call sites (`leave_main` passes `app: &AppHandle`, `ask_close` passes `&app`); `windowAdjustHeight(height)` matches the Rust `(height: f64)`; `session_end_active(kind)` ↔ `Result<bool, String>` ↔ `invoke<boolean>`; `ask_current` payload keys match `AskCurrent`.
