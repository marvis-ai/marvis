//! Window pool, layout math, and the movement animator.
//!
//! ALL geometry in this module is LOGICAL pixels (`f64`). `Monitor::work_area()`
//! returns PHYSICAL pixels and is converted with `.to_logical(scale_factor)`
//! at the boundary — never mix the two (the prior Retina bug).

pub mod layout;
pub mod movement;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Monitor, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

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
    Settings,
}

impl Panel {
    pub const ALL: [Panel; 3] = [Panel::Ask, Panel::Listen, Panel::Settings];

    /// Window label — also the `?view=` query value.
    pub fn label(self) -> &'static str {
        match self {
            Panel::Ask => "ask",
            Panel::Listen => "listen",
            Panel::Settings => "settings",
        }
    }

    /// Resolve a window label string back to a panel (`window_adjust_height`
    /// passes the label through unmodified).
    pub fn from_label(label: &str) -> Option<Panel> {
        Some(match label {
            "ask" => Panel::Ask,
            "listen" => Panel::Listen,
            "settings" => Panel::Settings,
            _ => return None,
        })
    }

    pub fn width(self) -> f64 {
        match self {
            Panel::Ask => 600.0,
            Panel::Listen => 400.0,
            Panel::Settings => 240.0,
        }
    }

    /// `adjust_height` upper bound (spec: ask/listen ≤900, settings ≤400).
    pub fn max_height(self) -> f64 {
        match self {
            Panel::Ask | Panel::Listen => 900.0,
            Panel::Settings => 400.0,
        }
    }

    /// Height used until the webview reports its content height.
    pub fn default_height(self) -> f64 {
        match self {
            Panel::Ask | Panel::Listen => 480.0,
            Panel::Settings => 320.0,
        }
    }
}

/// Cardinal direction for bar step-move and edge-snap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

const BAR_W: f64 = 353.0;
const BAR_H: f64 = 47.0;
/// Distance below the work-area top for the default bar position.
const BAR_TOP_OFFSET: f64 = 21.0;
/// `Cmd+Arrow` step size.
const MOVE_STEP: f64 = 40.0;
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
/// Liquid-glass corner radius — spec elides the exact value.
const GLASS_CORNER_RADIUS: f64 = 16.0;

/// Owns the bar + feature-panel windows and orchestrates their layout.
///
/// Lives inside `AppState` (Task 14) behind a `Mutex`; all methods take
/// `&self`/`&mut self`.
pub struct WindowPool {
    bar: Option<WebviewWindow>,
    panels: BTreeMap<Panel, WebviewWindow>,
    visible: BTreeSet<Panel>,
    /// Panels visible before the last `toggle_all` hide — restored on show.
    remembered: BTreeSet<Panel>,
    /// Last known bar rect (logical). Refreshed from the live window before
    /// layout so user drags via `data-tauri-drag-region` aren't lost.
    bar_rect: Rect,
    /// Content-driven heights reported by `adjust_height` (defaults until then).
    heights: BTreeMap<Panel, f64>,
    click_through: bool,
}

impl WindowPool {
    /// Build ONLY the bar window and show it. Panels are created later by
    /// [`create_feature_windows`](Self::create_feature_windows) once the
    /// header-state gate opens (keystore unlocked + screen permission).
    pub fn create_bar_only(app: &AppHandle) -> anyhow::Result<Self> {
        let mut pool = Self {
            bar: None,
            panels: BTreeMap::new(),
            visible: BTreeSet::new(),
            remembered: BTreeSet::new(),
            bar_rect: DEFAULT_WORK,
            heights: BTreeMap::new(),
            click_through: false,
        };
        let bar = build_window(app, "bar", BAR_W, BAR_H)?;
        pool.bar = Some(bar);
        pool.position_bar_at_startup();
        if let Some(bar) = &pool.bar {
            let _ = bar.show();
        }
        Ok(pool)
    }

