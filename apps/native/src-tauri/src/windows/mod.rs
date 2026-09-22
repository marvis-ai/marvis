//! Window pool, layout math, and the movement animator.
//!
//! ALL geometry in this module is LOGICAL pixels (`f64`). `Monitor::work_area()`
//! returns PHYSICAL pixels and is converted with `.to_logical(scale_factor)`
//! at the boundary — never mix the two (the prior Retina bug).

pub mod layout;
pub mod movement;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Monitor, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use tauri_plugin_liquid_glass::{LiquidGlassConfig, LiquidGlassExt};

use layout::{clamp_to_work_area, panel_rects};

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

/// Feature panel windows that stack under the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Panel {
    Ask,
    Listen,
}

impl Panel {
    pub const ALL: [Panel; 2] = [Panel::Ask, Panel::Listen];

    /// Window label — also the `?view=` query value.
    pub fn label(self) -> &'static str {
        match self {
            Panel::Ask => "ask",
            Panel::Listen => "listen",
        }
    }

    /// Resolve a window label string back to a panel (`window_adjust_height`
    /// passes the label through unmodified).
    pub fn from_label(label: &str) -> Option<Panel> {
        Some(match label {
            "ask" => Panel::Ask,
            "listen" => Panel::Listen,
            _ => return None,
        })
    }

    pub fn width(self) -> f64 {
        match self {
            Panel::Ask => 600.0,
            Panel::Listen => 400.0,
        }
    }

    /// `adjust_height` upper bound (spec: ask/listen ≤900).
    pub fn max_height(self) -> f64 {
        match self {
            Panel::Ask | Panel::Listen => 900.0,
        }
    }

    /// Height used until the webview reports its content height.
    pub fn default_height(self) -> f64 {
        match self {
            Panel::Ask | Panel::Listen => 480.0,
        }
    }

    /// Glass corner radius matching the panel's CSS card radius.
    fn corner_radius(self) -> f64 {
        match self {
            Panel::Ask => 18.0,
            Panel::Listen => 16.0,
        }
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

const BAR_W: f64 = 480.0;
const BAR_H: f64 = 64.0;
/// The preferences window — a normal decorated macOS window (native
/// traffic lights, opaque, NOT always-on-top), not an overlay panel.
/// Label `prefs`, `?view=prefs`; it hosts both the settings sidebar and
/// the onboarding wizard, switched by `prefs:mode` emits.
pub const PREFS_LABEL: &str = "prefs";
const PREFS_W: f64 = 720.0;
const PREFS_H: f64 = 520.0;
/// Transient alert toast — its own window because the bar is a fixed
/// 480×64 pill with no room for an error row (the old inline row
/// squeezed the pill's content). Label `alert`, `?view=alert`.
pub const ALERT_LABEL: &str = "alert";
const ALERT_W: f64 = 340.0;
/// Fixed toast height — informational only, so one layout suffices.
const ALERT_H: f64 = 100.0;
/// Distance below the work-area top for the default bar position.
const BAR_TOP_OFFSET: f64 = 21.0;
/// Slide-in distance above the target rect when a panel appears.
const SHOW_OFFSET_Y: f64 = 10.0;
/// Lower bound for `adjust_height` so a panel can't collapse to nothing.
const MIN_PANEL_H: f64 = 40.0;
/// Fallback work area if every monitor query fails.
const DEFAULT_WORK: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 1920.0,
    h: 1080.0,
};
/// Show/restack animation duration (spec: ~200 ms; ~180 ms reads well).
const ANIM_DUR: std::time::Duration = std::time::Duration::from_millis(180);

/// Owns the bar + feature-panel windows and orchestrates their layout.
///
/// Lives inside `AppState` (Task 14) behind a `Mutex`; all methods take
/// `&self`/`&mut self`.
pub struct WindowPool {
    bar: Option<WebviewWindow>,
    /// Alert toast — built with the bar (not a [`Panel`]: it never joins
    /// the stacking row, `toggle_all`, or the gate's panel lifecycle, and
    /// it must exist before `Main` to report startup failures).
    alert: Option<WebviewWindow>,
    /// The decorated preferences window (settings + onboarding). Built
    /// lazily on first `show_prefs` — unlike panels it is NOT part of
    /// `Main` (settings and the wizard must work before the gate opens).
    prefs: Option<WebviewWindow>,
    /// The mode `prefs` was last opened in (`"settings"|"onboarding"`) —
    /// the `prefs_mode` command returns it so a `prefs:mode` emit that
    /// raced a still-loading webview isn't lost.
    prefs_mode: String,
    panels: BTreeMap<Panel, WebviewWindow>,
    visible: BTreeSet<Panel>,
    /// Panels visible before the last `toggle_all` hide — restored on show.
    remembered: BTreeSet<Panel>,
    /// Last known bar rect (logical). Refreshed from the live window before
    /// layout so user drags via `data-tauri-drag-region` aren't lost.
    bar_rect: Rect,
    /// Content-driven heights reported by `adjust_height` (defaults until then).
    heights: BTreeMap<Panel, f64>,
}

