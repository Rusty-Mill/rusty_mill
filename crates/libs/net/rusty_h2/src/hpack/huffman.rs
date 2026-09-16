//! HPACK Huffman coding (RFC 7541 §5.2 and Appendix B).

use crate::error::{ErrorCode, H2Error, Result};
use std::sync::OnceLock;

/// (code, bit-length) for each of the 256 byte symbols, in order, taken
/// verbatim from RFC 7541 Appendix B.
const CODES: [(u32, u8); 256] = [
    (0x1ff8, 13),
    (0x7fffd8, 23),
    (0xfffffe2, 28),
    (0xfffffe3, 28),
    (0xfffffe4, 28),
    (0xfffffe5, 28),
    (0xfffffe6, 28),
    (0xfffffe7, 28),
    (0xfffffe8, 28),
    (0xffffea, 24),
    (0x3ffffffc, 30),
    (0xfffffe9, 28),
    (0xfffffea, 28),
    (0x3ffffffd, 30),
    (0xfffffeb, 28),
    (0xfffffec, 28),
    (0xfffffed, 28),
    (0xfffffee, 28),
    (0xfffffef, 28),
    (0xffffff0, 28),
    (0xffffff1, 28),
    (0xffffff2, 28),
    (0x3ffffffe, 30),
    (0xffffff3, 28),
    (0xffffff4, 28),
    (0xffffff5, 28),
    (0xffffff6, 28),
    (0xffffff7, 28),
    (0xffffff8, 28),
    (0xffffff9, 28),
    (0xffffffa, 28),
    (0xffffffb, 28),
    (0x14, 6),
    (0x3f8, 10),
    (0x3f9, 10),
    (0xffa, 12),
    (0x1ff9, 13),
    (0x15, 6),
    (0xf8, 8),
    (0x7fa, 11),
    (0x3fa, 10),
    (0x3fb, 10),
    (0xf9, 8),
    (0x7fb, 11),
    (0xfa, 8),
    (0x16, 6),
    (0x17, 6),
    (0x18, 6),
    (0x0, 5),
    (0x1, 5),
    (0x2, 5),
    (0x19, 6),
    (0x1a, 6),
    (0x1b, 6),
    (0x1c, 6),
    (0x1d, 6),
    (0x1e, 6),
    (0x1f, 6),
    (0x5c, 7),
    (0xfb, 8),
    (0x7ffc, 15),
    (0x20, 6),
    (0xffb, 12),
    (0x3fc, 10),
    (0x1ffa, 13),
    (0x21, 6),
    (0x5d, 7),
    (0x5e, 7),
    (0x5f, 7),
    (0x60, 7),
    (0x61, 7),
    (0x62, 7),
    (0x63, 7),
    (0x64, 7),
    (0x65, 7),
    (0x66, 7),
    (0x67, 7),
    (0x68, 7),
    (0x69, 7),
    (0x6a, 7),
    (0x6b, 7),
    (0x6c, 7),
    (0x6d, 7),
    (0x6e, 7),
    (0x6f, 7),
    (0x70, 7),
    (0x71, 7),
    (0x72, 7),
    (0xfc, 8),
    (0x73, 7),
    (0xfd, 8),
    (0x1ffb, 13),
    (0x7fff0, 19),
    (0x1ffc, 13),
    (0x3ffc, 14),
    (0x22, 6),
    (0x7ffd, 15),
    (0x3, 5),
    (0x23, 6),
    (0x4, 5),
    (0x24, 6),
    (0x5, 5),
    (0x25, 6),
    (0x26, 6),
    (0x27, 6),
    (0x6, 5),
    (0x74, 7),
    (0x75, 7),
    (0x28, 6),
    (0x29, 6),
    (0x2a, 6),
    (0x7, 5),
    (0x2b, 6),
    (0x76, 7),
    (0x2c, 6),
    (0x8, 5),
    (0x9, 5),
    (0x2d, 6),
    (0x77, 7),
    (0x78, 7),
    (0x79, 7),
    (0x7a, 7),
    (0x7b, 7),
    (0x7ffe, 15),
    (0x7fc, 11),
    (0x3ffd, 14),
    (0x1ffd, 13),
    (0xffffffc, 28),
    (0xfffe6, 20),
    (0x3fffd2, 22),
    (0xfffe7, 20),
    (0xfffe8, 20),
    (0x3fffd3, 22),
    (0x3fffd4, 22),
    (0x3fffd5, 22),
    (0x7fffd9, 23),
    (0x3fffd6, 22),
    (0x7fffda, 23),
    (0x7fffdb, 23),
    (0x7fffdc, 23),
    (0x7fffdd, 23),
    (0x7fffde, 23),
    (0xffffeb, 24),
    (0x7fffdf, 23),
    (0xffffec, 24),
    (0xffffed, 24),
    (0x3fffd7, 22),
    (0x7fffe0, 23),
    (0xffffee, 24),
    (0x7fffe1, 23),
    (0x7fffe2, 23),
    (0x7fffe3, 23),
    (0x7fffe4, 23),
    (0x1fffdc, 21),
    (0x3fffd8, 22),
    (0x7fffe5, 23),
    (0x3fffd9, 22),
    (0x7fffe6, 23),
    (0x7fffe7, 23),
    (0xffffef, 24),
    (0x3fffda, 22),
    (0x1fffdd, 21),
    (0xfffe9, 20),
    (0x3fffdb, 22),
    (0x3fffdc, 22),
    (0x7fffe8, 23),
    (0x7fffe9, 23),
    (0x1fffde, 21),
    (0x7fffea, 23),
    (0x3fffdd, 22),
    (0x3fffde, 22),
    (0xfffff0, 24),
    (0x1fffdf, 21),
    (0x3fffdf, 22),
    (0x7fffeb, 23),
    (0x7fffec, 23),
    (0x1fffe0, 21),
    (0x1fffe1, 21),
    (0x3fffe0, 22),
    (0x1fffe2, 21),
    (0x7fffed, 23),
    (0x3fffe1, 22),
    (0x7fffee, 23),
    (0x7fffef, 23),
    (0xfffea, 20),
    (0x3fffe2, 22),
    (0x3fffe3, 22),
    (0x3fffe4, 22),
    (0x7ffff0, 23),
    (0x3fffe5, 22),
    (0x3fffe6, 22),
    (0x7ffff1, 23),
    (0x3ffffe0, 26),
    (0x3ffffe1, 26),
    (0xfffeb, 20),
    (0x7fff1, 19),
    (0x3fffe7, 22),
    (0x7ffff2, 23),
    (0x3fffe8, 22),
    (0x1ffffec, 25),
    (0x3ffffe2, 26),
    (0x3ffffe3, 26),
    (0x3ffffe4, 26),
    (0x7ffffde, 27),
    (0x7ffffdf, 27),
    (0x3ffffe5, 26),
    (0xfffff1, 24),
    (0x1ffffed, 25),
    (0x7fff2, 19),
    (0x1fffe3, 21),
    (0x3ffffe6, 26),
    (0x7ffffe0, 27),
    (0x7ffffe1, 27),
    (0x3ffffe7, 26),
    (0x7ffffe2, 27),
    (0xfffff2, 24),
    (0x1fffe4, 21),
    (0x1fffe5, 21),
    (0x3ffffe8, 26),
    (0x3ffffe9, 26),
    (0xffffffd, 28),
    (0x7ffffe3, 27),
    (0x7ffffe4, 27),
    (0x7ffffe5, 27),
    (0xfffec, 20),
    (0xfffff3, 24),
    (0xfffed, 20),
    (0x1fffe6, 21),
    (0x3fffe9, 22),
    (0x1fffe7, 21),
    (0x1fffe8, 21),
    (0x7ffff3, 23),
    (0x3fffea, 22),
    (0x3fffeb, 22),
    (0x1ffffee, 25),
    (0x1ffffef, 25),
    (0xfffff4, 24),
    (0xfffff5, 24),
    (0x3ffffea, 26),
    (0x7ffff4, 23),
    (0x3ffffeb, 26),
    (0x7ffffe6, 27),
    (0x3ffffec, 26),
    (0x3ffffed, 26),
    (0x7ffffe7, 27),
    (0x7ffffe8, 27),
    (0x7ffffe9, 27),
    (0x7ffffea, 27),
    (0x7ffffeb, 27),
    (0xffffffe, 28),
    (0x7ffffec, 27),
    (0x7ffffed, 27),
    (0x7ffffee, 27),
    (0x7ffffef, 27),
    (0x7fffff0, 27),
    (0x3ffffee, 26),
];

