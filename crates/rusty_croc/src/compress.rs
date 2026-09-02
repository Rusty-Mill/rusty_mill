//! Port of `src/compress` — raw DEFLATE compression.
//!
//! Go uses `compress/flate` with `HuffmanOnly` for protocol messages and a
//! configurable level for file chunks. Any conformant raw-DEFLATE stream is
//! wire-compatible, so this port uses `flate2`'s fastest setting for messages;
//! byte-identical output is not required (and not produced), only mutual
//! decodability, which the interop tests exercise in both directions.

use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use std::io::{Read, Write};

/// Compress with a specific level (0-9), mirroring `CompressWithOption`.
pub fn compress_with_option(src: &[u8], level: u32) -> Vec<u8> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::new(level));
    // Writing to a Vec cannot fail.
    let _ = e.write_all(src);
    e.finish().unwrap_or_default()
}

/// Compress for protocol messages (Go uses HuffmanOnly; we use the fastest
/// standard level, which every DEFLATE decoder can read).
pub fn compress(src: &[u8]) -> Vec<u8> {
    compress_with_option(src, 1)
}

/// Decompress a raw DEFLATE stream. Mirrors Go's behavior of returning
/// whatever could be read on error (croc ignores decompression errors).
pub fn decompress(src: &[u8]) -> Vec<u8> {
    let mut d = DeflateDecoder::new(src);
    let mut out = Vec::new();
    let _ = d.read_to_end(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let data = b"the quick brown fox jumps over the lazy dog".repeat(20);
        let c = compress(&data);
        assert!(c.len() < data.len());
        assert_eq!(decompress(&c), data);
    }

    // Stream produced by croc's Go compress.Compress (flate.HuffmanOnly);
    // proves we can inflate Go's output.
    #[test]
    fn go_flate_vector() {
        let go_compressed = hex::decode(
            "04c0870180300844d155fe6a1634d65304dbf479518c33a76ea1753d3b835ee6dc8e0bdde64431d6e6ffe835d6000000ffff",
        )
        .unwrap();
        assert_eq!(
            decompress(&go_compressed),
            b"the quick brown fox jumps over the lazy dog"
        );
    }

    #[test]
    fn levels() {
        let data = b"aaaaaaaaaabbbbbbbbbbcccccccccc".repeat(10);
        for level in [1, 6, 9] {
            assert_eq!(decompress(&compress_with_option(&data, level)), data);
        }
    }
}
