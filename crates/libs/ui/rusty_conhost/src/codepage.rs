//! Code pages for the `A` APIs (design §5.9, host PR 9).
//!
//! The console keeps two: the input code page (`SetConsoleCP`), which
//! ANSI reads and the `A` title calls use, and the output code page
//! (`SetConsoleOutputCP`), which `WriteConsoleA` and raw writes use.
//! Both start at 65001 (divergence V4). Conversion goes through a
//! [`Codec`] injected like `KeyFor`: `platform_windows::condrv` on
//! Windows, tables for 437, 850 and a slice of 932 in tests.
//!
//! Behaviour follows microsoft/terminal's conhost (`src/host/getset.cpp`,
//! `_stream.cpp`, `dbcs.cpp`, `inputBuffer.cpp`; see
//! THIRD_PARTY_NOTICES.md):
//!
//! Copyright (c) Microsoft Corporation. Licensed under the MIT license.

/// UTF-8, the code page both start at (V4).
pub(crate) const UTF8: u32 = 65001;
/// UTF-7: its bytes are all ASCII, so it has no lead bytes to probe.
pub(crate) const UTF7: u32 = 65000;

/// Code-page conversion. `decode` and `encode` are `MultiByteToWideChar`
/// and `WideCharToMultiByte` without `MB_ERR_INVALID_CHARS`: what a code
/// page can't represent becomes U+FFFD or its default character. Both
/// give `None` only for a code page `valid` refuses.
#[derive(Clone, Copy)]
pub(crate) struct Codec {
    pub valid: fn(u32) -> bool,
    pub decode: fn(u32, &[u8]) -> Option<String>,
    pub encode: fn(u32, &str) -> Option<Vec<u8>>,
}

/// A code page and its DBCS lead bytes, which `WriteConsoleA` needs to
/// keep a character whole across calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CodePage {
    pub cp: u32,
    /// Bit `b - 0x80` is set when byte `b` starts a two-byte character.
    lead: u128,
}

impl CodePage {
    pub(crate) const UTF8: CodePage = CodePage { cp: UTF8, lead: 0 };

    /// `cp` with its lead bytes, or `None` when it is not installed
    /// (conhost: E_INVALIDARG). conhost reads them from `GetCPInfo`; the
    /// codec gives the same answer, since a lead byte and the trail byte
    /// `A` (valid in 932, 936, 949, 950 and 54936) decode to one unit,
    /// any other byte and `A` to two.
    pub(crate) fn new(cp: u32, codec: &Codec) -> Option<CodePage> {
        if !(codec.valid)(cp) {
            return None;
        }
        if cp == UTF8 || cp == UTF7 {
            return Some(CodePage { cp, lead: 0 });
        }
        let lead = (0x80..=0xFFu8)
            .filter(|&b| {
                (codec.decode)(cp, &[b, b'A']).is_some_and(|s| s.encode_utf16().count() == 1)
            })
            .fold(0u128, |bits, b| bits | 1 << (b - 0x80));
        Some(CodePage { cp, lead })
    }

    pub(crate) fn is_utf8(self) -> bool {
        self.cp == UTF8
    }

    /// A code page whose decoding carries shift state from byte to byte:
    /// UTF-7, the ISO-2022 family and HZ. A write in one decodes whole,
    /// so it can't be streamed in slices (design §7.4).
    pub(crate) fn is_stateful(self) -> bool {
        matches!(self.cp, UTF7 | 50220..=50229 | 52936)
    }

    fn is_lead(self, b: u8) -> bool {
        b >= 0x80 && self.lead & 1 << (b - 0x80) != 0
    }

