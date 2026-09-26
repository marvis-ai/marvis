//! Window pool, layout math, and the movement animator.
//!
//! ALL geometry in this module is LOGICAL pixels (`f64`). `Monitor::work_area()`
//! returns PHYSICAL pixels and is converted with `.to_logical(scale_factor)`
//! at the boundary — never mix the two (the prior Retina bug).

pub mod layout;
pub mod movement;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Monitor, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use tauri_plugin_liquid_glass::{GlassMaterialVariant, LiquidGlassConfig, LiquidGlassExt};

use layout::{clamp_to_work_area, derive_pill_rect, expand_dir_for, expanded_rect};

/// A rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
    pub fn center_x(&self) -> f64 {
        self.x + self.w / 2.0
    }
    pub fn center_y(&self) -> f64 {
        self.y + self.h / 2.0
    }
    pub fn contains_point(&self, px: f64, py: f64) -> bool {
        px >= self.x && px <= self.right() && py >= self.y && py <= self.bottom()
    }
    #[allow(dead_code)] // reserved geometry helper (overlap checks)
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
}

/// Cardinal direction — the bar's edge-snap target (Settings → Bar
/// picker) and the persisted "nearest edge" read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The bar window's two widths: the capsule IS the window under liquid
/// glass, so idle rests at BAR_IDLE_W (the four capsule controls) and
/// any expanded content — input row, gate cards — uses BAR_W. Height
/// never changes.
const BAR_IDLE_W: f64 = 172.0;
const BAR_W: f64 = 600.0;
const BAR_H: f64 = 64.0;
/// Bar window label — the ask event target now that the chat card lives
/// inside the bar window (`emit_to(BAR_LABEL, "ask:*", …)`).
pub const BAR_LABEL: &str = "bar";
/// Card content height until the webview's first `window_adjust_height`
/// report (spec default).
const CHAT_DEFAULT_H: f64 = 480.0;
/// The expanded card's corner radius — matches the CSS card's
/// `rounded-[18px]`; the glass shape follows it while a card is up.
const CARD_RADIUS: f64 = 18.0;
/// The preferences window — a decorated macOS window backed by the
/// AbuttedSidebar liquid-glass material (transparent titlebar, traffic
/// lights overlaying the sidebar column), not an overlay panel. It
/// floats only while
/// focused — the bar keeps its global always-on-top level. Label
/// `prefs`, `?view=prefs`; it hosts both the settings sidebar and
/// the onboarding wizard, switched by `prefs:mode` emits.
pub const PREFS_LABEL: &str = "prefs";
const PREFS_W: f64 = 720.0;
const PREFS_H: f64 = 520.0;
/// Transient alert toast — its own window because the bar is a fixed
/// 600×64 pill with no room for an error row (the old inline row
/// squeezed the pill's content). Label `alert`, `?view=alert`.
pub const ALERT_LABEL: &str = "alert";
const ALERT_W: f64 = 340.0;
/// Fixed toast height — informational only, so one layout suffices.
const ALERT_H: f64 = 100.0;
/// Slide-in distance above the target rect when the alert toast appears.
const SHOW_OFFSET_Y: f64 = 10.0;
/// Fallback work area if every monitor query fails.
const DEFAULT_WORK: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 1920.0,
    h: 1080.0,
};
/// Card-morph/snap animation duration (spec: ~200 ms; ~180 ms reads well).
const ANIM_DUR: std::time::Duration = std::time::Duration::from_millis(180);

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

/// Debounce counter for the bar's `Moved` event — a drag fires one event
/// per frame, so the persist write waits for the LAST position (400 ms
/// quiet) instead of rewriting `config.toml` on every pixel.
static BAR_MOVE_GEN: AtomicU64 = AtomicU64::new(0);

