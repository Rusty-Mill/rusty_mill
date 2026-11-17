//! Floating blocks' geometry (rusty_mux C42; gui-design D89, D91): a float
//! is a block drawn over the tiled layout, placed in whole percent of the
//! body so every client scales it to its own pixels.
//!
//! Pure: [`to_px`] and [`to_percent`] convert between the two, [`grab_at`]
//! says what a press on a float grabs, [`dragged`] where a drag leaves it,
//! and [`moved`] and [`resized`] step it by keys. The window draws and
//! commits; it asks the source for the new place only on mouse-up.

use super::layout::Rect;
use super::source::FloatAt;

/// The least a float's width or height may be, in percent of the body, so
/// a float stays big enough to grab (the server refuses less).
pub(crate) const MIN: u8 = 10;

/// How far a key moves or resizes a float, in percent of the body.
pub(crate) const STEP: u8 = 5;

/// The edge band a resize grabs, in logical px inside the border (D91).
pub(crate) const BAND: f32 = 6.0;

/// The sides of a float a press grabbed: a corner holds two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Edges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

/// What a press on a float takes hold of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grab {
    /// The header: dragging moves the float.
    Move,
    /// An edge or corner: dragging resizes it.
    Resize(Edges),
    /// The grid or another control: the click only focuses and raises.
    Body,
}

/// `value` percent of `total`, rounded to the nearest px.
fn share(total: usize, value: u8) -> usize {
    (total * usize::from(value) + 50) / 100
}

/// `px` of `total` as whole percent, rounded to the nearest.
fn percent(px: usize, total: usize) -> usize {
    match total {
        0 => 0,
        _ => (px * 100 + total / 2) / total,
    }
}

/// `at` in px on `area`, the body's box. Never past the area's edge and,
/// with an area of any size, at least one px.
pub(crate) fn to_px(at: FloatAt, area: Rect) -> Rect {
    let col = share(area.cols, at.x).min(area.cols.saturating_sub(1));
    let row = share(area.rows, at.y).min(area.rows.saturating_sub(1));
    let cols = share(area.cols, at.w).clamp(1, (area.cols - col).max(1));
    let rows = share(area.rows, at.h).clamp(1, (area.rows - row).max(1));
    Rect::new(area.col + col, area.row + row, cols, rows)
}

/// The place `r` (px, in the body `area`) holds, as whole percent: kept at
/// least [`MIN`] wide and tall and inside the body, as the server asks.
pub(crate) fn to_percent(r: Rect, area: Rect) -> FloatAt {
    let inside = |at: usize, len: usize, from: usize, total: usize| {
        let len = percent(len, total).clamp(usize::from(MIN), 100);
        let at = percent(at.saturating_sub(from), total).min(100 - len);
        (at as u8, len as u8)
    };
    let (x, w) = inside(r.col, r.cols, area.col, area.cols);
    let (y, h) = inside(r.row, r.rows, area.row, area.rows);
    FloatAt { x, y, w, h }
}

/// What a press at `(x, y)` takes hold of on the float at `r`, all in the
/// same px, with `header` px of header from the top: the edge band
/// ([`BAND`] scaled, inside the border) first, then the header, else the
/// body. `None` when the press is off the float.
pub(crate) fn grab_at(r: Rect, header: f32, scale: f32, x: f32, y: f32) -> Option<Grab> {
    let (left, top) = (r.col as f32, r.row as f32);
    let (right, bottom) = (left + r.cols as f32, top + r.rows as f32);
    if x < left || x >= right || y < top || y >= bottom {
        return None;
    }
    let band = BAND * scale;
    let edges = Edges {
        left: x < left + band,
        right: x >= right - band,
        top: y < top + band,
        bottom: y >= bottom - band,
    };
    if edges != Edges::default() {
        return Some(Grab::Resize(edges));
    }
    Some(match y < top + header {
        true => Grab::Move,
        false => Grab::Body,
    })
}

/// The float `start` after a drag of `(dx, dy)` px that took hold as `grab`,
/// kept inside `area` and at least [`MIN`] percent of it each way. A resize
/// moves only the sides grabbed.
pub(crate) fn dragged(start: Rect, grab: Grab, (dx, dy): (isize, isize), area: Rect) -> Rect {
    match grab {
        Grab::Body => start,
        Grab::Move => {
            let col = shift(
                start.col,
                dx,
                area.col,
                area.cols.saturating_sub(start.cols),
            );
            let row = shift(
                start.row,
                dy,
                area.row,
                area.rows.saturating_sub(start.rows),
            );
            Rect::new(col, row, start.cols, start.rows)
        }
        Grab::Resize(e) => {
            let least = |total: usize| share(total, MIN).max(1);
            let (col, cols) = stretch(
                (start.col, start.cols),
                (e.left, e.right),
                dx,
                (area.col, area.cols),
                least(area.cols),
            );
            let (row, rows) = stretch(
                (start.row, start.rows),
                (e.top, e.bottom),
                dy,
                (area.row, area.rows),
                least(area.rows),
            );
            Rect::new(col, row, cols, rows)
        }
    }
}