    /// conhost `CheckBisectStringA`: whether the last byte of `bytes` is
    /// a lead byte whose trail byte has not come yet.
    pub(crate) fn ends_in_lead(self, bytes: &[u8]) -> bool {
        let mut i = 0;
        while i < bytes.len() {
            if self.is_lead(bytes[i]) {
                if i + 1 == bytes.len() {
                    return true;
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        false
    }
}

/// The length of the unfinished UTF-8 sequence that ends `bytes`, if
/// any: a lead byte whose continuation bytes have not all come.
pub(crate) fn unfinished_utf8(bytes: &[u8]) -> usize {
    (1..=bytes.len().min(3))
        .find(|&n| {
            std::str::from_utf8(&bytes[bytes.len() - n..])
                .is_err_and(|e| e.valid_up_to() == 0 && e.error_len().is_none())
        })
        .unwrap_or(0)
}

/// `bytes` in `cp` as text; bytes a codec can't take come through as
/// UTF-8, lossily, which only a code page uninstalled under us can cause.
pub(crate) fn decode(codec: &Codec, cp: u32, bytes: &[u8]) -> String {
    if cp == UTF8 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    (codec.decode)(cp, bytes).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
}

/// `text` in `cp`, falling back to UTF-8 as [`decode`] does.
pub(crate) fn encode(codec: &Codec, cp: u32, text: &str) -> Vec<u8> {
    if cp == UTF8 {
        return text.as_bytes().to_vec();
    }
    (codec.encode)(cp, text).unwrap_or_else(|| text.as_bytes().to_vec())
}

/// Tables standing in for the OS in tests: 437, 850, and of 932 the
/// ASCII half, half-width katakana and two hiragana pairs. Also the fuzz
/// driver's codec, which must not reach the OS.
#[cfg(any(test, feature = "fuzz"))]
pub(crate) mod table {
    use super::{Codec, UTF8};

    pub(crate) const CODEC: Codec = Codec {
        valid,
        decode,
        encode,
    };

    const CP437: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{a0}";
    const CP850: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜø£Ø×ƒáíóúñÑªº¿®¬½¼¡«»░▒▓│┤ÁÂÀ©╣║╗╝¢¥┐└┴┬├─┼ãÃ╚╔╩╦╠═╬¤ðÐÊËÈıÍÎÏ┘┌█▄¦Ì▀ÓßÔÒõÕµþÞÚÛÙýÝ¯´\u{ad}±‗¾¶§÷¸°¨·¹³²■\u{a0}";
    const PAIRS_932: [([u8; 2], char); 2] = [([0x82, 0xA0], 'あ'), ([0x82, 0xA2], 'い')];
    /// What 932 shows for a pair it has no character for.
    const DEFAULT_932: char = '\u{30FB}';

    fn valid(cp: u32) -> bool {
        matches!(cp, 437 | 850 | 932 | UTF8)
    }

    fn high(cp: u32) -> Option<&'static str> {
        match cp {
            437 => Some(CP437),
            850 => Some(CP850),
            _ => None,
        }
    }

    fn is_lead_932(b: u8) -> bool {
        matches!(b, 0x81..=0x9F | 0xE0..=0xFC)
    }

    fn decode(cp: u32, bytes: &[u8]) -> Option<String> {
        if cp == UTF8 {
            return Some(String::from_utf8_lossy(bytes).into_owned());
        }
        if cp == 932 {
            return Some(decode_932(bytes));
        }
        let table = high(cp)?;
        Some(
            bytes
                .iter()
                .map(|&b| match b {
                    0..=0x7F => char::from(b),
                    _ => table.chars().nth(usize::from(b - 0x80)).unwrap_or('?'),
                })
                .collect(),
        )
    }

    fn decode_932(bytes: &[u8]) -> String {
        let mut out = String::new();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if is_lead_932(b) && i + 1 < bytes.len() {
                let pair = [b, bytes[i + 1]];
                let ch = PAIRS_932
                    .iter()
                    .find(|(p, _)| *p == pair)
                    .map_or(DEFAULT_932, |e| e.1);
                out.push(ch);
                i += 2;
                continue;
            }
            out.push(match b {
                0..=0x7F => char::from(b),
                0xA1..=0xDF => char::from_u32(0xFF61 + u32::from(b - 0xA1)).unwrap_or('?'),
                _ => DEFAULT_932,
            });
            i += 1;
        }
        out
    }

    fn encode(cp: u32, text: &str) -> Option<Vec<u8>> {
        if cp == UTF8 {
            return Some(text.as_bytes().to_vec());
        }
        let mut out = Vec::new();
        for ch in text.chars() {
            if ch.is_ascii() {
                out.push(ch as u8);
                continue;
            }
            if cp == 932 {
                match PAIRS_932.iter().find(|e| e.1 == ch) {
                    Some((pair, _)) => out.extend(pair),
                    None => out.push(b'?'),
                }
                continue;
            }
            let at = high(cp)?.chars().position(|c| c == ch);
            out.push(at.map_or(b'?', |i| 0x80 + i as u8));
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::table::CODEC;
    use super::*;

    #[test]
    fn lead_bytes_come_from_the_codec() {
        let sbcs = CodePage::new(850, &CODEC).expect("850");
        assert!(!(0x80..=0xFF).any(|b| sbcs.is_lead(b)));
        let dbcs = CodePage::new(932, &CODEC).expect("932");
        assert!(dbcs.is_lead(0x81) && dbcs.is_lead(0x9F) && dbcs.is_lead(0xE0));
        assert!(!dbcs.is_lead(0xA1) && !dbcs.is_lead(0x80) && !dbcs.is_lead(b'A'));
        assert_eq!(CodePage::new(UTF8, &CODEC), Some(CodePage::UTF8));
        assert_eq!(CodePage::new(1, &CODEC), None, "not installed");
    }

    #[test]
    fn a_trailing_lead_byte_is_found_after_whole_pairs() {
        let cp = CodePage::new(932, &CODEC).expect("932");
        assert!(cp.ends_in_lead(b"a\x82"));
        assert!(!cp.ends_in_lead(b"a\x82\xa0"));
        // 0x82 0x82 is one pair, so the last 0x82 is its trail byte.
        assert!(!cp.ends_in_lead(b"\x82\x82"));
        assert!(cp.ends_in_lead(b"\x82\x82\x82"));
        assert!(!cp.ends_in_lead(b""));
        let sbcs = CodePage::new(437, &CODEC).expect("437");
        assert!(!sbcs.ends_in_lead(b"\x82"));
    }

    #[test]
    fn an_unfinished_utf8_sequence_is_measured() {
        assert_eq!(unfinished_utf8(b"a\xc3"), 1);
        assert_eq!(unfinished_utf8(b"\xe2\x96"), 2);
        assert_eq!(unfinished_utf8(b"\xf0\x9f\x98"), 3);
        assert_eq!(unfinished_utf8("é".as_bytes()), 0);
        assert_eq!(unfinished_utf8(b"\xa9"), 0, "a stray continuation byte");
        assert_eq!(unfinished_utf8(b""), 0);
    }

    #[test]
    fn conversion_skips_the_codec_for_utf8() {
        assert_eq!(decode(&CODEC, 437, b"\xb0A"), "\u{2591}A");
        assert_eq!(decode(&CODEC, 850, b"\x9b"), "ø");
        assert_eq!(encode(&CODEC, 850, "ø\u{4e00}"), b"\x9b?");
        assert_eq!(decode(&CODEC, UTF8, b"\xe2\x96\x91"), "\u{2591}");
        assert_eq!(encode(&CODEC, UTF8, "é"), "é".as_bytes());
        assert_eq!(decode(&CODEC, 932, b"\x82\xa0\xb1"), "あｱ");
        assert_eq!(encode(&CODEC, 932, "あ"), b"\x82\xa0");
    }
}
