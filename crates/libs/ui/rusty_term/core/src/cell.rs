//! Cell model and character-width classification (L05).
//!
//! The [`Cell`] is the atom of the screen buffer — a base glyph plus any trailing
//! grapheme continuation (combining marks, ZWJ joins, …) and truecolor
//! attributes — and [`char_width`] classifies a code point's display width per
//! Unicode UAX #11.

use unicode_width::UnicodeWidthChar;

use crate::style::{CellStyle, StyleId};

/// Default foreground color (white) used on reset and for blank cells.
pub const DEFAULT_FG: u32 = 0xFFFFFF;
/// Default background color (black) used on reset and for blank cells.
pub const DEFAULT_BG: u32 = 0x000000;

/// [`Cell::flags`] bit marking the trailing (second) cell of a double-width
/// character. The renderer skips these so the wide glyph occupies two columns.
///
/// This is a *layout* flag (bit 0); the `ATTR_*` rendition bits live above it,
/// so a cell's `flags` is `WIDE_TRAILER | <pen attributes>`.
pub const WIDE_TRAILER: u16 = 1 << 0;

// SGR text-attribute bits, stored in [`Cell::flags`] and carried by [`Pen`].
// They occupy bits 1.. so they never collide with [`WIDE_TRAILER`].
/// SGR 1 — bold / increased intensity.
pub const ATTR_BOLD: u16 = 1 << 1;
/// SGR 2 — dim / decreased intensity.
pub const ATTR_DIM: u16 = 1 << 2;
/// SGR 3 — italic.
pub const ATTR_ITALIC: u16 = 1 << 3;
/// SGR 4 — underline.
pub const ATTR_UNDERLINE: u16 = 1 << 4;
/// SGR 5 — blink.
pub const ATTR_BLINK: u16 = 1 << 5;
/// SGR 7 — reverse video (swap fg/bg).
pub const ATTR_REVERSE: u16 = 1 << 6;
/// SGR 8 — concealed / hidden.
pub const ATTR_HIDDEN: u16 = 1 << 7;
/// SGR 9 — crossed-out / strikethrough.
pub const ATTR_STRIKE: u16 = 1 << 8;
/// SGR 58 — a custom underline color is set (in [`Cell::underline_color`] /
/// [`Pen::underline_color`]) rather than following the foreground.
pub const ATTR_UNDERLINE_COLOR: u16 = 1 << 12;
/// DECSCA (`CSI 1 " q`) — the cell is protected from selective erase
/// (DECSED/DECSEL). Deliberately *not* an SGR rendition: SGR 0 must not
/// clear it, so the parser tracks it separately from the pen's SGR state
/// and stamps it onto cells at write time.
pub const ATTR_PROTECTED: u16 = 1 << 13;

/// Bit offset of the 3-bit underline-style sub-field (SGR `4:0`-`4:5`) packed
/// into the same `u16` as the `ATTR_*` bits. Only meaningful when
/// [`ATTR_UNDERLINE`] is set.
const UNDERLINE_STYLE_SHIFT: u16 = 9;
/// Mask isolating the underline-style sub-field before shifting.
const UNDERLINE_STYLE_MASK: u16 = 0b111 << UNDERLINE_STYLE_SHIFT;

/// Mask of every rendition attribute bit (everything except [`WIDE_TRAILER`]).
pub const ATTR_MASK: u16 = ATTR_BOLD
    | ATTR_DIM
    | ATTR_ITALIC
    | ATTR_UNDERLINE
    | ATTR_BLINK
    | ATTR_REVERSE
    | ATTR_HIDDEN
    | ATTR_STRIKE
    | ATTR_UNDERLINE_COLOR
    | UNDERLINE_STYLE_MASK;

/// Underline stroke style (SGR `4:0`.."4:5"` — the colon sub-parameter form
/// Kitty and others use to pick something other than a plain straight line).
/// Packed into 3 bits of [`Cell::flags`] / [`Pen::attrs`]; only meaningful
/// while [`ATTR_UNDERLINE`] is also set — clearing the attribute (SGR `24`)
/// hides the stroke regardless of what style bits are left behind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnderlineStyle {
    #[default]
    Straight,
    Double,
    Curly,
    Dotted,
    Dashed,
}