    /// Build the three feature panels HIDDEN. Must not be called before the
    /// Task-14 gate opens; panels must not exist earlier.
    pub fn create_feature_windows(&mut self, app: &AppHandle) -> anyhow::Result<()> {
        for panel in Panel::ALL {
            if self.panels.contains_key(&panel) {
                continue;
            }
            let win = build_window(app, panel.label(), panel.width(), panel.default_height())?;
            if self.click_through {
                let _ = win.set_ignore_cursor_events(true);
            }
            self.panels.insert(panel, win);
        }
        Ok(())
    }

    pub fn bar(&self) -> Option<&WebviewWindow> {
        self.bar.as_ref()
    }

    pub fn panel_window(&self, panel: Panel) -> Option<&WebviewWindow> {
        self.panels.get(&panel)
    }

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
        self.refresh_bar_rect();
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

    /// Hide `panel` and restack the remaining visible panels.
    pub fn hide(&mut self, panel: Panel) {
        self.refresh_bar_rect();
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

    /// `Cmd+M` click-through: ignore cursor events on every window.
    pub fn set_click_through(&mut self, on: bool) {
        self.click_through = on;
        if let Some(bar) = &self.bar {
            let _ = bar.set_ignore_cursor_events(on);
        }
        for w in self.panels.values() {
            let _ = w.set_ignore_cursor_events(on);
        }
    }

    /// `Cmd+Arrow`: move the bar 40 px, clamped to the work area; panels follow.
    pub fn move_bar_step(&mut self, dir: Dir) {
        self.reclamp();
        let work = self.bar_work_area();
        let mut b = self.bar_rect;
        match dir {
            Dir::Left => b.x -= MOVE_STEP,
            Dir::Right => b.x += MOVE_STEP,
            Dir::Up => b.y -= MOVE_STEP,
            Dir::Down => b.y += MOVE_STEP,
        }
        b = clamp_to_work_area(b, work);
        self.bar_rect = b;
        if let Some(bar) = &self.bar {
            set_rect(bar, b);
        }
        self.restack();
    }

    /// `Cmd+Shift+Arrow`: animate the bar to the named work-area edge (12 px
    /// margin); panels follow.
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

    /// `window_adjust_height(name, px)`: `name` is the window label
    /// (`"ask"|"listen"|"settings"`). Clamps to the panel's max height and
    /// animates the bounds keeping the top edge anchored; result is re-clamped
    /// so the bottom stays inside the work area.
    pub fn adjust_height(&mut self, name: &str, px: f64) {
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
    fn refresh_bar_rect(&mut self) {
        if let Some(bar) = &self.bar {
            if let Some(r) = window_rect(bar) {
                self.bar_rect = r;
            }
        }
    }
}

/// Shared builder flags for every Marvis window (spec): frameless,
/// transparent, always-on-top, non-resizable, skip-taskbar, no shadow —
/// then `set_visible_on_all_workspaces`, `set_content_protected`, and the
/// liquid-glass effect applied post-build.
fn build_window(app: &AppHandle, label: &str, w: f64, h: f64) -> anyhow::Result<WebviewWindow> {
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
    apply_glass(app, &win);
    Ok(win)
}

/// Liquid glass `Bubbles` from Rust at creation (spec). Cosmetic: any error —
/// including the plugin not being registered yet — is logged and ignored.
/// On macOS <26 the plugin falls back to `NSVisualEffectView` internally.
fn apply_glass(app: &AppHandle, win: &WebviewWindow) {
    use tauri_plugin_liquid_glass::{GlassMaterialVariant, LiquidGlassConfig};
    let Some(glass) = app.try_state::<tauri_plugin_liquid_glass::LiquidGlass<tauri::Wry>>() else {
        log::warn!("windows: liquid-glass plugin not registered; skipping effect");
        return;
    };
    let config = LiquidGlassConfig {
        variant: GlassMaterialVariant::Bubbles,
        corner_radius: GLASS_CORNER_RADIUS,
        ..Default::default()
    };
    if let Err(e) = glass.set_effect(win, config) {
        log::warn!("windows: liquid-glass set_effect failed: {e}");
    }
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
