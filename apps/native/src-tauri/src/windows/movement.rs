//! Window bounds animator — a 60 Hz `tauri::async_runtime` task lerping
//! `set_position`/`set_size` over ~180 ms with ease-out cubic.
//!
//! Per-window cancellation: a second `animate` on the same window cancels the
//! first via an `AtomicBool` flag kept in a label-keyed map (the task exits on
//! its next tick; the new task picks up from the bounds the old one wrote).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{LogicalPosition, LogicalSize, WebviewWindow};

use super::{window_rect, Rect};

const TICK: Duration = Duration::from_millis(16);

/// One in-flight animation per window label. `cancel` tells the driving
/// task to exit on its next tick; `last` is the bounds the task last
/// wrote — the continuation point a replacement animation chains from.
/// An OS read mid-flight samples position and size as two calls, and a
/// tick landing between them pairs a stale x with a fresh w: on
/// expansion (x falls while w grows) that tear is always right-biased,
/// so a fresh read can never be trusted while an animation runs.
struct Slot {
    cancel: AtomicBool,
    last: Mutex<Rect>,
}

fn slots() -> &'static Mutex<HashMap<String, Arc<Slot>>> {
    static SLOTS: OnceLock<Mutex<HashMap<String, Arc<Slot>>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Whether an `animate` task is currently driving `label`'s bounds —
/// `Resized` enforcement must skip those events (the animator's own
/// `set_size` ticks emit them too).
pub fn is_animating(label: &str) -> bool {
    slots().lock().unwrap().contains_key(label)
}

/// Animate `window`'s bounds to `to` over `dur`. `WebviewWindow` has no
/// atomic bounds setter in Tauri 2 (`Webview::set_bounds` resizes the webview
/// inside the window, not the window), so each tick applies position and
/// size inside ONE `run_on_main_thread` dispatch — two separate calls let
/// a frame composite the half-applied rect (the new width at the old x
/// overshoots the target's right edge mid-expansion, reading as a
/// rightward shove instead of a centered bloom).
pub fn animate(window: &WebviewWindow, to: Rect, dur: Duration) {
    let label = window.label().to_string();
    let (slot, prev) = {
        let mut map = slots().lock().unwrap();
        let slot = Arc::new(Slot {
            cancel: AtomicBool::new(false),
            last: Mutex::new(to),
        });
        let prev = map.insert(label.clone(), slot.clone());
        (slot, prev)
    };
    if let Some(prev) = &prev {
        prev.cancel.store(true, Ordering::SeqCst);
    }
    let win = window.clone();
    tauri::async_runtime::spawn(async move {
        let from = prev
            .map(|p| *p.last.lock().unwrap())
            .unwrap_or_else(|| window_rect(&win).unwrap_or(to));
        *slot.last.lock().unwrap() = from;
        let start = Instant::now();
        loop {
            if slot.cancel.load(Ordering::SeqCst) {
                return;
            }
            let t = (start.elapsed().as_secs_f64() / dur.as_secs_f64()).clamp(0.0, 1.0);
            let r = lerp(from, to, ease_out_cubic(t));
            *slot.last.lock().unwrap() = r;
            let win2 = win.clone();
            let _ = win.run_on_main_thread(move || {
                let _ = win2.set_size(LogicalSize::new(r.w, r.h));
                let _ = win2.set_position(LogicalPosition::new(r.x, r.y));
            });
            if t >= 1.0 {
                break;
            }
            tokio::time::sleep(TICK).await;
        }
        // Remove our slot only if no newer animate() replaced it.
        let mut map = slots().lock().unwrap();
        if map.get(&label).is_some_and(|s| Arc::ptr_eq(s, &slot)) {
            map.remove(&label);
        }
    });
}

fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

fn lerp(from: Rect, to: Rect, e: f64) -> Rect {
    let l = |a: f64, b: f64| a + (b - a) * e;
    Rect {
        x: l(from.x, to.x),
        y: l(from.y, to.y),
        w: l(from.w, to.w),
        h: l(from.h, to.h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_out_cubic_endpoints() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        // Ease-out: ahead of linear early, converging late.
        assert!(ease_out_cubic(0.5) > 0.5);
        assert!(ease_out_cubic(0.9) < 1.0);
    }

    #[test]
    fn lerp_endpoints_and_midpoint() {
        let a = Rect { x: 0.0, y: 10.0, w: 100.0, h: 50.0 };
        let b = Rect { x: 100.0, y: 30.0, w: 200.0, h: 90.0 };
        assert_eq!(lerp(a, b, 0.0), a);
        assert_eq!(lerp(a, b, 1.0), b);
        assert_eq!(
            lerp(a, b, 0.5),
            Rect { x: 50.0, y: 20.0, w: 150.0, h: 70.0 }
        );
    }
}
