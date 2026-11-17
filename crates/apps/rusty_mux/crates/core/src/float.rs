//! Floating layers (C42): blocks drawn over a window's tiled layout. A
//! float is out of the layout, so a window's blocks are tiled, floating
//! or detached, never two at once. The server keeps the floats back to
//! front; clients draw them in that order, each at its own size.

use std::fmt;
use std::str::FromStr;

use crate::{BlockId, Error, ParseError, Rect, Size};

/// The least a float's width or height may be, in percent of the window,
/// so a float stays big enough to grab.
pub const MIN_FLOAT: u8 = 10;

/// A float's place, in whole percent of its window's area from the top
/// left: every client scales it to its own cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloatRect {
    x: u8,
    y: u8,
    w: u8,
    h: u8,
}

impl FloatRect {
    /// Checks that the rect is at least [`MIN_FLOAT`] wide and tall and
    /// lies inside the window.
    pub fn new(x: u8, y: u8, w: u8, h: u8) -> Result<Self, Error> {
        let fits = |at: u8, len: u8| len >= MIN_FLOAT && u16::from(at) + u16::from(len) <= 100;
        if !fits(x, w) || !fits(y, h) {
            return Err(Error::BadFloat { x, y, w, h });
        }
        Ok(Self { x, y, w, h })
    }

    pub const fn x(self) -> u8 {
        self.x
    }

    pub const fn y(self) -> u8 {
        self.y
    }

    pub const fn w(self) -> u8 {
        self.w
    }

    pub const fn h(self) -> u8 {
        self.h
    }

    /// The rect on a window drawn `area` cells large; never empty and
    /// never past the window's edge.
    pub fn cells(self, area: Size) -> Rect {
        let (x, cols) = span(self.x, self.w, area.cols());
        let (y, rows) = span(self.y, self.h, area.rows());
        Rect {
            x,
            y,
            size: Size { cols, rows },
        }
    }
}

/// `at` and `len` percent of `total` cells, as a start and a length of at
/// least one cell that ends inside `total`.
fn span(at: u8, len: u8, total: u16) -> (u16, u16) {
    let part = |p: u8| u16::try_from(u32::from(total) * u32::from(p) / 100).unwrap_or(total);
    let start = part(at).min(total - 1);
    let len = part(len).clamp(1, total - start);
    (start, len)
}

/// One floating block and where it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Float {
    pub block: BlockId,
    pub at: FloatRect,
}

/// `block@x,y,w,h`: a block id, then its [`FloatRect`] in percent.
impl fmt::Display for Float {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = self.at;
        write!(f, "{}@{},{},{},{}", self.block, at.x, at.y, at.w, at.h)
    }
}

impl FromStr for Float {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (id, rect) = text.split_once('@').ok_or(ParseError)?;
        let block = BlockId::from_uuid(id.parse().map_err(|_| ParseError)?);
        let mut parts = rect
            .split(',')
            .map(|n| n.parse::<u8>().map_err(|_| ParseError));
        let mut next = || parts.next().unwrap_or(Err(ParseError));
        let (x, y, w, h) = (next()?, next()?, next()?, next()?);
        if parts.next().is_some() {
            return Err(ParseError);
        }
        let at = FloatRect::new(x, y, w, h).map_err(|_| ParseError)?;
        Ok(Self { block, at })
    }
}

/// Floats as the wire carries them: each in its text form, joined by `;`,
/// back to front; empty for none.
pub fn floats_text(floats: &[Float]) -> String {
    let texts: Vec<String> = floats.iter().map(Float::to_string).collect();
    texts.join(";")
}

/// Floats from [`floats_text`]; refuses a block listed twice.
pub fn parse_floats(text: &str) -> Result<Vec<Float>, ParseError> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let floats = text
        .split(';')
        .map(str::parse)
        .collect::<Result<Vec<Float>, _>>()?;
    let mut blocks: Vec<BlockId> = floats.iter().map(|f| f.block).collect();
    blocks.sort_unstable();
    blocks.dedup();
    if blocks.len() != floats.len() {
        return Err(ParseError);
    }
    Ok(floats)
}
