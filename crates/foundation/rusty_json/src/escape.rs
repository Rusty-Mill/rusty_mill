//! RFC 8259 §7 string escaping, shared by every JSON writer in this crate
//! ([`crate::ser`] when the `serde` feature is on, [`crate::value_io`]
//! always) so there is exactly one place that decides which characters get
//! escaped.

use crate::formatter::{CharEscape, Formatter};
use alloc::string::String;

pub(crate) fn write_escaped_str<F: Formatter>(formatter: &mut F, out: &mut String, value: &str) {
    formatter.begin_string(out);
    let mut start = 0;
    for (i, c) in value.char_indices() {
        let escape = match c {
            '"' => Some(CharEscape::Quote),
            '\\' => Some(CharEscape::ReverseSolidus),
            '\u{0008}' => Some(CharEscape::Backspace),
            '\u{000C}' => Some(CharEscape::FormFeed),
            '\n' => Some(CharEscape::LineFeed),
            '\r' => Some(CharEscape::CarriageReturn),
            '\t' => Some(CharEscape::Tab),
            c if (c as u32) < 0x20 => Some(CharEscape::AsciiControl(c as u8)),
            _ => None,
        };
        if let Some(escape) = escape {
            if start < i {
                formatter.write_string_fragment(out, &value[start..i]);
            }
            formatter.write_char_escape(out, escape);
            start = i + c.len_utf8();
        }
    }
    if start < value.len() {
        formatter.write_string_fragment(out, &value[start..]);
    }
    formatter.end_string(out);
}
