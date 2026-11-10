//! Split-pane layout for a tab: a binary tree of panes tiled over the tab's
//! cell area.
//!
//! Each leaf is a pane id; each split divides its area in two (with a one-cell
//! divider between) either side-by-side ([`Dir::Vertical`]) or stacked
//! ([`Dir::Horizontal`]). The tree is pure geometry over ids, so the window's
//! pane lifecycle (one shell per id) stays separate and this is unit-testable
//! without a GUI.

use super::source::Tree;

/// Which way a split divides its area.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    /// A horizontal divider: the two children stack (top / bottom).
    Horizontal,
    /// A vertical divider: the two children sit side by side (left / right).
    Vertical,
}

/// A rectangle in terminal cells: top-left `(col, row)` and size `cols × rows`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    pub col: usize,
    pub row: usize,
    pub cols: usize,
    pub rows: usize,
}

impl Rect {
    pub fn new(col: usize, row: usize, cols: usize, rows: usize) -> Self {
        Self {
            col,
            row,
            cols,
            rows,
        }
    }

    pub(crate) fn contains(&self, col: usize, row: usize) -> bool {
        col >= self.col
            && col < self.col + self.cols
            && row >= self.row
            && row < self.row + self.rows
    }
}

enum Node {
    Leaf(u64),
    Split {
        dir: Dir,
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

impl Node {
    fn from_tree(tree: &Tree) -> Self {
        match tree {
            Tree::Pane(id) => Node::Leaf(*id),
            Tree::Split { dir, ratio, a, b } => Node::Split {
                dir: *dir,
                ratio: ratio.clamp(0.1, 0.9),
                a: Box::new(Node::from_tree(a)),
                b: Box::new(Node::from_tree(b)),
            },
        }
    }

    fn collect(&self, out: &mut Vec<u64>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
        }
    }

    /// Replace the leaf for `target` with a split of `[target, new]`.
    #[cfg(test)]
    fn split(&mut self, target: u64, new: u64, dir: Dir) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                *self = Node::Split {
                    dir,
                    ratio: 0.5,
                    a: Box::new(Node::Leaf(target)),
                    b: Box::new(Node::Leaf(new)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, new, dir) || b.split(target, new, dir),
        }
    }

    fn rects(&self, area: Rect, gap: usize, out: &mut Vec<(u64, Rect)>) {
        match self {
            Node::Leaf(id) => out.push((*id, area)),
            Node::Split { dir, ratio, a, b } => {
                let (ra, rb) = split_area(area, *dir, *ratio, gap);
                a.rects(ra, gap, out);
                b.rects(rb, gap, out);
            }
        }
    }
}

/// Split `area` into two sub-areas for a [`Dir`] at `ratio`, reserving `gap`
/// units between them for the divider. Degenerates gracefully when the area
/// is too small to divide (the first child takes it all).
fn split_area(area: Rect, dir: Dir, ratio: f32, gap: usize) -> (Rect, Rect) {
    match dir {
        Dir::Vertical => {
            let Some((a, b)) = divide(area.cols, ratio, gap) else {
                return (area, Rect::new(area.col, area.row, 0, area.rows));
            };
            (
                Rect::new(area.col, area.row, a, area.rows),
                Rect::new(area.col + a + gap, area.row, b, area.rows),
            )
        }
        Dir::Horizontal => {
            let Some((a, b)) = divide(area.rows, ratio, gap) else {
                return (area, Rect::new(area.col, area.row, area.cols, 0));
            };
            (
                Rect::new(area.col, area.row, area.cols, a),
                Rect::new(area.col, area.row + a + gap, area.cols, b),
            )
        }
    }
}

/// `len` less `gap`, split at `ratio` with at least one unit each side;
/// `None` when that doesn't fit.
fn divide(len: usize, ratio: f32, gap: usize) -> Option<(usize, usize)> {
    let usable = len.checked_sub(gap).filter(|&u| u >= 2)?;
    let a = ((usable as f32 * ratio).round() as usize).clamp(1, usable - 1);
    Some((a, usable - a))
}

/// A tab's pane layout.
pub struct Layout {
    root: Node,
}

impl Layout {
    /// The layout a session source states (see [`Tree`]).
    pub(crate) fn from_tree(tree: &Tree) -> Self {
        Self {
            root: Node::from_tree(tree),
        }
    }

    /// A layout with a single full-area pane. Tests build layouts with it;
    /// windows take theirs from the source (D159).
    #[cfg(test)]
    pub fn single(id: u64) -> Self {
        Self {
            root: Node::Leaf(id),
        }
    }

    /// Split the `target` pane, giving its new sibling `new`. No-op (returns
    /// `false`) if `target` isn't present.
    #[cfg(test)]
    pub fn split(&mut self, target: u64, new: u64, dir: Dir) -> bool {
        self.root.split(target, new, dir)
    }

    /// Every pane and its rectangle within `area`, in tree (left-to-right,
    /// top-to-bottom) order.
    #[cfg(test)]
    pub fn rects(&self, area: Rect) -> Vec<(u64, Rect)> {
        self.boxes(area, 1)
    }

    /// The same tiling in any unit, such as pixels, with `gap` units
    /// between neighbours.
    pub(crate) fn boxes(&self, area: Rect, gap: usize) -> Vec<(u64, Rect)> {
        let mut out = Vec::new();
        self.root.rects(area, gap, &mut out);
        out
    }

