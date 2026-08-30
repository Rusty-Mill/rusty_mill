//! A zero-allocation, `#![no_std]` VT100/CSI/OSC ANSI escape sequence parser core.
//!
//! Provides fast, zero-copy parsing of ANSI escape sequences into explicit tokens,
//! stripping ANSI codes from string buffers, and calculating visible text display widths.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use unicode_width::UnicodeWidthChar;

/// An ANSI color representation (8-color, 16-color, 256-color, or 24-bit TrueColor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// Standard 8/16 ANSI color (0..15).
    Named(u8),
    /// 256-color palette index (0..255).
    Palette(u8),
    /// 24-bit RGB TrueColor.
    Rgb(u8, u8, u8),
}

/// SGR (Select Graphic Rendition) text attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    /// Foreground color.
    pub fg: Option<Color>,
    /// Background color.
    pub bg: Option<Color>,
    /// Bold attribute.
    pub bold: bool,
    /// Dim/faint attribute.
    pub dim: bool,
    /// Italic attribute.
    pub italic: bool,
    /// Underline attribute.
    pub underline: bool,
    /// Reverse/inverse video attribute.
    pub reverse: bool,
    /// Hidden/conceal attribute.
    pub hidden: bool,
    /// Strikethrough attribute.
    pub strikethrough: bool,
}

/// Parsed ANSI token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnsiToken<'a> {
    /// Plain printable text slice.
    Text(&'a str),
    /// Control character (e.g. `\n`, `\r`, `\t`, `\x07`).
    Control(char),
    /// CSI (Control Sequence Introducer) sequence with parameter bytes and final command char.
    Csi {
        /// Intermediate/parameter slice (e.g. `"1;31"`).
        params: &'a str,
        /// Final command character (e.g. `'m'`, `'H'`, `'J'`).
        action: char,
    },
    /// OSC (Operating System Command) sequence.
    Osc {
        /// Command code.
        code: u32,
        /// Payload string slice.
        payload: &'a str,
    },
}

/// Zero-allocation, streaming ANSI escape sequence iterator over a string slice.
pub struct AnsiParser<'a> {
    input: &'a str,
}

impl<'a> AnsiParser<'a> {
    /// Create a new ANSI parser over input string `s`.
    pub fn new(s: &'a str) -> Self {
        AnsiParser { input: s }
    }
}

impl<'a> Iterator for AnsiParser<'a> {
    type Item = AnsiToken<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.input.is_empty() {
            return None;
        }

        // Find next ESC (\x1B) or control character
        if let Some(esc_idx) = self.input.find('\x1B') {
            if esc_idx > 0 {
                let text = &self.input[..esc_idx];
                self.input = &self.input[esc_idx..];
                return Some(AnsiToken::Text(text));
            }

            // Starts with ESC
            let rest = &self.input[1..];
            if let Some(csi_body) = rest.strip_prefix('[') {
                // CSI sequence: ESC [ <params> <final_char>
                if let Some(end_idx) = csi_body.find(|c: char| (0x40..=0x7E).contains(&(c as u32)))
                {
                    let params = &csi_body[..end_idx];
                    let action = csi_body[end_idx..].chars().next().unwrap();
                    let total_len = 1 + 1 + end_idx + action.len_utf8();
                    self.input = &self.input[total_len..];
                    return Some(AnsiToken::Csi { params, action });
                }
            } else if let Some(osc_body) = rest.strip_prefix(']') {
                // OSC sequence: ESC ] <code> ; <payload> (ST | BEL)
                if let Some(term_idx) = osc_body.find(['\x07', '\x1B']) {
                    let osc_str = &osc_body[..term_idx];
                    let terminator_len = if osc_body[term_idx..].starts_with("\x1B\\") {
                        2
                    } else {
                        1
                    };
                    let (code, payload) =
                        if let Some((code_str, payload_str)) = osc_str.split_once(';') {
                            (code_str.parse::<u32>().unwrap_or(0), payload_str)
                        } else {
                            (osc_str.parse::<u32>().unwrap_or(0), "")
                        };

                    self.input = &self.input[1 + 1 + term_idx + terminator_len..];
                    return Some(AnsiToken::Osc { code, payload });
                }
            }

            // Fallback for isolated ESC
            self.input = &self.input[1..];
            return Some(AnsiToken::Control('\x1B'));
        }

        // Check for non-ESC control characters
        let mut char_iter = self.input.char_indices();
        let (_first_idx, first_char) = char_iter.next().unwrap();
        if first_char.is_control() && first_char != '\n' && first_char != '\r' && first_char != '\t'
        {
            self.input = &self.input[first_char.len_utf8()..];
            return Some(AnsiToken::Control(first_char));
        }

        // Return printable text up to next ESC or control char
        let end_idx = self
            .input
            .find(|c: char| c == '\x1B' || (c.is_control() && c != '\n' && c != '\r' && c != '\t'))
            .unwrap_or(self.input.len());

        let text = &self.input[..end_idx];
        self.input = &self.input[end_idx..];
        Some(AnsiToken::Text(text))
    }
}

/// Strip all ANSI escape sequences from input `s`.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for token in AnsiParser::new(s) {
        match token {
            AnsiToken::Text(t) => out.push_str(t),
            AnsiToken::Control(c) if c == '\n' || c == '\r' || c == '\t' => out.push(c),
            _ => {}
        }
    }
    out
}

/// Calculate the visible display width (columns) of a string ignoring ANSI codes.
pub fn visible_width(s: &str) -> usize {
    let mut width = 0;
    for token in AnsiParser::new(s) {
        if let AnsiToken::Text(t) = token {
            for c in t.chars() {
                width += c.width().unwrap_or(0);
            }
        }
    }
    width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_text() {
        let mut p = AnsiParser::new("hello world");
        assert_eq!(p.next(), Some(AnsiToken::Text("hello world")));
        assert_eq!(p.next(), None);
    }

    #[test]
    fn parses_csi_sequences() {
        let mut p = AnsiParser::new("\x1B[1;31mRed Text\x1B[0m");
        assert_eq!(
            p.next(),
            Some(AnsiToken::Csi {
                params: "1;31",
                action: 'm'
            })
        );
        assert_eq!(p.next(), Some(AnsiToken::Text("Red Text")));
        assert_eq!(
            p.next(),
            Some(AnsiToken::Csi {
                params: "0",
                action: 'm'
            })
        );
        assert_eq!(p.next(), None);
    }

    #[test]
    fn strips_ansi_codes() {
        let colored = "\x1B[1;32mSUCCESS:\x1B[0m All tests passed!";
        assert_eq!(strip_ansi(colored), "SUCCESS: All tests passed!");
    }

    #[test]
    fn calculates_visible_width() {
        let colored = "\x1B[31mhello\x1B[0m \x1B[32mworld\x1B[0m";
        assert_eq!(visible_width(colored), 11);
    }
}