/// `at` moved by `d`, kept in `from..=from + room`.
fn shift(at: usize, d: isize, from: usize, room: usize) -> usize {
    at.saturating_add_signed(d).clamp(from, from + room)
}

/// The span `(at, len)` with its low side (`sides.0`) and/or its high side
/// (`sides.1`) pulled by `d`, inside the `total` span `(from, len)` and at
/// least `least` long.
fn stretch(
    (at, len): (usize, usize),
    sides: (bool, bool),
    d: isize,
    (from, total): (usize, usize),
    least: usize,
) -> (usize, usize) {
    let end = at + len;
    let limit = from + total;
    let low = match sides.0 {
        true => shift(at, d, from, end.saturating_sub(least).saturating_sub(from)),
        false => at,
    };
    let high = match sides.1 {
        true => end
            .saturating_add_signed(d)
            .clamp(low + least, limit.max(low + least)),
        false => end,
    };
    (low, high - low)
}

/// `at` moved one step toward `dir`; `None` at the body's edge.
pub(crate) fn moved(at: FloatAt, dir: Dir4) -> Option<FloatAt> {
    let FloatAt { x, y, w, h } = at;
    let (x, y) = match dir {
        Dir4::Left => (x.checked_sub(STEP)?, y),
        Dir4::Up => (x, y.checked_sub(STEP)?),
        Dir4::Right => (x.checked_add(STEP).filter(|x| *x + w <= 100)?, y),
        Dir4::Down => (x, y.checked_add(STEP).filter(|y| *y + h <= 100)?),
    };
    Some(FloatAt { x, y, w, h })
}

/// `at` one step wider (`Right`), narrower (`Left`), taller (`Down`) or
/// shorter (`Up`); `None` at the body's edge or the least size.
pub(crate) fn resized(at: FloatAt, dir: Dir4) -> Option<FloatAt> {
    let FloatAt { x, y, w, h } = at;
    let (w, h) = match dir {
        Dir4::Left => (w.checked_sub(STEP).filter(|w| *w >= MIN)?, h),
        Dir4::Up => (w, h.checked_sub(STEP).filter(|h| *h >= MIN)?),
        Dir4::Right => (w.checked_add(STEP).filter(|w| x + *w <= 100)?, h),
        Dir4::Down => (w, h.checked_add(STEP).filter(|h| y + *h <= 100)?),
    };
    Some(FloatAt { x, y, w, h })
}