/// Debounce counter for the bar's `Moved` event — a drag fires one event
/// per frame, so the persist write waits for the LAST position (400 ms
/// quiet) instead of rewriting `config.toml` on every pixel.
static BAR_MOVE_GEN: AtomicU64 = AtomicU64::new(0);

impl WindowPool {
    /// Build ONLY the bar window; panels are created later by
    /// [`create_feature_windows`](Self::create_feature_windows) once the
    /// gate opens (onboarding done + screen permission). `show_bar` is
    /// the same `onboarding_done` flag — first-run installs keep the bar
    /// hidden until the wizard finishes; `set_bar_shown` owns it after.
    pub fn create_bar_only(app: &AppHandle, show_bar: bool) -> anyhow::Result<Self> {
        let mut pool = Self {
            bar: None,
            alert: None,
            prefs: None,
            prefs_mode: String::new(),
            panels: BTreeMap::new(),
            visible: BTreeSet::new(),
            remembered: BTreeSet::new(),
            bar_rect: DEFAULT_WORK,
            heights: BTreeMap::new(),
        };
        let bar = build_window(app, "bar", BAR_W, BAR_H, BAR_H / 2.0)?;
        {
            // Persist the resting place on every move — user drags via
            // `data-tauri-drag-region`, edge snaps, and reclamps alike.
            // `Moved` fires per frame during a drag, so the write is
            // debounced: the last generation wins after 400 ms of quiet.
            let app_moved = app.clone();
            bar.on_window_event(move |event| {
                if !matches!(event, tauri::WindowEvent::Moved(_)) {
                    return;
                }
                let gen = BAR_MOVE_GEN.fetch_add(1, Ordering::Relaxed) + 1;
                let app = app_moved.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(400));
                    if BAR_MOVE_GEN.load(Ordering::Relaxed) == gen {
                        crate::persist_bar_position(&app);
                    }
                });
            });
        }
        pool.bar = Some(bar);
        // Built (hidden) up front so its webview is loaded and listening
        // before the first alert — an emit to a still-loading window
        // would be dropped.
        pool.alert = Some(build_window(app, ALERT_LABEL, ALERT_W, ALERT_H, 14.0)?);
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
            panels: BTreeMap::new(),
            visible: BTreeSet::new(),
            remembered: BTreeSet::new(),
            bar_rect: DEFAULT_WORK,
            heights: BTreeMap::new(),
        }
    }

    /// Build the feature panels HIDDEN. Must not be called before the
    /// gate opens; panels must not exist earlier.
    pub fn create_feature_windows(&mut self, app: &AppHandle) -> anyhow::Result<()> {
        for panel in Panel::ALL {
            if self.panels.contains_key(&panel) {
                continue;
            }
            let win = build_window(
                app,
                panel.label(),
                panel.width(),
                panel.default_height(),
                panel.corner_radius(),
            )?;
            self.panels.insert(panel, win);
        }
        Ok(())
    }

    pub fn bar(&self) -> Option<&WebviewWindow> {
        self.bar.as_ref()
    }

    #[allow(dead_code)] // accessor kept for future panel consumers
    pub fn panel_window(&self, panel: Panel) -> Option<&WebviewWindow> {
        self.panels.get(&panel)
    }

    #[allow(dead_code)] // panel accessors for later status/debug consumers
    pub fn is_visible(&self, panel: Panel) -> bool {
        self.visible.contains(&panel)
    }

    /// Slide `panel` in under the bar and restack the other visible panels.
    /// No-op if the panel window doesn't exist yet.
    pub fn show(&mut self, panel: Panel) {
        let Some(win) = self.panels.get(&panel).cloned() else {
            log::warn!("windows::show({:?}): panel not created", panel);
            return;
        };
        self.reclamp();
        self.visible.insert(panel);
        let targets = self.layout_targets();
        let Some(&target) = targets.get(&panel) else {
            return;
        };
        // Start ~10px above the target, hidden -> show -> slide into place.
        // (No per-window opacity in Tauri — position-only "fade".)
        set_rect(
            &win,
            Rect {
                y: target.y - SHOW_OFFSET_Y,
                ..target
            },
        );
        let _ = win.show();
        movement::animate(&win, target, ANIM_DUR);
        for (p, w) in &self.panels {
            if *p != panel && self.visible.contains(p) {
                if let Some(&t) = targets.get(p) {
                    movement::animate(w, t, ANIM_DUR);
                }
            }
        }
    }

    /// Slide the alert toast in, centered under the bar. Deliberately
    /// overlaps whatever panels are open and is shown last so it lands
    /// on top of them; it is never focused, so a bar/panel field keeps
    /// its focus.
    pub fn show_alert(&mut self) {
        let Some(win) = self.alert.clone() else {
            log::warn!("windows::show_alert: alert window not created");
            return;
        };
        self.reclamp();
        let target = clamp_to_work_area(
            Rect {
                x: self.bar_rect.center_x() - ALERT_W / 2.0,
                y: self.bar_rect.bottom() + layout::PANEL_PAD,
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
            let _ = win.hide();
        }
    }

    pub fn prefs_mode(&self) -> &str {
        &self.prefs_mode
    }

    /// Hide `panel` and restack the remaining visible panels.
    pub fn hide(&mut self, panel: Panel) {
        self.reclamp();
        if let Some(win) = self.panels.get(&panel) {
            let _ = win.hide();
        }
        self.visible.remove(&panel);
        self.restack();
    }

    /// `Cmd+/`: hide all visible panels (remembering the set), or restore the
    /// remembered set — showing `ask` if nothing was remembered.
    pub fn toggle_all(&mut self) {
        if !self.visible.is_empty() {
            self.remembered = self.visible.clone();
            for p in self.visible.clone() {
                if let Some(w) = self.panels.get(&p) {
                    let _ = w.hide();
                }
            }
            self.visible.clear();
            return;
        }
        self.reclamp();
        let restore = if self.remembered.is_empty() {
            BTreeSet::from([Panel::Ask])
        } else {
            self.remembered.clone()
        };
        for p in restore {
            self.show(p);
        }
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
    /// (12 px margin); panels follow. The move fires `Moved`, so the new
    /// position persists through the debounced write — no extra save here.
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
            movement::animate(bar, self.bar_rect, ANIM_DUR);
        }
        self.restack();
    }

    /// Settings → Bar "Re-center": restore the startup default — centered
    /// on the primary work area, 21 px under the top. Persists via the
    /// `Moved` debounce like any other move.
    pub fn recenter_bar(&mut self) {
        let work = self.primary_work_area();
        self.bar_rect = Rect {
            x: work.center_x() - BAR_W / 2.0,
            y: work.y + BAR_TOP_OFFSET,
            w: BAR_W,
            h: BAR_H,
        };
        if let Some(bar) = &self.bar {
            movement::animate(bar, self.bar_rect, ANIM_DUR);
        }
        self.restack();
    }

    /// `window_adjust_height(name, px)`: `name` is the window label
    /// (`"ask"|"listen"`). Clamps to the panel's max height and
    /// animates the bounds keeping the top edge anchored; result is re-clamped
    /// so the bottom stays inside the work area.
    pub fn adjust_height(&mut self, name: &str, px: f64) {
        if !px.is_finite() {
            log::warn!("windows::adjust_height: non-finite height {px} for {name:?}");
            return;
        }
        let Some(panel) = Panel::from_label(name) else {
            log::warn!("windows::adjust_height: unknown window label {name:?}");
            return;
        };
        let Some(win) = self.panels.get(&panel).cloned() else {
            return;
        };
        let h = px.clamp(MIN_PANEL_H, panel.max_height());
        self.heights.insert(panel, h);
        let Some(mut r) = window_rect(&win) else {
            let _ = win.set_size(LogicalSize::new(panel.width(), h));
            return;
        };
        r.h = h;
        let work = work_area_of(&win);
        let r = clamp_to_work_area(r, work);
        movement::animate(&win, r, ANIM_DUR);
    }

    /// Initial bar position: `config.window.bar_x/y` if BOTH are set, else
    /// centered 21 px under the work-area top. Re-clamps to the primary
    /// monitor when the saved position's center is off-screen.
    pub fn position_bar_at_startup(&mut self) {
        let prefs = crate::config::load().window;
        let rect = match (prefs.bar_x, prefs.bar_y) {
            (Some(x), Some(y)) => Rect {
                x,
                y,
                w: BAR_W,
                h: BAR_H,
            },
            _ => {
                let work = self.primary_work_area();
                Rect {
                    x: work.center_x() - BAR_W / 2.0,
                    y: work.y + BAR_TOP_OFFSET,
                    w: BAR_W,
                    h: BAR_H,
                }
            }
        };
        self.bar_rect = rect;
        if let Some(bar) = &self.bar {
            set_rect(bar, rect);
        }
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
            set_rect(&bar, clamped);
            self.restack();
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

    /// Target rects for all visible panels, with stored content heights.
    fn layout_targets(&self) -> BTreeMap<Panel, Rect> {
        panel_rects(self.bar_rect, &self.visible)
            .into_iter()
            .map(|(p, r)| {
                (
                    p,
                    Rect {
                        h: self.height_of(p),
                        ..r
                    },
                )
            })
            .collect()
    }

    fn height_of(&self, panel: Panel) -> f64 {
        self.heights
            .get(&panel)
            .copied()
            .unwrap_or_else(|| panel.default_height())
    }

    /// Animate every visible panel to its current layout target.
    fn restack(&mut self) {
        let targets = self.layout_targets();
        for (p, w) in &self.panels {
            if self.visible.contains(p) {
                if let Some(&t) = targets.get(p) {
                    movement::animate(w, t, ANIM_DUR);
                }
            }
        }
    }

    /// Pull the live bar rect from the OS so user drags stay authoritative.
    pub(crate) fn refresh_bar_rect(&mut self) {
        if let Some(bar) = &self.bar {
            if let Some(r) = window_rect(bar) {
                self.bar_rect = r;
            }
        }
    }

    /// The bar rect with the OS as the source of truth: refresh first,
    /// then return. The `Moved` debounce writer and `window_bar_edge`
    /// both read through here so a just-finished drag is never stale.
    pub(crate) fn current_bar_rect(&mut self) -> Rect {
        self.refresh_bar_rect();
        self.bar_rect
    }
}

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

