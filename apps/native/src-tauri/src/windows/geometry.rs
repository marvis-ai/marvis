use super::*;

/// A monitor's work area in LOGICAL pixels (`work_area()` returns physical).
pub(super) fn logical_work_area(m: &Monitor) -> Rect {
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
pub(super) fn window_rect(win: &WebviewWindow) -> Option<Rect> {
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
pub(super) fn set_rect(win: &WebviewWindow, r: Rect) {
    let _ = win.set_size(LogicalSize::new(r.w, r.h));
    let _ = win.set_position(LogicalPosition::new(r.x, r.y));
}

/// Center `win` on the monitor containing the pointer — physical px
/// math (monitor position/size are physical; the window's logical
/// size scales by `scale_factor`). Primary monitor on any miss.
pub(super) fn center_on_pointer_display(win: &WebviewWindow) {
    let monitor = win
        .cursor_position()
        .ok()
        .and_then(|pos| {
            // `cursor_position` is physical f64; monitor bounds are
            // physical i32/u32 — compare in i32 space.
            let (cx, cy) = (pos.x as i32, pos.y as i32);
            win.available_monitors().ok()?.into_iter().find(|m| {
                let (p, s) = (m.position(), m.size());
                cx >= p.x && cx < p.x + s.width as i32 && cy >= p.y && cy < p.y + s.height as i32
            })
        })
        .or_else(|| win.primary_monitor().ok().flatten())
        .or_else(|| win.available_monitors().ok()?.into_iter().next());
    let Some(mon) = monitor else { return };
    let scale = mon.scale_factor();
    let (mp, ms) = (mon.position(), mon.size());
    let (pw, ph) = ((PICKER_W * scale) as i32, (PICKER_H * scale) as i32);
    let x = mp.x + (ms.width as i32 - pw).max(0) / 2;
    let y = mp.y + (ms.height as i32 - ph).max(0) / 2;
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
}
