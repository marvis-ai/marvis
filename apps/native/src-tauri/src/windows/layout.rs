//! Pure layout math for the bar + feature panels — no Tauri calls, all
//! logical pixels. Unit-tested exhaustively.

use std::collections::{BTreeMap, BTreeSet};

use super::{Dir, Panel, Rect};

/// Vertical gap between the bar's bottom edge and the panel row, and the
/// horizontal gap between adjacent panels.
pub const PANEL_PAD: f64 = 8.0;
/// Margin kept between the bar and a work-area edge on `snap_edge`.
pub const EDGE_MARGIN: f64 = 12.0;

/// Rects for every panel in `visible`, stacked under `bar`.
///
/// Rules (spec):
/// - `ask` 600w centered under the bar (`y = bar.bottom + 8`).
/// - `listen` 400w to the LEFT of `ask` when both are visible (8 px gap),
///   else centered under the bar.
///
/// Settings is no longer a panel — it lives in the decorated `prefs`
/// window, which the pool positions independently.
///
/// Heights are each panel's [`Panel::default_height`]; `WindowPool` overrides
/// them with stored content heights when applying the layout.
pub fn panel_rects(bar: Rect, visible: &BTreeSet<Panel>) -> BTreeMap<Panel, Rect> {
    let mut out = BTreeMap::new();
    let y = bar.bottom() + PANEL_PAD;

    if visible.contains(&Panel::Ask) {
        out.insert(
            Panel::Ask,
            Rect {
                x: bar.center_x() - Panel::Ask.width() / 2.0,
                y,
                w: Panel::Ask.width(),
                h: Panel::Ask.default_height(),
            },
        );
    }

    if visible.contains(&Panel::Listen) {
        let x = match out.get(&Panel::Ask) {
            Some(ask) => ask.x - Panel::Listen.width() - PANEL_PAD,
            None => bar.center_x() - Panel::Listen.width() / 2.0,
        };
        out.insert(
            Panel::Listen,
            Rect {
                x,
                y,
                w: Panel::Listen.width(),
                h: Panel::Listen.default_height(),
            },
        );
    }

    out
}

/// Keep `rect`'s position inside `work`. Position-only clamp — a rect larger
/// than the work area keeps its top-left edge inside and overflows.
pub fn clamp_to_work_area(rect: Rect, work: Rect) -> Rect {
    let x = if rect.w >= work.w {
        work.x
    } else {
        rect.x.clamp(work.x, work.right() - rect.w)
    };
    let y = if rect.h >= work.h {
        work.y
    } else {
        rect.y.clamp(work.y, work.bottom() - rect.h)
    };
    Rect { x, y, ..rect }
}

/// Bar position snapped to the named work-area edge with a 12 px margin.
/// `Left`/`Right` move x (y kept); `Up`/`Down` move y (x kept).
pub fn snap_edge(bar: Rect, dir: Dir, work: Rect) -> (f64, f64) {
    match dir {
        Dir::Left => (work.x + EDGE_MARGIN, bar.y),
        Dir::Right => (work.right() - bar.w - EDGE_MARGIN, bar.y),
        Dir::Up => (bar.x, work.y + EDGE_MARGIN),
        Dir::Down => (bar.x, work.bottom() - bar.h - EDGE_MARGIN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: Rect = Rect {
        x: 100.0,
        y: 21.0,
        w: 353.0,
        h: 47.0,
    };
    const WORK: Rect = Rect {
        x: 0.0,
        y: 25.0,
        w: 1440.0,
        h: 875.0,
    };

    fn vis(panels: &[Panel]) -> BTreeSet<Panel> {
        panels.iter().copied().collect()
    }

    #[test]
    fn ask_is_centered_under_bar() {
        let rects = panel_rects(BAR, &vis(&[Panel::Ask]));
        let ask = rects[&Panel::Ask];
        assert_eq!(ask.w, 600.0);
        assert_eq!(ask.y, BAR.bottom() + 8.0);
        assert_eq!(ask.center_x(), BAR.center_x());
    }

    #[test]
    fn listen_offsets_left_of_ask_when_both_visible() {
        let rects = panel_rects(BAR, &vis(&[Panel::Ask, Panel::Listen]));
        let ask = rects[&Panel::Ask];
        let listen = rects[&Panel::Listen];
        assert_eq!(listen.w, 400.0);
        assert_eq!(listen.right() + 8.0, ask.x);
        assert_eq!(listen.y, ask.y);
    }

    #[test]
    fn listen_is_centered_when_ask_hidden() {
        let rects = panel_rects(BAR, &vis(&[Panel::Listen]));
        let listen = rects[&Panel::Listen];
        assert_eq!(listen.center_x(), BAR.center_x());
        assert_eq!(listen.y, BAR.bottom() + 8.0);
    }

    #[test]
    fn both_visible_do_not_overlap() {
        let rects = panel_rects(BAR, &vis(&[Panel::Ask, Panel::Listen]));
        assert_eq!(rects.len(), 2);
        let v: Vec<Rect> = rects.values().copied().collect();
        for i in 0..v.len() {
            for j in (i + 1)..v.len() {
                assert!(!v[i].intersects(&v[j]), "rects {i} and {j} overlap: {v:?}");
            }
        }
    }

    #[test]
    fn snap_right_lands_twelve_px_from_edge() {
        let (x, y) = snap_edge(BAR, Dir::Right, WORK);
        assert_eq!(x, WORK.right() - BAR.w - 12.0);
        assert_eq!(y, BAR.y);
    }

    #[test]
    fn snap_left_up_down_use_named_edge() {
        assert_eq!(snap_edge(BAR, Dir::Left, WORK), (WORK.x + 12.0, BAR.y));
        assert_eq!(snap_edge(BAR, Dir::Up, WORK), (BAR.x, WORK.y + 12.0));
        assert_eq!(
            snap_edge(BAR, Dir::Down, WORK),
            (BAR.x, WORK.bottom() - BAR.h - 12.0)
        );
    }

    #[test]
    fn clamp_keeps_rect_inside_work_area() {
        let off = Rect {
            x: -50.0,
            y: 10.0,
            w: 200.0,
            h: 100.0,
        };
        let c = clamp_to_work_area(off, WORK);
        assert_eq!(c.x, WORK.x);
        assert_eq!(c.y, WORK.y);

        let off = Rect {
            x: 2000.0,
            y: 5000.0,
            w: 200.0,
            h: 100.0,
        };
        let c = clamp_to_work_area(off, WORK);
        assert_eq!(c.right(), WORK.right());
        assert_eq!(c.bottom(), WORK.bottom());

        let inside = Rect {
            x: 100.0,
            y: 100.0,
            w: 200.0,
            h: 100.0,
        };
        assert_eq!(clamp_to_work_area(inside, WORK), inside);
    }

    #[test]
    fn clamp_oversized_rect_keeps_top_left_inside() {
        let big = Rect {
            x: 500.0,
            y: 600.0,
            w: 2000.0,
            h: 2000.0,
        };
        let c = clamp_to_work_area(big, WORK);
        assert_eq!((c.x, c.y), (WORK.x, WORK.y));
        assert!(c.w > WORK.w); // size untouched — position-only clamp
    }
}
