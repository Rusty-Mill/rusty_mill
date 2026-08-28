//! Pluggable control over the whitespace/indentation/escaping
//! [`crate::to_string`] and friends produce, via the [`Formatter`] trait.
//! [`CompactFormatter`] and [`PrettyFormatter`] are the built-in compact/
//! pretty implementations; write your own for anything else (custom
//! indentation, HTML-safe escaping, etc.).

use alloc::string::String;

/// A single-character escape sequence, passed to
/// [`Formatter::write_char_escape`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharEscape {
    /// `"` -> `\"`
    Quote,
    /// `\` -> `\\`
    ReverseSolidus,
    /// U+0008 -> `\b`
    Backspace,
    /// U+000C -> `\f`
    FormFeed,
    /// `\n` -> `\n` (escaped)
    LineFeed,
    /// `\r` -> `\r` (escaped)
    CarriageReturn,
    /// `\t` -> `\t` (escaped)
    Tab,
    /// Any other control character (U+0000..U+001F) -> `\u00XX`
    AsciiControl(u8),
}

/// Controls how [`crate::to_string`]/[`crate::to_string_pretty`] (and
/// friends) render JSON syntax around values. All methods have defaults
/// producing [`CompactFormatter`]'s output; override only what you need to
/// change.
pub trait Formatter {
    /// Writes a JSON `null`.
    fn write_null(&mut self, out: &mut String) {
        out.push_str("null");
    }

    /// Writes a JSON `true`/`false`.
    fn write_bool(&mut self, out: &mut String, value: bool) {
        out.push_str(if value { "true" } else { "false" });
    }

    /// Writes an already-formatted JSON number token verbatim.
    fn write_number_str(&mut self, out: &mut String, value: &str) {
        out.push_str(value);
    }

    /// Writes the opening `"` of a JSON string.
    fn begin_string(&mut self, out: &mut String) {
        out.push('"');
    }

    /// Writes the closing `"` of a JSON string.
    fn end_string(&mut self, out: &mut String) {
        out.push('"');
    }

    /// Writes a run of string content that needs no escaping.
    fn write_string_fragment(&mut self, out: &mut String, fragment: &str) {
        out.push_str(fragment);
    }

    /// Writes one escaped character within a JSON string.
    fn write_char_escape(&mut self, out: &mut String, escape: CharEscape) {
        match escape {
            CharEscape::Quote => out.push_str("\\\""),
            CharEscape::ReverseSolidus => out.push_str("\\\\"),
            CharEscape::Backspace => out.push_str("\\b"),
            CharEscape::FormFeed => out.push_str("\\f"),
            CharEscape::LineFeed => out.push_str("\\n"),
            CharEscape::CarriageReturn => out.push_str("\\r"),
            CharEscape::Tab => out.push_str("\\t"),
            CharEscape::AsciiControl(byte) => {
                out.push_str(&alloc::format!("\\u{byte:04x}"));
            }
        }
    }

    /// Writes the opening `[` of a JSON array.
    fn begin_array(&mut self, out: &mut String) {
        out.push('[');
    }

    /// Writes the closing `]` of a JSON array. `empty` is `true` if no
    /// elements were written.
    fn end_array(&mut self, out: &mut String, empty: bool) {
        let _ = empty;
        out.push(']');
    }

    /// Writes whatever separates array elements, before the `first` one
    /// (never a separator) or a later one (a `,`).
    fn begin_array_value(&mut self, out: &mut String, first: bool) {
        if !first {
            out.push(',');
        }
    }

    /// Called after an array element has been written.
    fn end_array_value(&mut self, out: &mut String) {
        let _ = out;
    }

    /// Writes the opening `{` of a JSON object.
    fn begin_object(&mut self, out: &mut String) {
        out.push('{');
    }

    /// Writes the closing `}` of a JSON object. `empty` is `true` if no
    /// entries were written.
    fn end_object(&mut self, out: &mut String, empty: bool) {
        let _ = empty;
        out.push('}');
    }

    /// Writes whatever separates object entries, before the `first` one
    /// (never a separator) or a later one (a `,`).
    fn begin_object_key(&mut self, out: &mut String, first: bool) {
        if !first {
            out.push(',');
        }
    }

    /// Called after an object key has been written.
    fn end_object_key(&mut self, out: &mut String) {
        let _ = out;
    }

    /// Writes the `:` between an object key and its value.
    fn begin_object_value(&mut self, out: &mut String) {
        out.push(':');
    }

    /// Called after an object value has been written.
    fn end_object_value(&mut self, out: &mut String) {
        let _ = out;
    }
}

/// The default [`Formatter`]: no extra whitespace, matching
/// [`crate::to_string`].
#[derive(Clone, Copy, Debug, Default)]
pub struct CompactFormatter;

impl Formatter for CompactFormatter {}

/// A [`Formatter`] that indents nested arrays/objects one level per
/// two spaces (configurable), matching [`crate::to_string_pretty`]. Empty
/// arrays/objects stay inline (`[]`, `{}`) rather than spreading across
/// lines.
#[derive(Clone, Debug)]
pub struct PrettyFormatter {
    indent_width: usize,
    current_indent: usize,
}

impl PrettyFormatter {
    /// Two-space indentation, matching [`crate::to_string_pretty`]'s default.
    pub fn new() -> Self {
        Self::with_indent_width(2)
    }

    /// Indents `width` spaces per nesting level.
    pub fn with_indent_width(width: usize) -> Self {
        PrettyFormatter {
            indent_width: width,
            current_indent: 0,
        }
    }

    fn push_indent(&self, out: &mut String) {
        for _ in 0..self.current_indent {
            for _ in 0..self.indent_width {
                out.push(' ');
            }
        }
    }
}

impl Default for PrettyFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl Formatter for PrettyFormatter {
    fn begin_array(&mut self, out: &mut String) {
        self.current_indent += 1;
        out.push('[');
    }

    fn end_array(&mut self, out: &mut String, empty: bool) {
        self.current_indent -= 1;
        if !empty {
            out.push('\n');
            self.push_indent(out);
        }
        out.push(']');
    }

    fn begin_array_value(&mut self, out: &mut String, first: bool) {
        if !first {
            out.push(',');
        }
        out.push('\n');
        self.push_indent(out);
    }

    fn begin_object(&mut self, out: &mut String) {
        self.current_indent += 1;
        out.push('{');
    }

    fn end_object(&mut self, out: &mut String, empty: bool) {
        self.current_indent -= 1;
        if !empty {
            out.push('\n');
            self.push_indent(out);
        }
        out.push('}');
    }

    fn begin_object_key(&mut self, out: &mut String, first: bool) {
        if !first {
            out.push(',');
        }
        out.push('\n');
        self.push_indent(out);
    }

    fn begin_object_value(&mut self, out: &mut String) {
        out.push_str(": ");
    }
}