impl WindowPool {
    /// Build ONLY the bar window — it now hosts the chat/listen card
    /// modes itself, so no feature windows exist to build later.
    /// `show_bar` is the same `onboarding_done` flag — first-run
    /// installs keep the bar hidden until the wizard finishes;
    /// `set_bar_shown` owns it after.
    pub fn create_bar_only(app: &AppHandle, show_bar: bool, accent: &str) -> anyhow::Result<Self> {
        let mut pool = Self {
            bar: None,
            alert: None,
            prefs: None,
            prefs_mode: String::new(),
            chat_open: false,
            chat_height: CHAT_DEFAULT_H,
            expand_dir: Dir::Down,
            bar_rect: DEFAULT_WORK,
        };
        let bar = build_window(
            app,
            BAR_LABEL,
            BAR_IDLE_W,
            BAR_H,
            BAR_H / 2.0,
            accent_glass_tint(accent),
        )?;
        {
            // Persist the resting place on every move — user drags via
            // `data-tauri-drag-region`, edge snaps, and reclamps alike.
            // `Moved` fires per frame during a drag, so the write is
            // debounced: the last generation wins after 400 ms of quiet.
            //
            // `Resized` is different: macOS 26 edge-drags resize
            // borderless windows even with `resizable(false)`, so any
            // stray size snaps back to the canonical bounds — fully for
            // the fixed-size pill, x/width only for the open card (its
            // dragged height is adopted via `window_adjust_height`).
            let app_moved = app.clone();
            let app_resized = app.clone();
            bar.on_window_event(move |event| {
                match event {
                    tauri::WindowEvent::Moved(_) => {
                        let gen = BAR_MOVE_GEN.fetch_add(1, Ordering::Relaxed) + 1;
                        let app = app_moved.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(400));
                            if BAR_MOVE_GEN.load(Ordering::Relaxed) == gen {
                                crate::persist_bar_position(&app);
                            }
                        });
                    }
                    tauri::WindowEvent::Resized(_) => {
                        // Our animator emits `Resized` per tick — only
                        // enforce while no animation is in flight.
                        if movement::is_animating(BAR_LABEL) {
                            return;
                        }
                        // This handler registers before `app.manage(AppState)`
                        // (setup order) — a `Resized` delivered inside
                        // `position_bar_at_startup` must not panic on the
                        // missing state.
                        let Some(state) = app_resized.try_state::<crate::AppState>() else {
                            return;
                        };
                        state.pool.lock().enforce_bar_bounds();
                    }
                    _ => {}
                }
            });
        }
        pool.bar = Some(bar);
        // Built (hidden) up front so its webview is loaded and listening
        // before the first alert — an emit to a still-loading window
        // would be dropped.
        pool.alert = Some(build_window(
            app,
            ALERT_LABEL,
            ALERT_W,
            ALERT_H,
            14.0,
            None,
        )?);
        pool.position_bar_at_startup();
        if show_bar {
            if let Some(bar) = &pool.bar {
                let _ = bar.show();
            }
        }
        Ok(pool)
    }

    /// The bar's visibility rule: it floats only when onboarding is done
    /// AND the prefs window isn't showing the wizard — a re-run hides the
    /// bar again until the window closes. `AppState::sync_bar_visibility`
    /// calls this after every prefs show/hide and every
    /// `app.onboarding_done` write.
    pub fn set_bar_shown(&self, onboarding_done: bool) {
        let onboarding_up = self.prefs_mode == "onboarding"
            && self
                .prefs
                .as_ref()
                .is_some_and(|w| w.is_visible().unwrap_or(false));
        if let Some(bar) = &self.bar {
            if onboarding_done && !onboarding_up {
                let _ = bar.show();
            } else {
                let _ = bar.hide();
            }
        }
    }

    /// Windowless pool — the `AppState::for_test` seam: state needs a pool
    /// value even when no runtime (and thus no windows) exists.
    #[cfg(test)]
    pub fn new_empty() -> Self {
        Self {
            bar: None,
            alert: None,
            prefs: None,
            prefs_mode: String::new(),
            chat_open: false,
            chat_height: CHAT_DEFAULT_H,
            expand_dir: Dir::Down,
            bar_rect: DEFAULT_WORK,
        }
    }

    pub fn bar(&self) -> Option<&WebviewWindow> {
        self.bar.as_ref()
    }

    /// Slide the alert toast in, centered under the bar's current
    /// target — the expanded card when open, the pill otherwise (for
    /// grow-up they coincide). Deliberately overlaps the card and is
    /// shown last so it lands on top; it is never focused, so a bar
    /// field keeps its focus.
    pub fn show_alert(&mut self) {
        let Some(win) = self.alert.clone() else {
            log::warn!("windows::show_alert: alert window not created");
            return;
        };
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
        set_rect(
            &win,
            Rect {
                y: target.y - SHOW_OFFSET_Y,
                ..target
            },
        );
        let _ = win.show();
        let _ = win.set_always_on_top(true);
        movement::animate(&win, target, ANIM_DUR);
    }

    pub fn hide_alert(&self) {
        if let Some(win) = &self.alert {
            let _ = win.hide();
        }
    }

    /// Show the prefs window in `mode` (`"settings"` or `"onboarding"`).
    /// Built lazily — once it exists the same webview is re-shown and the
    /// mode is pushed via `prefs:mode` (the frontend also re-reads it with
    /// `prefs_mode` on mount, so a show that raced the load still lands).
    /// The window title follows the mode: `Marvis — Settings` /
    /// `Marvis — Set up`. Onboarding mode hides the bar — the wizard
    /// must not share the screen with the floating UI.
    pub fn show_prefs(&mut self, app: &AppHandle, mode: &str) {
        let win = match &self.prefs {
            Some(w) => w.clone(),
            None => match build_prefs_window(app) {
                Ok(w) => {
                    // Center only on first show — after that the user's
                    // placement (normal macOS window behavior) is kept.
                    let _ = w.center();
                    self.prefs = Some(w.clone());
                    w
                }
                Err(e) => {
                    log::warn!("windows: prefs window failed to build: {e}");
                    return;
                }
            },
        };
        self.prefs_mode = mode.to_string();
        let _ = app.emit_to(
            PREFS_LABEL,
            "prefs:mode",
            serde_json::json!({ "mode": mode }),
        );
        let _ = win.set_title(match mode {
            "onboarding" => "Marvis — Set up",
            _ => "Marvis — Settings",
        });
        let _ = win.show();
        let _ = win.set_focus();
        // The bar's visibility rule — applied AFTER the prefs window is
        // visible so `set_bar_shown` sees the wizard up.
        let done = app.state::<crate::AppState>().onboarding_done();
        self.set_bar_shown(done);
    }

    pub fn hide_prefs(&self) {
        if let Some(win) = &self.prefs {
            // Drop the focused-only float before hiding — a hidden
            // window must never hold the always-on-top level.
            let _ = win.set_always_on_top(false);
            let _ = win.hide();
        }
    }

    pub fn prefs_mode(&self) -> &str {
        &self.prefs_mode
    }

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

    /// Whether the card is currently expanded. ask's pre-flight reads
    /// this BEFORE `set_chat_open(true)` to tell a fresh pill send (new
    /// conversation) from a card follow-up.
    pub fn is_chat_open(&self) -> bool {
        self.chat_open
    }

    /// `window_set_chat_open` / tray Toggle / ask
    /// pre-flight: animate the bar
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
        self.sync_bar_size_limits();
    }

    /// Re-apply the bar's glass effect at its CURRENT radius — called
    /// after an `app.accent` config write so the tint follows the new
    /// accent without waiting for the next pill⇄card morph.
    pub fn refresh_bar_glass(&self, app: &AppHandle) {
        let Some(bar) = self.bar.clone() else { return };
        let radius = if self.chat_open {
            CARD_RADIUS
        } else {
            BAR_H / 2.0
        };
        set_glass_radius(app, &bar, radius);
    }

    /// `window_toggle_all`/tray Toggle: open ⇄ close the card. (The
    /// `toggle_input` hotkey is NOT here — it only morphs the bar's
    /// input pill, webview-side.)
    pub fn toggle_chat(&mut self, app: &AppHandle) {
        self.set_chat_open(app, !self.chat_open);
    }

    /// The work-area edge the bar's center is nearest to — what the
    /// Settings → Bar picker shows as the current position (the bar
    /// can rest anywhere after a drag, so "current" is always an edge
    /// read, not a stored field).
    pub fn bar_edge(&self) -> Dir {
        let work = self.bar_work_area();
        let (cx, cy) = (self.bar_rect.center_x(), self.bar_rect.center_y());
        let d_top = (cy - work.y).abs();
        let d_bottom = (work.bottom() - cy).abs();
        let d_left = (cx - work.x).abs();
        let d_right = (work.right() - cx).abs();
        let min = d_top.min(d_bottom).min(d_left).min(d_right);
        if min == d_bottom {
            Dir::Down
        } else if min == d_left {
            Dir::Left
        } else if min == d_right {
            Dir::Right
        } else {
            Dir::Up
        }
    }

    /// Settings → Bar picker: animate the bar to the named work-area edge
    /// (12 px margin); the open card follows as a unit. The move fires
    /// `Moved`, so the new position persists through the debounced write
    /// — no extra save here.
    pub fn snap_edge(&mut self, dir: Dir) {
        self.reclamp();
        let work = self.bar_work_area();
        let (x, y) = layout::snap_edge(self.bar_rect, dir, work);
        self.bar_rect = Rect {
            x,
            y,
            ..self.bar_rect
        };
        if let Some(bar) = &self.bar {
            movement::animate(bar, self.target_rect(), ANIM_DUR);
        }
    }

    /// Settings → Bar "Re-center": restore the startup default — the
    /// middle of the primary work area. Persists via the `Moved`
    /// debounce like any other move.
    pub fn recenter_bar(&mut self) {
        let work = self.primary_work_area();
        self.bar_rect = Rect {
            x: work.center_x() - BAR_IDLE_W / 2.0,
            y: work.center_y() - BAR_H / 2.0,
            w: BAR_IDLE_W,
            h: BAR_H,
        };
        self.sync_bar_size_limits();
        if let Some(bar) = &self.bar {
            movement::animate(bar, self.target_rect(), ANIM_DUR);
        }
    }

    /// `window_set_bar_expanded(bool)` — the webview reports its
    /// pill⇄input morph so the window can match it: under liquid glass
    /// the capsule IS the window, so idle rests at `BAR_IDLE_W` and any
    /// expanded content (input row, gate cards) needs `BAR_W`. Width
    /// changes recenter on center-x — the capsule blooms symmetrically —
    /// then re-clamp inside the work area.
    pub fn set_bar_expanded(&mut self, expanded: bool) {
        self.refresh_bar_rect();
        let w = if expanded { BAR_W } else { BAR_IDLE_W };
        if (self.bar_rect.w - w).abs() < f64::EPSILON {
            return;
        }
        self.bar_rect = clamp_to_work_area(
            Rect {
                x: self.bar_rect.center_x() - w / 2.0,
                w,
                ..self.bar_rect
            },
            self.bar_work_area(),
        );
        if let Some(bar) = &self.bar {
            movement::animate(bar, self.target_rect(), ANIM_DUR);
        }
        self.sync_bar_size_limits();
    }

    /// `window_adjust_height(px)`: `px` is the desired TOTAL window
    /// height (the frontend measures the whole card). Expanded-only —
    /// ignored when the card is closed. `expanded_rect` clamps to
    /// `[BAR_H + 40, free space]` keeping the anchored edge
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

    /// Initial bar position: `config.window.bar_x/y` if BOTH are set, else
    /// the middle of the primary work area. Re-clamps to the primary
    /// monitor when the saved position's center is off-screen.
    pub fn position_bar_at_startup(&mut self) {
        let prefs = crate::config::load().window;
        let rect = match (prefs.bar_x, prefs.bar_y) {
            (Some(x), Some(y)) => Rect {
                x,
                y,
                w: BAR_IDLE_W,
                h: BAR_H,
            },
            _ => {
                let work = self.primary_work_area();
                Rect {
                    x: work.center_x() - BAR_IDLE_W / 2.0,
                    y: work.center_y() - BAR_H / 2.0,
                    w: BAR_IDLE_W,
                    h: BAR_H,
                }
            }
        };
        self.bar_rect = rect;
        if let Some(bar) = &self.bar {
            set_rect(bar, rect);
        }
        self.sync_bar_size_limits();
        self.reclamp();
    }

    /// Display-change recovery (no Tauri display event exists): if the bar's
    /// center is no longer inside ANY monitor's work area, re-clamp it into
    /// the primary monitor's. A periodic watch is a Task-18 manual item.
    fn reclamp(&mut self) {
        self.refresh_bar_rect();
        let Some(bar) = self.bar.clone() else { return };
        let (cx, cy) = (self.bar_rect.center_x(), self.bar_rect.center_y());
        let Ok(monitors) = bar.available_monitors() else {
            return;
        };
        let inside = monitors
            .iter()
            .any(|m| logical_work_area(m).contains_point(cx, cy));
        if inside {
            return;
        }
        let Some(primary) = bar
            .primary_monitor()
            .ok()
            .flatten()
            .or_else(|| monitors.into_iter().next())
        else {
            return;
        };
        let clamped = clamp_to_work_area(self.bar_rect, logical_work_area(&primary));
        if clamped != self.bar_rect {
            self.bar_rect = clamped;
            let target = self.target_rect();
            set_rect(&bar, target);
        }
    }

    /// Primary monitor's work area (first available as fallback).
    fn primary_work_area(&self) -> Rect {
        let Some(bar) = &self.bar else {
            return DEFAULT_WORK;
        };
        if let Some(p) = bar.primary_monitor().ok().flatten() {
            return logical_work_area(&p);
        }
        match bar.available_monitors() {
            Ok(ms) if !ms.is_empty() => logical_work_area(&ms[0]),
            _ => DEFAULT_WORK,
        }
    }

    /// Work area containing the bar's center; primary (or first) monitor if
    /// the bar is off-screen; `DEFAULT_WORK` if every query fails.
    fn bar_work_area(&self) -> Rect {
        let Some(bar) = &self.bar else {
            return DEFAULT_WORK;
        };
        let Ok(monitors) = bar.available_monitors() else {
            return DEFAULT_WORK;
        };
        if monitors.is_empty() {
            return DEFAULT_WORK;
        }
        let (cx, cy) = (self.bar_rect.center_x(), self.bar_rect.center_y());
        if let Some(m) = monitors
            .iter()
            .find(|m| logical_work_area(m).contains_point(cx, cy))
        {
            return logical_work_area(m);
        }
        match bar.primary_monitor().ok().flatten() {
            Some(p) => logical_work_area(&p),
            None => logical_work_area(&monitors[0]),
        }
    }

    /// Keep Tahoe's resize affordance honest — macOS 26 edge-drags
    /// borderless windows even with `resizable(false)`, and the ↔
    /// cursor still appears on the side edges. Pinning min == max on
    /// the width axis makes AppKit drop the horizontal affordance and
    /// clamps the drag itself (`enforce_bar_bounds` stays as the
    /// backstop). An open card keeps a free height axis within
    /// `[BAR_H + MIN_CHAT_H, free space in the grow direction]` — the
    /// same bound `expanded_rect` applies, so an edge-drag can't pull
    /// the card past the work-area edge it grows toward. Programmatic
    /// `set_size` ignores these limits, so the morph animations are
    /// unaffected.
    fn sync_bar_size_limits(&self) {
        let Some(bar) = &self.bar else { return };
        // `bar_rect` still holds the DEFAULT_WORK sentinel until
        // `position_bar_at_startup` runs — a real bar is never wider
        // than EXPANDED_W.
        if self.bar_rect.w > layout::EXPANDED_W {
            return;
        }
        let (w, min_h, max_h) = if self.chat_open {
            let work = self.bar_work_area();
            let free = if self.expand_dir == Dir::Up {
                self.bar_rect.bottom() - work.y
            } else {
                work.bottom() - self.bar_rect.y
            };
            (
                layout::EXPANDED_W,
                BAR_H + layout::MIN_CHAT_H,
                free.max(BAR_H + layout::MIN_CHAT_H),
            )
        } else {
            (self.bar_rect.w, BAR_H, BAR_H)
        };
        let _ = bar.set_min_size(Some(LogicalSize::new(w, min_h)));
        let _ = bar.set_max_size(Some(LogicalSize::new(w, max_h)));
    }

    /// Snap the bar back to its canonical bounds after a stray user
    /// resize — macOS 26 edge-drags resize borderless windows even with
    /// `resizable(false)`. The pill is fixed-size and restores whole;
    /// the open card restores x/width and re-pins the anchored edge
    /// (an edge-drag always changes `w`, and a corner drag can move
    /// `y` too), keeping the dragged height the webview adopts through
    /// `window_adjust_height`. Called on `Resized` when no animation is
    /// in flight.
    fn enforce_bar_bounds(&self) {
        let Some(bar) = &self.bar else { return };
        // `bar_rect` still holds the DEFAULT_WORK sentinel until
        // `position_bar_at_startup` runs — a real bar is never wider
        // than EXPANDED_W.
        if self.bar_rect.w > layout::EXPANDED_W {
            return;
        }
        let Some(live) = window_rect(bar) else { return };
        let off = |a: f64, b: f64| (a - b).abs() > 0.5;
        if self.chat_open {
            if !off(live.w, layout::EXPANDED_W) {
                return;
            }
            let target = self.target_rect();
            // Re-pin the anchored edge: grow-down keeps the pill's top,
            // grow-up keeps its bottom — otherwise a corner drag leaves
            // the card detached until the next `window_adjust_height`.
            let y = if self.expand_dir == Dir::Up {
                target.bottom() - live.h
            } else {
                target.y
            };
            set_rect(
                bar,
                Rect {
                    x: target.x,
                    y,
                    w: target.w,
                    ..live
                },
            );
        } else if off(live.w, self.bar_rect.w) || off(live.h, self.bar_rect.h) {
            set_rect(bar, self.bar_rect);
        }
    }

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

    /// The resting capsule rect — what `persist_bar_position` writes.
    /// Expansion recenters on center-x, so the persisted anchor is always
    /// the idle capsule: a drag on the expanded bar must not shift where
    /// the capsule reappears next launch.
    pub(crate) fn idle_bar_rect(&mut self) -> Rect {
        self.refresh_bar_rect();
        Rect {
            x: self.bar_rect.x + (self.bar_rect.w - BAR_IDLE_W) / 2.0,
            w: BAR_IDLE_W,
            ..self.bar_rect
        }
    }
}