/// The prefs window is deliberately NOT built by [`build_window`]: it's a
/// real macOS window, not overlay chrome — native decorations (traffic
/// lights + title), opaque, normal focus, no always-on-top. It stays
/// content-protected (the privacy spec holds for every window) and
/// joinable on all workspaces so it can be summoned over any space.
/// `CloseRequested` is intercepted into a hide: the window is owned by
/// the pool for the app's lifetime, so the red light must not destroy
/// the webview (a fresh build would lose scroll/tab state).
fn build_prefs_window(app: &AppHandle) -> anyhow::Result<WebviewWindow> {
    let url = WebviewUrl::App(format!("index.html?view={PREFS_LABEL}").into());
    let win = WebviewWindowBuilder::new(app, PREFS_LABEL, url)
        .inner_size(PREFS_W, PREFS_H)
        .title("Marvis — Settings")
        .decorations(true)
        .transparent(false)
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
        win.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = handle.hide();
                // Closing a wizard re-exposes the bar if onboarding is
                // already done (a re-run); on a true first run the flag
                // is still false and the bar correctly stays hidden.
                app.state::<crate::AppState>().sync_bar_visibility();
            }
        });
    }
    if let Err(e) = win.set_visible_on_all_workspaces(true) {
        log::warn!("windows: set_visible_on_all_workspaces failed for prefs: {e}");
    }
    if let Err(e) = win.set_content_protected(true) {
        log::warn!("windows: set_content_protected failed for prefs: {e}");
    }
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

/// Work area of the monitor a window currently sits on (logical).
fn work_area_of(win: &WebviewWindow) -> Rect {
    if let Ok(Some(m)) = win.current_monitor() {
        return logical_work_area(&m);
    }
    if let Ok(Some(p)) = win.primary_monitor() {
        return logical_work_area(&p);
    }
    DEFAULT_WORK
}