/// A step's way: [`Dir`] says a split's axis, a float moves four ways.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dir4 {
    Left,
    Down,
    Up,
    Right,
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        col: 10,
        row: 50,
        cols: 1000,
        rows: 600,
    };

    fn at(x: u8, y: u8, w: u8, h: u8) -> FloatAt {
        FloatAt { x, y, w, h }
    }

    #[test]
    fn a_place_scales_to_the_body_and_back() {
        let r = to_px(at(20, 10, 60, 50), AREA);
        assert_eq!(r, Rect::new(210, 110, 600, 300));
        assert_eq!(to_percent(r, AREA), at(20, 10, 60, 50));
    }

    #[test]
    fn a_place_never_leaves_the_body_or_vanishes() {
        let r = to_px(at(90, 90, 10, 10), AREA);
        assert!(r.col + r.cols <= AREA.col + AREA.cols);
        assert!(r.row + r.rows <= AREA.row + AREA.rows);
        let tiny = Rect::new(0, 0, 3, 3);
        let r = to_px(at(95, 95, 10, 10), tiny);
        assert!(r.cols >= 1 && r.rows >= 1);
        assert_eq!(to_px(at(0, 0, 10, 10), Rect::new(0, 0, 0, 0)).cols, 1);
    }

    #[test]
    fn a_rect_becomes_percent_at_the_least_size_inside_the_body() {
        let sliver = Rect::new(AREA.col, AREA.row, 5, 5);
        assert_eq!(to_percent(sliver, AREA), at(0, 0, MIN, MIN));
        let past = Rect::new(AREA.col + 900, AREA.row + 500, 400, 400);
        let p = to_percent(past, AREA);
        assert!(u16::from(p.x) + u16::from(p.w) <= 100);
        assert!(u16::from(p.y) + u16::from(p.h) <= 100);
        let before = Rect::new(0, 0, 500, 300);
        assert_eq!(
            (to_percent(before, AREA).x, to_percent(before, AREA).y),
            (0, 0)
        );
    }

    #[test]
    fn a_press_takes_the_edge_band_before_the_header() {
        let r = Rect::new(100, 100, 400, 300);
        let (header, scale) = (38.0, 1.0);
        let grab = |x, y| grab_at(r, header, scale, x, y);
        assert_eq!(grab(300.0, 120.0), Some(Grab::Move));
        assert_eq!(grab(300.0, 250.0), Some(Grab::Body));
        let left = Edges {
            left: true,
            ..Edges::default()
        };
        assert_eq!(grab(102.0, 250.0), Some(Grab::Resize(left)));
        // The top edge is in the header's rows but is a resize, not a move.
        let top = Edges {
            top: true,
            ..Edges::default()
        };
        assert_eq!(grab(300.0, 103.0), Some(Grab::Resize(top)));
        let corner = Edges {
            right: true,
            bottom: true,
            ..Edges::default()
        };
        assert_eq!(grab(498.0, 398.0), Some(Grab::Resize(corner)));
        // 6 px at 1x: the 7th is the body.
        assert_eq!(grab(107.0, 250.0), Some(Grab::Body));
        assert_eq!(grab(99.0, 250.0), None);
        assert_eq!(grab(500.0, 250.0), None);
    }

    #[test]
    fn the_band_grows_with_the_scale() {
        let r = Rect::new(0, 0, 400, 300);
        assert_eq!(
            grab_at(r, 76.0, 2.0, 10.0, 150.0),
            Some(Grab::Resize(Edges {
                left: true,
                ..Edges::default()
            }))
        );
        assert_eq!(grab_at(r, 76.0, 2.0, 13.0, 150.0), Some(Grab::Body));
    }

    #[test]
    fn a_move_drag_keeps_the_float_inside_the_body() {
        let start = Rect::new(210, 110, 600, 300);
        assert_eq!(
            dragged(start, Grab::Move, (40, -20), AREA),
            Rect::new(250, 90, 600, 300)
        );
        assert_eq!(
            dragged(start, Grab::Move, (-5000, -5000), AREA),
            Rect::new(AREA.col, AREA.row, 600, 300)
        );
        assert_eq!(
            dragged(start, Grab::Move, (5000, 5000), AREA),
            Rect::new(AREA.col + 400, AREA.row + 300, 600, 300)
        );
        assert_eq!(dragged(start, Grab::Body, (9, 9), AREA), start);
    }

    #[test]
    fn a_resize_drag_moves_only_the_sides_grabbed() {
        let start = Rect::new(210, 110, 600, 300);
        let right_bottom = Grab::Resize(Edges {
            right: true,
            bottom: true,
            ..Edges::default()
        });
        assert_eq!(
            dragged(start, right_bottom, (50, 25), AREA),
            Rect::new(210, 110, 650, 325)
        );
        let top_left = Grab::Resize(Edges {
            left: true,
            top: true,
            ..Edges::default()
        });
        assert_eq!(
            dragged(start, top_left, (-30, -10), AREA),
            Rect::new(180, 100, 630, 310)
        );
    }

    #[test]
    fn a_resize_drag_stops_at_the_least_size_and_the_body() {
        let start = Rect::new(210, 110, 600, 300);
        let right = Grab::Resize(Edges {
            right: true,
            ..Edges::default()
        });
        // 10% of 1000 px.
        assert_eq!(dragged(start, right, (-5000, 0), AREA).cols, 100);
        assert_eq!(
            dragged(start, right, (5000, 0), AREA).cols,
            AREA.col + AREA.cols - 210
        );
        let left = Grab::Resize(Edges {
            left: true,
            ..Edges::default()
        });
        let r = dragged(start, left, (5000, 0), AREA);
        assert_eq!((r.col, r.cols), (710, 100));
        assert_eq!(dragged(start, left, (-5000, 0), AREA).col, AREA.col);
    }

    #[test]
    fn a_key_moves_a_float_a_step_and_stops_at_the_edges() {
        assert_eq!(
            moved(at(20, 20, 60, 60), Dir4::Left),
            Some(at(15, 20, 60, 60))
        );
        assert_eq!(
            moved(at(20, 20, 60, 60), Dir4::Down),
            Some(at(20, 25, 60, 60))
        );
        assert_eq!(moved(at(0, 0, 60, 60), Dir4::Left), None);
        assert_eq!(moved(at(0, 0, 60, 60), Dir4::Up), None);
        assert_eq!(moved(at(40, 40, 60, 60), Dir4::Right), None);
        assert_eq!(moved(at(40, 40, 60, 60), Dir4::Down), None);
    }

    #[test]
    fn a_key_resizes_within_the_body_and_the_least_size() {
        assert_eq!(
            resized(at(20, 20, 60, 60), Dir4::Right),
            Some(at(20, 20, 65, 60))
        );
        assert_eq!(
            resized(at(20, 20, 60, 60), Dir4::Up),
            Some(at(20, 20, 60, 55))
        );
        assert_eq!(resized(at(50, 0, 50, 50), Dir4::Right), None);
        assert_eq!(resized(at(0, 50, 50, 50), Dir4::Down), None);
        assert_eq!(resized(at(0, 0, 10, 50), Dir4::Left), None);
        assert_eq!(resized(at(0, 0, 50, 10), Dir4::Up), None);
    }
}