/// Shared builder flags for every Marvis overlay window (spec): frameless,
/// transparent, always-on-top, non-resizable, skip-taskbar, no shadow —
/// then `set_visible_on_all_workspaces`, `set_content_protected`, and the
/// liquid-glass material. `corner_radius` matches the surface's CSS radius —
/// the glass view fills the window, so its shape IS the surface shape.
/// `app.accent` (`#rrggbb`) at 8% alpha (`{accent}15`) → the bar's glass
/// `tint_color`.
/// The alpha is load-bearing: the pre-26 `NSVisualEffectView` fallback
/// paints the tint as an overlay fill, so an opaque value would bury
/// the vibrancy entirely — and even on glass, a stronger tint reads
/// heavy over the capsule.
fn accent_glass_tint(accent: &str) -> Option<String> {
    let accent = accent.trim();
    (accent.len() == 7 && accent.starts_with('#')).then(|| format!("{accent}15"))
}

fn build_window(
    app: &AppHandle,
    label: &str,
    w: f64,
    h: f64,
    corner_radius: f64,
    tint_color: Option<String>,
) -> anyhow::Result<WebviewWindow> {
    let url = WebviewUrl::App(format!("index.html?view={label}").into());
    let win = WebviewWindowBuilder::new(app, label, url)
        .inner_size(w, h)
        // Pin user-resize to the built size: Tahoe edge-drags borderless
        // windows even with `resizable(false)`, and min == max drops the
        // affordance/cursor. Programmatic `set_size` is not limited, so
        // the bar's morphs still animate — `sync_bar_size_limits`
        // re-pins to each canonical width afterward.
        .min_inner_size(w, h)
        .max_inner_size(w, h)
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
    // set_effect dispatches to the main queue and blocks on it; callers
    // hold the pool lock, which main-thread callbacks also take — apply
    // on a detached thread so the lock is never held across the wait.
    // Warn-only on failure, same as the calls above.
    let app = app.clone();
    let label = label.to_string();
    std::thread::spawn({
        let win = win.clone();
        move || {
            if let Err(e) = app.liquid_glass().set_effect(
                &win,
                LiquidGlassConfig {
                    corner_radius,
                    tint_color,
                    ..Default::default()
                },
            ) {
                log::warn!("windows: liquid glass failed for {label}: {e}");
            }
        }
    });
    Ok(win)
}