/// The end-of-string symbol: 30 one-bits. Only ever used as padding at the
/// end of an encoded string; explicit occurrence in the input is an error.
const EOS: (u32, u8) = (0x3fffffff, 30);
const EOS_SYMBOL: u16 = 256;

/// Huffman-encode `input`, per RFC 7541 §5.2, padding the final byte with
/// the high-order bits of the EOS code.
pub fn encode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut acc: u64 = 0;
    let mut bits: u32 = 0;

    for &byte in input {
        let (code, len) = CODES[byte as usize];
        acc = (acc << len) | code as u64;
        bits += len as u32;
        while bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    if bits > 0 {
        let pad_len = 8 - bits;
        let padded = (acc << pad_len) | ((1u64 << pad_len) - 1);
        out.push(padded as u8);
    }
    out
}

/// Return the encoded length in bits, without allocating — used by the
/// encoder to decide whether Huffman coding is a net win over the literal.
pub fn encoded_len_bits(input: &[u8]) -> usize {
    input.iter().map(|&b| CODES[b as usize].1 as usize).sum()
}

struct TrieNode {
    children: [Option<Box<TrieNode>>; 2],
    symbol: Option<u16>,
}

impl TrieNode {
    fn new() -> Self {
        TrieNode {
            children: [None, None],
            symbol: None,
        }
    }
}

