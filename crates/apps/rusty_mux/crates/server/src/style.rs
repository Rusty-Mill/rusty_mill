//! A cell's style as one SGR sequence, shared by `terminal.format --vt` and
//! the attach client's renderer.

use std::fmt::Write as _;

use rusty_term_core::{
    ATTR_BLINK, ATTR_BOLD, ATTR_DIM, ATTR_HIDDEN, ATTR_ITALIC, ATTR_MASK, ATTR_REVERSE,
    ATTR_STRIKE, ATTR_UNDERLINE, ATTR_UNDERLINE_COLOR, Cell, UnderlineStyle,
};

/// What a cell's SGR sequence sets. Colours equal to the grid's defaults
/// are left to the terminal's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub fg: u32,
    pub bg: u32,
    pub attrs: u16,
    /// Only with [`ATTR_UNDERLINE_COLOR`].
    pub underline: Option<u32>,
}

impl Style {
    pub fn of(cell: &Cell) -> Self {
        let attrs = cell.flags & ATTR_MASK;
        Self {
            fg: cell.fg,
            bg: cell.bg,
            attrs,
            underline: (attrs & ATTR_UNDERLINE_COLOR != 0).then_some(cell.underline_color),
        }
    }

    /// One SGR sequence that resets, then sets this style.
    pub fn sgr(self, out: &mut String) {
        let blank = Cell::blank();
        out.push_str("\x1b[0");
        for (bit, code) in [
            (ATTR_BOLD, "1"),
            (ATTR_DIM, "2"),
            (ATTR_ITALIC, "3"),
            (ATTR_BLINK, "5"),
            (ATTR_REVERSE, "7"),
            (ATTR_HIDDEN, "8"),
            (ATTR_STRIKE, "9"),
        ] {
            if self.attrs & bit != 0 {
                out.push(';');
                out.push_str(code);
            }
        }
        if self.attrs & ATTR_UNDERLINE != 0 {
            out.push_str(match UnderlineStyle::from_attrs(self.attrs) {
                UnderlineStyle::Straight => ";4",
                UnderlineStyle::Double => ";4:2",
                UnderlineStyle::Curly => ";4:3",
                UnderlineStyle::Dotted => ";4:4",
                UnderlineStyle::Dashed => ";4:5",
            });
        }
        if self.fg != blank.fg {
            push_rgb(out, 38, self.fg);
        }
        if self.bg != blank.bg {
            push_rgb(out, 48, self.bg);
        }
        if let Some(rgb) = self.underline {
            push_rgb(out, 58, rgb);
        }
        out.push('m');
    }
}

fn push_rgb(out: &mut String, code: u8, rgb: u32) {
    let [_, r, g, b] = rgb.to_be_bytes();
    // Writing to a String cannot fail.
    let _ = write!(out, ";{code};2;{r};{g};{b}");
}
