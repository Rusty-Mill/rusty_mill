//! What a `terminal.set-theme` call changes, checked whole at the boundary.
//!
//! A colour is `#RRGGBB` or `default`, nothing else: the server turns each
//! into one OSC sequence it feeds the block's terminal, so no text from the
//! caller reaches the byte stream unchecked. Each index is set at most
//! once, so a call holds at most 256 palette entries.

use crate::schema::{CallError, Value};

/// A colour to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    /// `0xRRGGBB`.
    Rgb(u32),
    /// Back to the terminal's own colour.
    Default,
}

impl Colour {
    /// Parses `#RRGGBB` (either case) or `default`.
    pub fn parse(text: &str) -> Option<Self> {
        if text == "default" {
            return Some(Self::Default);
        }
        let hex = text.strip_prefix('#')?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(hex, 16).ok().map(Self::Rgb)
    }
}

/// What one `terminal.set-theme` call changes; what it leaves out stays.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeChange {
    pub foreground: Option<Colour>,
    pub background: Option<Colour>,
    pub cursor: Option<Colour>,
    /// Palette entries, each index at most once, in the order given.
    pub palette: Vec<(u8, Colour)>,
}

impl ThemeChange {
    /// The change the named arguments ask for, or why it cannot be made
    /// whole: a colour that does not parse, a palette entry that is not
    /// `INDEX=COLOUR` with an index from 0 to 255, an index given twice, or
    /// nothing to change. Other arguments are not looked at.
    pub fn from_args(args: &[(&str, Value)]) -> Result<Self, CallError> {
        let mut change = Self::default();
        for (name, value) in args {
            match (*name, value) {
                ("foreground", Value::Text(text)) => {
                    change.foreground = Some(colour("foreground", text)?);
                }
                ("background", Value::Text(text)) => {
                    change.background = Some(colour("background", text)?);
                }
                ("cursor", Value::Text(text)) => change.cursor = Some(colour("cursor", text)?),
                ("palette", Value::Words(entries)) => {
                    for entry in entries {
                        change.add_entry(entry)?;
                    }
                }
                _ => {}
            }
        }
        if change.is_empty() {
            return Err(invalid(
                "give at least one of foreground, background, cursor and palette",
            ));
        }
        Ok(change)
    }

    fn is_empty(&self) -> bool {
        self.foreground.is_none()
            && self.background.is_none()
            && self.cursor.is_none()
            && self.palette.is_empty()
    }

    fn add_entry(&mut self, entry: &str) -> Result<(), CallError> {
        let bad = || invalid(format!("`{entry}` is not INDEX=COLOUR"));
        let (index, value) = entry.split_once('=').ok_or_else(bad)?;
        if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        let index: u8 = index
            .parse()
            .map_err(|_| invalid(format!("index {index} is past 255")))?;
        if self.palette.iter().any(|(n, _)| *n == index) {
            return Err(invalid(format!("index {index} is given twice")));
        }
        let colour = Colour::parse(value)
            .ok_or_else(|| invalid(format!("`{value}` is not #RRGGBB or default")))?;
        self.palette.push((index, colour));
        Ok(())
    }
}

fn colour(param: &'static str, text: &str) -> Result<Colour, CallError> {
    Colour::parse(text).ok_or_else(|| CallError::Invalid {
        param,
        why: format!("`{text}` is not #RRGGBB or default"),
    })
}

fn invalid(why: impl Into<String>) -> CallError {
    CallError::Invalid {
        param: "palette",
        why: why.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(name: &'static str, value: &str) -> (&'static str, Value) {
        (name, Value::Text(value.to_owned()))
    }

    fn palette(entries: &[&str]) -> (&'static str, Value) {
        let words = entries.iter().map(|e| (*e).to_owned()).collect();
        ("palette", Value::Words(words))
    }

    #[test]
    fn colours_are_six_hex_digits_or_default() {
        assert_eq!(Colour::parse("#a0B1c2"), Some(Colour::Rgb(0xa0b1c2)));
        assert_eq!(Colour::parse("default"), Some(Colour::Default));
        for bad in [
            "", "#", "#12345", "#1234567", "#12345g", "a0b1c2", "red", "#+12345", "Default",
        ] {
            assert_eq!(Colour::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_change_keeps_every_colour_and_entry_in_order() {
        let change = ThemeChange::from_args(&[
            ("block", Value::Lines(0)),
            text("foreground", "#010203"),
            text("background", "default"),
            text("cursor", "#ffffff"),
            palette(&["255=#000001", "0=default"]),
        ])
        .expect("valid");
        assert_eq!(
            change,
            ThemeChange {
                foreground: Some(Colour::Rgb(0x010203)),
                background: Some(Colour::Default),
                cursor: Some(Colour::Rgb(0xffffff)),
                palette: vec![(255, Colour::Rgb(1)), (0, Colour::Default)],
            }
        );
    }

    #[test]
    fn a_change_is_refused_unless_whole() {
        for (args, param, why) in [
            (vec![], "palette", "at least one"),
            (vec![palette(&[])], "palette", "at least one"),
            (vec![text("cursor", "blue")], "cursor", "#RRGGBB"),
            (vec![palette(&["256=#000000"])], "palette", "past 255"),
            (vec![palette(&["-1=#000000"])], "palette", "INDEX=COLOUR"),
            (vec![palette(&["+1=#000000"])], "palette", "INDEX=COLOUR"),
            (vec![palette(&["=#000000"])], "palette", "INDEX=COLOUR"),
            (vec![palette(&["1"])], "palette", "INDEX=COLOUR"),
            (vec![palette(&["1=#00000"])], "palette", "#RRGGBB"),
            (
                vec![palette(&["1=default", "1=#000000"])],
                "palette",
                "twice",
            ),
        ] {
            match ThemeChange::from_args(&args) {
                Err(CallError::Invalid { param: p, why: w }) => {
                    assert_eq!(p, param, "{args:?}");
                    assert!(w.contains(why), "{args:?}: {w}");
                }
                other => panic!("{args:?}: expected Invalid, got {other:?}"),
            }
        }
    }
}