fn build_trie() -> TrieNode {
    let mut root = TrieNode::new();
    let insert = |root: &mut TrieNode, symbol: u16, code: u32, len: u8| {
        let mut node = root;
        for i in (0..len).rev() {
            let bit = ((code >> i) & 1) as usize;
            node = node.children[bit].get_or_insert_with(|| Box::new(TrieNode::new()));
        }
        node.symbol = Some(symbol);
    };
    for (sym, &(code, len)) in CODES.iter().enumerate() {
        insert(&mut root, sym as u16, code, len);
    }
    insert(&mut root, EOS_SYMBOL, EOS.0, EOS.1);
    root
}

fn trie() -> &'static TrieNode {
    static TRIE: OnceLock<TrieNode> = OnceLock::new();
    TRIE.get_or_init(build_trie)
}

/// Huffman-decode a byte string, enforcing RFC 7541 §5.2's padding rules:
/// leftover bits at the end must be fewer than 8 and must equal the
/// high-order bits of the EOS code (i.e. all ones).
pub fn decode(input: &[u8]) -> Result<Vec<u8>> {
    let root = trie();
    let mut out = Vec::new();
    let mut node = root;
    let mut pending = 0u32;
    let mut all_ones = true;

    for &byte in input {
        for i in (0..8).rev() {
            let bit = (byte >> i) & 1;
            let next = node.children[bit as usize].as_deref().ok_or_else(invalid)?;
            pending += 1;
            all_ones &= bit == 1;
            match next.symbol {
                Some(EOS_SYMBOL) => return Err(invalid()),
                Some(sym) => {
                    out.push(sym as u8);
                    node = root;
                    pending = 0;
                    all_ones = true;
                }
                None => node = next,
            }
        }
    }

    if pending >= 8 || !all_ones {
        return Err(invalid());
    }
    Ok(out)
}

fn invalid() -> H2Error {
    H2Error::Connection(
        ErrorCode::CompressionError,
        "invalid Huffman-encoded string",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7541_c4_1_examples() {
        // RFC 7541 C.4.1: "www.example.com" Huffman-encoded.
        let raw = "www.example.com".as_bytes();
        let encoded = encode(raw);
        assert_eq!(encoded, hex("f1e3c2e5f23a6ba0ab90f4ff"));
        assert_eq!(decode(&encoded).unwrap(), raw);
    }

    #[test]
    fn roundtrip_all_bytes() {
        let raw: Vec<u8> = (0..=255).collect();
        let encoded = encode(&raw);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn empty_roundtrip() {
        assert_eq!(encode(&[]), Vec::<u8>::new());
        assert_eq!(decode(&[]).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn explicit_eos_is_rejected() {
        // 30 one-bits packed into 4 bytes (with trailing 1-padding) is a
        // literal encoding of the EOS symbol, which must be rejected.
        let mut bits: u64 = (1u64 << 30) - 1; // 30 ones
        bits <<= 2; // pad final byte with two more 1s -> still all ones, 32 bits total
        bits |= 0b11;
        let bytes = bits.to_be_bytes();
        assert!(decode(&bytes[4..]).is_err());
    }

    #[test]
    fn overlong_padding_is_rejected() {
        // A byte's worth of trailing padding (>7 bits unresolved) is invalid.
        assert!(decode(&[0xff]).is_err() || decode(&[0xff, 0xff]).is_err());
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
}
