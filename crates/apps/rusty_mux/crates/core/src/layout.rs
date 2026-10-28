//! A window's layout: a tree of splits whose leaves are its blocks. The
//! server keeps it and clients draw it, each at its own size.
//!
//! The text form, used on the wire and in ctl, is a block id for a leaf,
//! or `cols` or `rows`, the first child's share in percent, then both
//! children in parentheses:
//!
//! ```
//! use rusty_mux_core::{Layout, Size};
//!
//! let text = "cols50(00000000-0000-0000-0000-000000000001,\
//!             rows50(00000000-0000-0000-0000-000000000002,\
//!             00000000-0000-0000-0000-000000000003))";
//! let layout: Layout = text.parse()?;
//! assert_eq!(layout.to_string(), text);
//! // 81 columns: 40 on the left, a divider, 40 on the right.
//! let rects = layout.rects(Size::new(81, 24)?);
//! assert_eq!(rects[1].1.x, 41);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fmt;
use std::str::FromStr;

use crate::{BlockId, Size};

/// How deep a layout may nest. It bounds the recursion of every walk, and
/// of a client parsing a layout it was sent.
pub const MAX_DEPTH: usize = 32;

/// Which way a split divides its area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Side by side: the first child on the left.
    Columns,
    /// Stacked: the first child on top.
    Rows,
}

impl Axis {
    const fn word(self) -> &'static str {
        match self {
            Self::Columns => "cols",
            Self::Rows => "rows",
        }
    }
}

/// Where a new block goes beside an existing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Up,
    Down,
}

impl Side {
    pub const fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Columns,
            Self::Up | Self::Down => Axis::Rows,
        }
    }

    /// Whether the new block is the split's first child.
    pub(crate) const fn first(self) -> bool {
        matches!(self, Self::Left | Self::Up)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

impl FromStr for Side {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        [Self::Left, Self::Right, Self::Up, Self::Down]
            .into_iter()
            .find(|side| side.name() == text)
            .ok_or(ParseError)
    }
}

/// The first child's share of a split, a percentage from 1 to 99, so
/// neither child is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Share(u8);

impl Share {
    pub const HALF: Self = Self(50);

    pub const fn new(percent: u8) -> Option<Self> {
        match percent {
            1..=99 => Some(Self(percent)),
            _ => None,
        }
    }

    pub const fn percent(self) -> u8 {
        self.0
    }

    /// Divides `len` cells into the first child's, a one-cell divider and
    /// the second child's; `None` when there is no room for all three.
    pub fn lengths(self, len: u16) -> Option<(u16, u16)> {
        let room = len.checked_sub(1).filter(|room| *room >= 2)?;
        let first = (u32::from(room) * u32::from(self.0) + 50) / 100;
        // Both fit in u16: first is clamped below room.
        let first = (first as u16).clamp(1, room - 1);
        Some((first, room - first))
    }
}

/// A block's place on a client's screen, in cells from the top left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub size: Size,
}

/// A split tree with at least one block. Built by [`Mux`](crate::Mux), or
/// parsed from its text form, which refuses a tree deeper than
/// [`MAX_DEPTH`] or naming a block twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
    Block(BlockId),
    Split {
        axis: Axis,
        share: Share,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

impl Layout {
    /// The blocks left to right and top to bottom.
    pub fn blocks(&self) -> Vec<BlockId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<BlockId>) {
        match self {
            Self::Block(block) => out.push(*block),
            Self::Split { first, second, .. } => {
                first.collect(out);
                second.collect(out);
            }
        }
    }

    /// How many splits enclose `block`, if it is in the layout.
    pub fn depth_of(&self, block: BlockId) -> Option<usize> {
        match self {
            Self::Block(b) => (*b == block).then_some(0),
            Self::Split { first, second, .. } => first
                .depth_of(block)
                .or_else(|| second.depth_of(block))
                .map(|d| d + 1),
        }
    }

    /// Where each block goes on a screen of `area`, splits leaving a
    /// one-cell divider between their children. A block with no room left
    /// is not listed: a split too small to divide gives its first child
    /// the whole area.
    pub fn rects(&self, area: Size) -> Vec<(BlockId, Rect)> {
        let mut out = Vec::new();
        self.place(
            Rect {
                x: 0,
                y: 0,
                size: area,
            },
            &mut out,
        );
        out
    }

    fn place(&self, rect: Rect, out: &mut Vec<(BlockId, Rect)>) {
        match self {
            Self::Block(block) => out.push((*block, rect)),
            Self::Split {
                axis,
                share,
                first,
                second,
            } => match divide(rect, *axis, *share) {
                Some((r1, r2)) => {
                    first.place(r1, out);
                    second.place(r2, out);
                }
                None => first.place(rect, out),
            },
        }
    }

    /// The block with the largest share of the area, the first of equals.
    pub(crate) fn largest(&self) -> BlockId {
        self.largest_in(1.0).1
    }

    fn largest_in(&self, part: f64) -> (f64, BlockId) {
        match self {
            Self::Block(block) => (part, *block),
            Self::Split {
                share,
                first,
                second,
                ..
            } => {
                let f = f64::from(share.percent()) / 100.0;
                let a = first.largest_in(part * f);
                let b = second.largest_in(part * (1.0 - f));
                if b.0 > a.0 { b } else { a }
            }
        }
    }

    /// This layout with `target` split in half, `new` on its `side`.
    pub(crate) fn split(self, target: BlockId, new: BlockId, side: Side) -> Self {
        match self {
            Self::Block(b) if b == target => {
                let (first, second) = match side.first() {
                    true => (new, b),
                    false => (b, new),
                };
                Self::Split {
                    axis: side.axis(),
                    share: Share::HALF,
                    first: Box::new(Self::Block(first)),
                    second: Box::new(Self::Block(second)),
                }
            }
            Self::Block(b) => Self::Block(b),
            Self::Split {
                axis,
                share,
                first,
                second,
            } => Self::Split {
                axis,
                share,
                first: Box::new(first.split(target, new, side)),
                second: Box::new(second.split(target, new, side)),
            },
        }
    }

    /// This layout without `block`, its sibling taking its split's place;
    /// `None` if it was the only block.
    pub(crate) fn remove(self, block: BlockId) -> Option<Self> {
        match self {
            Self::Block(b) => (b != block).then_some(Self::Block(b)),
            Self::Split {
                axis,
                share,
                first,
                second,
            } => match (first.remove(block), second.remove(block)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    axis,
                    share,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (one, other) => one.or(other),
            },
        }
    }
}