    /// All pane ids in tree order.
    pub fn ids(&self) -> Vec<u64> {
        let mut out = Vec::new();
        self.root.collect(&mut out);
        out
    }

    /// The pane after (`forward`) or before `current` in tree order, wrapping.
    /// Falls back to the first pane if `current` isn't found.
    pub fn cycle(&self, current: u64, forward: bool) -> u64 {
        let ids = self.ids();
        let n = ids.len();
        match ids.iter().position(|&id| id == current) {
            Some(i) if forward => ids[(i + 1) % n],
            Some(i) => ids[(i + n - 1) % n],
            None => ids[0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_pane_fills_the_area() {
        let l = Layout::single(1);
        let area = Rect::new(0, 1, 80, 24);
        assert_eq!(l.rects(area), vec![(1, area)]);
        assert_eq!(l.ids(), vec![1]);
    }

    #[test]
    fn vertical_split_tiles_side_by_side_with_a_divider() {
        let mut l = Layout::single(1);
        assert!(l.split(1, 2, Dir::Vertical));
        let rects = l.rects(Rect::new(0, 0, 81, 24));
        // 81 cols: 1 divider, 80 usable -> 40 / 40, divider at col 40.
        assert_eq!(
            rects,
            vec![(1, Rect::new(0, 0, 40, 24)), (2, Rect::new(41, 0, 40, 24))]
        );
    }

    #[test]
    fn horizontal_split_stacks_with_a_divider() {
        let mut l = Layout::single(1);
        assert!(l.split(1, 2, Dir::Horizontal));
        let rects = l.rects(Rect::new(0, 0, 80, 25));
        // 25 rows: 1 divider, 24 usable -> 12 / 12, divider at row 12.
        assert_eq!(
            rects,
            vec![(1, Rect::new(0, 0, 80, 12)), (2, Rect::new(0, 13, 80, 12))]
        );
    }

    #[test]
    fn nested_splits_subdivide_the_right_pane() {
        let mut l = Layout::single(1);
        l.split(1, 2, Dir::Vertical); // [1 | 2]
        l.split(2, 3, Dir::Horizontal); // 2 becomes [2 / 3]
        assert_eq!(l.ids(), vec![1, 2, 3]);
        let rects = l.rects(Rect::new(0, 0, 81, 25));
        // Left half is pane 1 full height; right half splits 2 over 3.
        assert_eq!(rects[0], (1, Rect::new(0, 0, 40, 25)));
        assert_eq!(rects[1].0, 2);
        assert_eq!(rects[2].0, 3);
        assert_eq!(rects[1].1.col, 41);
        assert_eq!(rects[2].1.col, 41);
        assert!(rects[2].1.row > rects[1].1.row); // 3 is below 2
    }

    #[test]
    fn pane_at_and_cycle() {
        let mut l = Layout::single(1);
        l.split(1, 2, Dir::Vertical);
        let area = Rect::new(0, 0, 81, 24);
        // Hit-testing goes through rects + Rect::contains (the window's
        // zoom-aware Tab::rects wraps the same primitive).
        let at = |l: &Layout, col: usize, row: usize| {
            l.rects(area)
                .into_iter()
                .find(|(_, r)| r.contains(col, row))
                .map(|(id, _)| id)
        };
        assert_eq!(at(&l, 0, 0), Some(1));
        assert_eq!(at(&l, 41, 0), Some(2));
        assert_eq!(at(&l, 40, 0), None); // the divider column
        assert_eq!(l.cycle(1, true), 2);
        assert_eq!(l.cycle(2, true), 1); // wraps
        assert_eq!(l.cycle(1, false), 2); // wraps backward
    }

    #[test]
    fn boxes_leave_the_gap_between_neighbours() {
        let mut l = Layout::single(1);
        l.split(1, 2, Dir::Vertical);
        l.split(2, 3, Dir::Horizontal);
        let boxes = l.boxes(Rect::new(8, 52, 808, 408), 8);
        assert_eq!(
            boxes,
            vec![
                (1, Rect::new(8, 52, 400, 408)),
                (2, Rect::new(416, 52, 400, 200)),
                (3, Rect::new(416, 260, 400, 200)),
            ]
        );
        let tiny = l.boxes(Rect::new(0, 0, 9, 9), 8);
        assert_eq!(tiny[0], (1, Rect::new(0, 0, 9, 9)), "too small to split");
    }

    #[test]
    fn a_stated_tree_tiles_as_the_same_splits_would() {
        let tree = Tree::Split {
            dir: Dir::Vertical,
            ratio: 0.5,
            a: Box::new(Tree::Pane(1)),
            b: Box::new(Tree::Split {
                dir: Dir::Horizontal,
                ratio: 0.5,
                a: Box::new(Tree::Pane(2)),
                b: Box::new(Tree::Pane(3)),
            }),
        };
        let mut built = Layout::single(1);
        built.split(1, 2, Dir::Vertical);
        built.split(2, 3, Dir::Horizontal);
        let area = Rect::new(0, 0, 81, 25);
        assert_eq!(Layout::from_tree(&tree).rects(area), built.rects(area));
    }
}