impl UnderlineStyle {
    /// Decode the style packed into `attrs` (a `Cell::flags` or `Pen::attrs`
    /// value). An out-of-range sub-field (shouldn't happen — only this module
    /// writes it) falls back to [`UnderlineStyle::Straight`].
    pub fn from_attrs(attrs: u16) -> Self {
        match (attrs & UNDERLINE_STYLE_MASK) >> UNDERLINE_STYLE_SHIFT {
            1 => UnderlineStyle::Double,
            2 => UnderlineStyle::Curly,
            3 => UnderlineStyle::Dotted,
            4 => UnderlineStyle::Dashed,
            _ => UnderlineStyle::Straight,
        }
    }

    /// Pack `self` into the style sub-field of an attribute bitset, replacing
    /// whatever was there (the other `ATTR_*` bits of `attrs` pass through
    /// untouched).
    pub fn pack_into(self, attrs: u16) -> u16 {
        let bits = match self {
            UnderlineStyle::Straight => 0,
            UnderlineStyle::Double => 1,
            UnderlineStyle::Curly => 2,
            UnderlineStyle::Dotted => 3,
            UnderlineStyle::Dashed => 4,
        };
        (attrs & !UNDERLINE_STYLE_MASK) | (bits << UNDERLINE_STYLE_SHIFT)
    }
}

/// The current SGR graphic rendition — foreground/background color and the set
/// of active text attributes — that the parser stamps onto each glyph it
/// writes. A single source of truth for "how the next character looks".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pen {
    /// Foreground color as `0xRRGGBB`.
    pub fg: u32,
    /// Background color as `0xRRGGBB`.
    pub bg: u32,
    /// Active text-attribute bits (`ATTR_*`).
    pub attrs: u16,
    /// Underline color as `0xRRGGBB` (SGR 58). Only consulted when
    /// [`ATTR_UNDERLINE_COLOR`] is set; otherwise the underline follows `fg`.
    pub underline_color: u32,
}

impl Default for Pen {
    /// The reset pen: default colors and no attributes.
    fn default() -> Self {
        Pen {
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            attrs: 0,
            underline_color: DEFAULT_FG,
        }
    }
}

/// Display width of `ch` in terminal cells: `0` for zero-width (combining
/// marks, joiners, variation selectors, …), `2` for wide East Asian / emoji
/// code points, and `1` otherwise.
///
/// Backed by the [`unicode-width`] crate, which implements the full Unicode
/// East Asian Width (UAX #11) and emoji-presentation property tables. Control
/// characters (for which the crate reports no width) collapse to `0`; the
/// parser handles C0/C1 controls before they ever reach this function.
/// Regional indicators are overridden to `2` (see the body).
///
/// [`unicode-width`]: https://docs.rs/unicode-width
pub fn char_width(ch: char) -> usize {
    // Regional indicators (the halves of a flag) are East Asian Width
    // Neutral, so the crate says 1, but vim and Ghostty draw each in two
    // columns. Matching them keeps vim's cursor and ours in step (glibc's
    // wcwidth says 1; the apps that draw flags disagree with it).
    if ('\u{1f1e6}'..='\u{1f1ff}').contains(&ch) {
        return 2;
    }
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

/// A single character cell: its base glyph, an optional grapheme-cluster id for
/// glyphs that span more than one code point (combining marks, ZWJ emoji
/// sequences, …), and truecolor attributes. Kept `Copy` so the grid can shift
/// cells with `copy_within`.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    /// The base (first) scalar of the grapheme. Used for width classification
    /// and as the whole glyph when [`cluster`](Self::cluster) is `0`.
    pub ch: char,
    /// Grapheme-continuation id: `0` for a lone `ch`, else an index+1 into
    /// `Grid::clusters`, whose string is the continuation appended after `ch`
    /// (combining marks, ZWJ-joined scalars, variation selectors, …).
    pub cluster: u16,
    /// Foreground color as `0xRRGGBB`.
    pub fg: u32,
    /// Background color as `0xRRGGBB`.
    pub bg: u32,
    /// Attribute bitset (bold, italic, …). Reserved for future use.
    pub flags: u16,
    /// Hyperlink id (OSC 8): `0` for none, else an index+1 into `Grid::links`.
    pub link: u16,
    /// Underline color as `0xRRGGBB` (SGR 58). Only meaningful when
    /// [`ATTR_UNDERLINE_COLOR`] is set in `flags`.
    pub underline_color: u32,
}

