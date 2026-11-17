//! Editing a float's place by keys (C42): each step moves or resizes it by
//! [`STEP`] percent, kept inside the window.

use rusty_mux_core::{FloatRect, MIN_FLOAT};

use crate::keys::Dir;

/// How far a key moves or resizes a float, in percent of the window.
pub const STEP: u8 = 5;

/// Where a block is floated at first: the middle of the window.
pub fn first() -> Option<FloatRect> {
    FloatRect::new(20, 20, 60, 60).ok()
}

/// `at` moved one step toward `dir`; `None` when it is at that edge.
pub fn moved(at: FloatRect, dir: Dir) -> Option<FloatRect> {
    let (x, y, w, h) = (at.x(), at.y(), at.w(), at.h());
    let (x, y) = match dir {
        Dir::Left => (x.checked_sub(STEP)?, y),
        Dir::Up => (x, y.checked_sub(STEP)?),
        Dir::Right => (x.checked_add(STEP).filter(|x| *x + w <= 100)?, y),
        Dir::Down => (x, y.checked_add(STEP).filter(|y| *y + h <= 100)?),
    };
    FloatRect::new(x, y, w, h).ok()
}

/// `at` one step wider (`Right`), narrower (`Left`), taller (`Down`) or
/// shorter (`Up`); `None` at the window's edge or the least size.
pub fn resized(at: FloatRect, dir: Dir) -> Option<FloatRect> {
    let (x, y, w, h) = (at.x(), at.y(), at.w(), at.h());
    let (w, h) = match dir {
        Dir::Left => (w.checked_sub(STEP).filter(|w| *w >= MIN_FLOAT)?, h),
        Dir::Up => (w, h.checked_sub(STEP).filter(|h| *h >= MIN_FLOAT)?),
        Dir::Right => (w + STEP, h),
        Dir::Down => (w, h + STEP),
    };
    FloatRect::new(x, y, w, h).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: u8, y: u8, w: u8, h: u8) -> FloatRect {
        FloatRect::new(x, y, w, h).expect("a float")
    }

    #[test]
    fn a_float_moves_a_step_and_stops_at_the_edges() {
        assert_eq!(
            moved(at(20, 20, 60, 60), Dir::Left),
            Some(at(15, 20, 60, 60))
        );
        assert_eq!(
            moved(at(20, 20, 60, 60), Dir::Down),
            Some(at(20, 25, 60, 60))
        );
        assert_eq!(moved(at(0, 0, 60, 60), Dir::Left), None);
        assert_eq!(moved(at(0, 0, 60, 60), Dir::Up), None);
        assert_eq!(moved(at(40, 40, 60, 60), Dir::Right), None);
        assert_eq!(moved(at(40, 40, 60, 60), Dir::Down), None);
    }

    #[test]
    fn a_float_resizes_within_the_window_and_the_least_size() {
        assert_eq!(
            resized(at(20, 20, 60, 60), Dir::Right),
            Some(at(20, 20, 65, 60))
        );
        assert_eq!(
            resized(at(20, 20, 60, 60), Dir::Up),
            Some(at(20, 20, 60, 55))
        );
        assert_eq!(resized(at(50, 0, 50, 50), Dir::Right), None);
        assert_eq!(resized(at(0, 50, 50, 50), Dir::Down), None);
        assert_eq!(resized(at(0, 0, 10, 50), Dir::Left), None);
        assert_eq!(resized(at(0, 0, 50, 10), Dir::Up), None);
    }
}