/// Re-apply the liquid-glass corner radius on a live window — capsule
/// (`BAR_H/2`) ⇄ card (`CARD_RADIUS`). Same warn-only detached-thread
/// pattern as `build_window`: the call blocks on the main queue and the
/// pool lock must not be held across the wait.
fn set_glass_radius(app: &AppHandle, win: &WebviewWindow, corner_radius: f64) {
    let app = app.clone();
    let win = win.clone();
    std::thread::spawn(move || {
        // Re-supply the accent tint — `set_effect` CLEARS the tint when
        // `tint_color` is `None`, so a radius-only re-apply would strip
        // it on every pill⇄card morph.
        let tint_color = accent_glass_tint(&app.state::<crate::AppState>().accent());
        if let Err(e) = app.liquid_glass().set_effect(
            &win,
            LiquidGlassConfig {
                corner_radius,
                tint_color,
                ..Default::default()
            },
        ) {
            log::warn!("windows: liquid glass radius update failed: {e}");
        }
    });
}

/// The prefs window is deliberately NOT built by [`build_window`]: it's a
/// real macOS window, not overlay chrome — native decorations, normal
/// focus — but it IS a liquid-glass surface: `TitleBarStyle::Overlay` +
/// `hidden_title` make the content full-size so the traffic lights
/// land inside the sidebar (the Settings.app look) and
/// `GlassMaterialVariant::AbuttedSidebar` paints the whole window in
/// the edge-abutting sidebar material — the same variant Settings.app
/// uses. It joins the bar's floating
/// level ONLY while focused (so it can overlap the bar the user keeps
/// on top) and drops back on blur — focus events keep `always_on_top`
/// mirroring the window's active state. It stays content-protected
/// (the privacy spec holds for every window) and joinable on all
/// workspaces so it can be summoned over any space.
/// `CloseRequested` is intercepted into a hide: the window is owned by
/// the pool for the app's lifetime, so the red light must not destroy
/// the webview (a fresh build would lose scroll/tab state).
fn build_prefs_window(app: &AppHandle) -> anyhow::Result<WebviewWindow> {
    let url = WebviewUrl::App(format!("index.html?view={PREFS_LABEL}").into());
    let win = WebviewWindowBuilder::new(app, PREFS_LABEL, url)
        .inner_size(PREFS_W, PREFS_H)
        .title("Marvis — Settings")
        .decorations(true)
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .transparent(true)
        .resizable(false)
        .visible(false)
        // Tauri's file-drop handler swallows element drags inside the
        // webview — the Providers list's HTML5 reorder needs the native
        // path. Marvis consumes no OS file drops, so nothing is lost.
        .disable_drag_drop_handler()
        .build()?;
    {
        let handle = win.clone();
        let app = app.clone();
        win.on_window_event(move |event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = handle.set_always_on_top(false);
                let _ = handle.hide();
                // Closing a wizard re-exposes the bar if onboarding is
                // already done (a re-run); on a true first run the flag
                // is still false and the bar correctly stays hidden.
                app.state::<crate::AppState>().sync_bar_visibility();
            }
            // Float only while active: focused prefs may overlap the
            // always-on-top bar; on blur it drops to the normal level.
            tauri::WindowEvent::Focused(focused) => {
                let _ = handle.set_always_on_top(*focused);
            }
            _ => {}
        });
    }
    if let Err(e) = win.set_visible_on_all_workspaces(true) {
        log::warn!("windows: set_visible_on_all_workspaces failed for prefs: {e}");
    }
    if let Err(e) = win.set_content_protected(true) {
        log::warn!("windows: set_content_protected failed for prefs: {e}");
    }
    // set_effect dispatches to the main queue and blocks on it; the pool
    // lock is held by `show_prefs`'s caller path, which main-thread
    // callbacks also take — apply on a detached thread so the lock is
    // never held across the wait. Warn-only on failure, same as above.
    std::thread::spawn({
        let app = app.clone();
        let win = win.clone();
        move || {
            if let Err(e) = app.liquid_glass().set_effect(
                &win,
                LiquidGlassConfig {
                    corner_radius: 0.0,
                    tint_color: None,
                    variant: GlassMaterialVariant::AbuttedSidebar,
                    ..Default::default()
                },
            ) {
                log::warn!("windows: liquid glass failed for prefs: {e}");
            }
        }
    });
    Ok(win)
}

/// A monitor's work area in LOGICAL pixels (`work_area()` returns physical).
fn logical_work_area(m: &Monitor) -> Rect {
    let scale = m.scale_factor();
    let wa = m.work_area();
    let pos: LogicalPosition<f64> = wa.position.to_logical(scale);
    let size: LogicalSize<f64> = wa.size.to_logical(scale);
    Rect {
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
    }
}

/// Live window bounds in LOGICAL pixels (outer position/size are physical).
fn window_rect(win: &WebviewWindow) -> Option<Rect> {
    let scale = win.scale_factor().ok()?;
    let pos: LogicalPosition<f64> = win.outer_position().ok()?.to_logical(scale);
    let size: LogicalSize<f64> = win.outer_size().ok()?.to_logical(scale);
    Some(Rect {
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
    })
}

/// Instant (non-animated) bounds application, logical pixels.
fn set_rect(win: &WebviewWindow, r: Rect) {
    let _ = win.set_size(LogicalSize::new(r.w, r.h));
    let _ = win.set_position(LogicalPosition::new(r.x, r.y));
}
