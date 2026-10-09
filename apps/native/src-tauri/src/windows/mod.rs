//! Window pool, layout math, and the movement animator.
//!
//! ALL geometry in this module is LOGICAL pixels (`f64`). `Monitor::work_area()`
//! returns PHYSICAL pixels and is converted with `.to_logical(scale_factor)`
//! at the boundary — never mix the two (the prior Retina bug).

pub mod layout;
pub mod movement;

mod build;
mod geometry;
mod pool;

use self::geometry::*;
pub(crate) use pool::WindowPool;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Monitor, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

#[cfg(target_os = "macos")]
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

/// The anchor a palette open resolved — `x` is the caret's screen-x.
/// `top` exists only on card-mode opens: the composer is the card's
/// bottom-anchored footer, so the palette pops above the row's
/// screen-top (`bar_rect`'s edge is the card's TOP when it grows
/// down, nowhere near the input). A height reflow re-anchors to the
/// same spot.
#[derive(Debug, Clone, Copy)]
struct PaletteAnchor {
    x: f64,
    top: Option<f64>,
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
/// Card content height's initial field value — `set_chat_open` resets
/// it to the 30% floor on every expand, so this only exists before the
/// first open.
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
/// The share-picker surface (`?view=picker`) — lazy like `prefs`,
/// borderless glass via `build_window` (own-pid filtered, so it
/// never appears in its own candidate list).
pub const PICKER_LABEL: &str = "picker";
const PICKER_W: f64 = 760.0;
const PICKER_H: f64 = 560.0;
const PICKER_RADIUS: f64 = 16.0;
/// The preset palette (`?view=palette`) — the wand's small Marvis-glass
/// overlay anchored above/below the bar at the pointer, sized for the
/// merged preset list (scrolls past ~6 rows) and the future skills
/// surface. Lazy like `picker`; a focus loss hides it, menu-style.
pub const PALETTE_LABEL: &str = "palette";
const PALETTE_W: f64 = 300.0;
/// Height is content-driven — the view reports its natural height
/// (`presets_palette_height`), clamped to these bounds; the list
/// scrolls past `PALETTE_MAX_H` (~6 rows ≈ 224 — the default shows
/// six, a 7th row peeks over the cap as the scroll affordance).
const PALETTE_H: f64 = 228.0;
/// Floor for the content-height clamp — kept just under a single
/// row's natural height (~68: header + row + list padding); a taller
/// floor leaves bare glass below the list.
const PALETTE_MIN_H: f64 = 64.0;
const PALETTE_MAX_H: f64 = 228.0;
const PALETTE_RADIUS: f64 = 18.0;
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

