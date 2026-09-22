//! Pure layout math for the bar + feature panels — no Tauri calls, all
//! logical pixels. Unit-tested exhaustively.

use std::collections::{BTreeMap, BTreeSet};

use super::{Dir, Panel, Rect};

/// Vertical gap between the bar's bottom edge and the panel row, and the
/// horizontal gap between adjacent panels.
pub const PANEL_PAD: f64 = 8.0;
/// Margin kept between the bar and a work-area edge on `snap_edge`.
pub const EDGE_MARGIN: f64 = 12.0;

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
    Rect {
        x,
        y,
        w: EXPANDED_W,
        h,
    }
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
        w: 441.0,
        h: 59.0,
    };
    const WORK: Rect = Rect {
        x: 0.0,
        y: 25.0,
        w: 1440.0,
        h: 875.0,
    };
    /// The input-mode pill (136-capsule expands identically — only
    /// center-x and the anchored edge matter).
    const PILL: Rect = Rect {
        x: 100.0,
        y: 33.0,
        w: 480.0,
        h: 64.0,
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
        let r = expanded_rect(PILL, Dir::Up, 200.0, WORK);
        assert_eq!(r.bottom(), PILL.bottom());
        // Only 72 px free above the anchored bottom → the 104 px floor
        // wins and the card overflows the work-area top rather than
        // moving the anchored edge.
        assert_eq!(r.h, PILL.h + MIN_CHAT_H);
        assert_eq!(r.y, PILL.bottom() - (PILL.h + MIN_CHAT_H));
        assert_eq!(r.center_x(), PILL.center_x());
    }

    #[test]
    fn expanded_rect_clamps_height_to_900_and_free_space() {
        // The 900 ceiling binds when the grow direction has ≥900 px free.
        let tall = Rect {
            x: 0.0,
            y: 0.0,
            w: 1440.0,
            h: 1200.0,
        };
        let r = expanded_rect(PILL, Dir::Down, 5000.0, tall);
        assert_eq!(r.h, 900.0);
        // In WORK the free space below PILL caps under the ceiling:
        // 900 − 33 = 867.
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
}