/// `rect` divided along `axis` into the first child's part and the
/// second's, a one-cell divider between; `None` when it is too small.
fn divide(rect: Rect, axis: Axis, share: Share) -> Option<(Rect, Rect)> {
    let Size { cols, rows } = rect.size;
    Some(match axis {
        Axis::Columns => {
            let (a, b) = share.lengths(cols)?;
            (
                Rect {
                    size: Size { cols: a, rows },
                    ..rect
                },
                Rect {
                    x: rect.x + a + 1,
                    size: Size { cols: b, rows },
                    ..rect
                },
            )
        }
        Axis::Rows => {
            let (a, b) = share.lengths(rows)?;
            (
                Rect {
                    size: Size { cols, rows: a },
                    ..rect
                },
                Rect {
                    y: rect.y + a + 1,
                    size: Size { cols, rows: b },
                    ..rect
                },
            )
        }
    })
}

impl fmt::Display for Layout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Block(block) => block.fmt(f),
            Self::Split {
                axis,
                share,
                first,
                second,
            } => write!(f, "{}{}({first},{second})", axis.word(), share.percent()),
        }
    }
}

/// Text that is not a layout, or a side that is not one of the four.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError;

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a valid layout")
    }
}

impl std::error::Error for ParseError {}

impl FromStr for Layout {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut rest = text;
        let layout = parse(&mut rest, 0)?;
        if !rest.is_empty() {
            return Err(ParseError);
        }
        let mut blocks = layout.blocks();
        let count = blocks.len();
        blocks.sort_unstable();
        blocks.dedup();
        if blocks.len() != count {
            return Err(ParseError);
        }
        Ok(layout)
    }
}

/// One layout from the front of `rest`, advancing past it.
fn parse(rest: &mut &str, depth: usize) -> Result<Layout, ParseError> {
    let axis = [Axis::Columns, Axis::Rows]
        .into_iter()
        .find(|axis| rest.starts_with(axis.word()));
    let Some(axis) = axis else {
        // A UUID's text form is 36 bytes.
        let id = rest.get(..36).ok_or(ParseError)?;
        let block = BlockId::from_uuid(id.parse().map_err(|_| ParseError)?);
        *rest = &rest[36..];
        return Ok(Layout::Block(block));
    };
    if depth >= MAX_DEPTH {
        return Err(ParseError);
    }
    *rest = &rest[axis.word().len()..];
    let digits = rest.find('(').ok_or(ParseError)?;
    let percent = &rest[..digits];
    if percent.is_empty() || !percent.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ParseError);
    }
    let share = percent
        .parse()
        .ok()
        .and_then(Share::new)
        .ok_or(ParseError)?;
    *rest = &rest[digits + 1..];
    let first = parse(rest, depth + 1)?;
    *rest = rest.strip_prefix(',').ok_or(ParseError)?;
    let second = parse(rest, depth + 1)?;
    *rest = rest.strip_prefix(')').ok_or(ParseError)?;
    Ok(Layout::Split {
        axis,
        share,
        first: Box::new(first),
        second: Box::new(second),
    })
}

#[cfg(test)]
mod tests;
