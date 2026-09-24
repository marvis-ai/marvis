//! Window bounds animator — a 60 Hz `tauri::async_runtime` task lerping
//! `set_position`/`set_size` over ~180 ms with ease-out cubic.
//!
//! Per-window cancellation: a second `animate` on the same window cancels the
//! first via an `AtomicBool` flag kept in a label-keyed map (the task exits on
//! its next tick; the new task picks up from the live bounds).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{LogicalPosition, LogicalSize, WebviewWindow};

use super::{window_rect, Rect};

const TICK: Duration = Duration::from_millis(16);

fn cancel_flags() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static FLAGS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    FLAGS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Whether an `animate` task is currently driving `label`'s bounds —
/// `Resized` enforcement must skip those events (the animator's own
/// `set_size` ticks emit them too).
pub fn is_animating(label: &str) -> bool {
    cancel_flags().lock().unwrap().contains_key(label)
}

/// Animate `window`'s bounds to `to` over `dur`. `WebviewWindow` has no
/// atomic bounds setter in Tauri 2 (`Webview::set_bounds` resizes the webview
/// inside the window, not the window), so position and size are applied
/// per tick.
pub fn animate(window: &WebviewWindow, to: Rect, dur: Duration) {
    let label = window.label().to_string();
    let flag = {
        let flag = Arc::new(AtomicBool::new(false));
        let mut map = cancel_flags().lock().unwrap();
        if let Some(prev) = map.insert(label.clone(), flag.clone()) {
            prev.store(true, Ordering::SeqCst);
        }
        flag
    };
    let win = window.clone();
    tauri::async_runtime::spawn(async move {
        let from = window_rect(&win).unwrap_or(to);
        let start = Instant::now();
        loop {
            if flag.load(Ordering::SeqCst) {
                return;
            }
            let t = (start.elapsed().as_secs_f64() / dur.as_secs_f64()).clamp(0.0, 1.0);
            let r = lerp(from, to, ease_out_cubic(t));
            let _ = win.set_size(LogicalSize::new(r.w, r.h));
            let _ = win.set_position(LogicalPosition::new(r.x, r.y));
            if t >= 1.0 {
                break;
            }
            tokio::time::sleep(TICK).await;
        }
        // Remove our flag only if no newer animate() replaced it.
        let mut map = cancel_flags().lock().unwrap();
        if map.get(&label).is_some_and(|f| Arc::ptr_eq(f, &flag)) {
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
