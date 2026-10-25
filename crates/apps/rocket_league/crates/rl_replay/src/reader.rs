//! A bounds-checked little-endian byte reader and the format's two string
//! encodings. Offsets in errors are absolute: `base` is where `data` starts in
//! the file.

use crate::error::Error;

/// Longest text the format allows, in characters (as boxcars).
const MAX_TEXT_CHARS: i32 = 10_000;

/// Reads `data` front to back.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    base: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8], base: usize) -> Self {
        Reader { data, pos: 0, base }
    }

    /// Absolute offset of the next byte.
    pub(crate) fn offset(&self) -> usize {
        self.base + self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub(crate) fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], Error> {
        let available = self.remaining();
        if n > available {
            return Err(Error::Truncated {
                what,
                offset: self.offset(),
                needed: n,
                available,
            });
        }
        let bytes = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N], Error> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N, what)?);
        Ok(out)
    }

    pub(crate) fn u8(&mut self, what: &'static str) -> Result<u8, Error> {
        Ok(self.array::<1>(what)?[0])
    }

    pub(crate) fn i32(&mut self, what: &'static str) -> Result<i32, Error> {
        self.array(what).map(i32::from_le_bytes)
    }

    pub(crate) fn u32(&mut self, what: &'static str) -> Result<u32, Error> {
        self.array(what).map(u32::from_le_bytes)
    }

    pub(crate) fn u64(&mut self, what: &'static str) -> Result<u64, Error> {
        self.array(what).map(u64::from_le_bytes)
    }

    pub(crate) fn f32(&mut self, what: &'static str) -> Result<f32, Error> {
        self.array(what).map(f32::from_le_bytes)
    }

    /// A non-negative `i32` length or count.
    pub(crate) fn len(&mut self, what: &'static str) -> Result<usize, Error> {
        let offset = self.offset();
        let value = self.i32(what)?;
        usize::try_from(value).map_err(|_| Error::BadLength {
            what,
            offset,
            value: value.into(),
        })
    }

    /// A UTF-8 name: byte length (including a trailing NUL), then the bytes.
    /// Used for property keys and kinds.
    pub(crate) fn name(&mut self, what: &'static str) -> Result<String, Error> {
        let n = self.len(what)?;
        let offset = self.offset();
        let bytes = self.take(n, what)?;
        let text = &bytes[..n.saturating_sub(1)];
        std::str::from_utf8(text)
            .map(str::to_owned)
            .map_err(|_| Error::InvalidUtf8 { offset })
    }

    /// Display text: a positive length is Windows-1252 bytes, a negative one is
    /// that many UTF-16LE units; either way the last character is a NUL.
    pub(crate) fn text(&mut self, what: &'static str) -> Result<String, Error> {
        let offset = self.offset();
        let chars = self.i32(what)?;
        if !(-MAX_TEXT_CHARS..=MAX_TEXT_CHARS).contains(&chars) {
            return Err(Error::BadLength {
                what,
                offset,
                value: chars.into(),
            });
        }
        let units = chars.unsigned_abs() as usize;
        if chars < 0 {
            let bytes = self.take(units * 2, what)?;
            Ok(decode_utf16le(&bytes[..bytes.len().saturating_sub(2)]))
        } else {
            let bytes = self.take(units, what)?;
            Ok(decode_windows1252(&bytes[..bytes.len().saturating_sub(1)]))
        }
    }
}

/// UTF-16LE with unpaired surrogates replaced by U+FFFD.
fn decode_utf16le(bytes: &[u8]) -> String {
    let units = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
    char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// Windows-1252 as the WHATWG encoding standard maps it: Latin-1 except
/// 0x80..=0x9F, whose five unassigned bytes map to the matching C1 controls.
fn decode_windows1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => HIGH_C1[usize::from(b - 0x80)],
            _ => char::from(b),
        })
        .collect()
}

#[rustfmt::skip]
const HIGH_C1: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn name_drops_the_trailing_nul() {
        let data = [5, 0, 0, 0, b'N', b'o', b'n', b'e', 0];
        assert_eq!(Reader::new(&data, 0).name("key").unwrap(), "None");
    }

    #[test]
    fn empty_text_is_empty() {
        assert_eq!(Reader::new(&[0, 0, 0, 0], 0).text("t").unwrap(), "");
    }

    #[test]
    fn negative_length_text_is_utf16() {
        // -3 units: U+2623, 'D', NUL.
        let data = [0xFD, 0xFF, 0xFF, 0xFF, 0x23, 0x26, b'D', 0, 0, 0];
        assert_eq!(Reader::new(&data, 0).text("t").unwrap(), "\u{2623}D");
    }

    #[test]
    fn unpaired_surrogate_becomes_a_replacement_character() {
        let data = [0xFD, 0xFF, 0xFF, 0xFF, 0xD8, 0xD8, 0, 0, 0, 0];
        assert_eq!(Reader::new(&data, 0).text("t").unwrap(), "\u{FFFD}\u{0}");
    }

    #[test]
    fn windows1252_maps_the_high_block() {
        assert_eq!(
            decode_windows1252(&[b'a', 0xB3, 0x80, 0x81, 0x9F]),
            "a\u{B3}\u{20AC}\u{81}\u{178}"
        );
    }

    #[test]
    fn oversized_text_is_a_bad_length() {
        let data = 0xCCCC_CCCCu32.to_le_bytes();
        let err = Reader::new(&data, 7).text("t").unwrap_err();
        assert!(matches!(err, Error::BadLength { offset: 7, .. }), "{err}");
    }

    #[test]
    fn short_data_reports_where_and_how_much() {
        let err = Reader::new(&[1, 2], 100).u32("size").unwrap_err();
        assert_eq!(
            err,
            Error::Truncated {
                what: "size",
                offset: 100,
                needed: 4,
                available: 2
            }
        );
    }

    #[test]
    fn negative_length_is_a_bad_length() {
        let data = (-1i32).to_le_bytes();
        assert!(matches!(
            Reader::new(&data, 0).len("count"),
            Err(Error::BadLength { value: -1, .. })
        ));
    }
}