impl Cell {
    /// Construct a blank cell (space glyph, default colors).
    pub fn blank() -> Self {
        Cell {
            ch: ' ',
            cluster: 0,
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            flags: 0,
            link: 0,
            underline_color: DEFAULT_FG,
        }
    }
}

/// [`PackedCell`] text bit: the cell is a wide glyph's trailer.
const TEXT_WIDE: u32 = 1 << 21;
/// [`PackedCell`] text bit: the cell is DECSCA-protected.
pub(crate) const TEXT_PROTECTED: u32 = 1 << 22;
/// The Unicode scalar bits of [`PackedCell`]'s text word.
const CHAR_BITS: u32 = 0x001F_FFFF;

/// A cell as the grid stores it: 8 bytes, the character and two layout bits
/// inline, everything else behind a [`StyleId`] into the grid's style table.
/// [`Cell`] is the resolved value readers see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PackedCell {
    /// Bits 0..=20 the scalar, then [`TEXT_WIDE`] and [`TEXT_PROTECTED`];
    /// the rest always zero.
    text: u32,
    style: StyleId,
}

impl PackedCell {
    /// A blank cell under the default style id: valid only in a fresh table,
    /// whose id 0 is still the blank style.
    pub(crate) fn blank() -> Self {
        PackedCell {
            text: u32::from(' '),
            style: StyleId::DEFAULT,
        }
    }

    /// A glyph `ch` under `style`, DECSCA-protected or not.
    pub(crate) fn glyph(ch: char, protected: bool, style: StyleId) -> Self {
        let text = u32::from(ch) | if protected { TEXT_PROTECTED } else { 0 };
        PackedCell { text, style }
    }

    /// A wide glyph's trailer (a space) under `style`.
    pub(crate) fn trailer(style: StyleId) -> Self {
        PackedCell {
            text: u32::from(' ') | TEXT_WIDE,
            style,
        }
    }

    /// Whether this cell is DECSCA-protected.
    pub(crate) fn is_protected(self) -> bool {
        self.text & TEXT_PROTECTED != 0
    }

    /// `cell` stored under `style`, which must be `cell`'s interned style.
    pub(crate) fn new(cell: &Cell, style: StyleId) -> Self {
        let mut text = u32::from(cell.ch);
        if cell.flags & WIDE_TRAILER != 0 {
            text |= TEXT_WIDE;
        }
        if cell.flags & ATTR_PROTECTED != 0 {
            text |= TEXT_PROTECTED;
        }
        PackedCell { text, style }
    }

    /// The base character.
    pub(crate) fn ch(self) -> char {
        char::from_u32(self.text & CHAR_BITS).unwrap_or(' ')
    }

    /// Whether this is a wide glyph's trailer.
    pub(crate) fn is_wide_trailer(self) -> bool {
        self.text & TEXT_WIDE != 0
    }

    /// The layout bits ([`TEXT_WIDE`], [`TEXT_PROTECTED`]).
    pub(crate) fn layout(self) -> u32 {
        self.text & !CHAR_BITS
    }

    /// The layout bits as a small number, below 4 for the known bits.
    pub(crate) fn layout_index(self) -> usize {
        (self.text >> CHAR_BITS.count_ones()) as usize
    }

    pub(crate) fn style(self) -> StyleId {
        self.style
    }

    /// This cell with another style id (a remap).
    pub(crate) fn with_style(self, style: StyleId) -> Self {
        PackedCell { style, ..self }
    }

    /// Same in everything but the character: one run style on the RTCP wire.
    pub(crate) fn same_run(self, other: PackedCell) -> bool {
        self.style == other.style && self.layout() == other.layout()
    }

    /// The full cell, given this cell's style.
    pub(crate) fn resolve(self, style: &CellStyle) -> Cell {
        let mut flags = style.attrs;
        if self.text & TEXT_WIDE != 0 {
            flags |= WIDE_TRAILER;
        }
        if self.text & TEXT_PROTECTED != 0 {
            flags |= ATTR_PROTECTED;
        }
        Cell {
            ch: self.ch(),
            cluster: style.cluster,
            fg: style.fg,
            bg: style.bg,
            flags,
            link: style.link,
            underline_color: style.underline_color,
        }
    }
}

crate::checkpoint::fields!(Pen, [fg, bg, attrs, underline_color]);
crate::checkpoint::fields!(Cell, [ch, fg, bg, flags, link, cluster, underline_color]);
