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

/// Maximum size of a decompressed message. Mirrors `comm::MAX_READ_MESSAGE_SIZE`,
/// the cap already applied to the raw (compressed) wire-frame size, since no
/// legitimate control message or file chunk is expected to expand anywhere
/// near it. Guards against decompression-bomb inputs: `message::decode` runs
/// this on every control-channel frame, including the very first one
/// received before the PAKE key is set, so it is reachable pre-authentication.
pub const MAX_DECOMPRESSED_SIZE: usize = crate::comm::MAX_READ_MESSAGE_SIZE as usize;

/// Decompress a raw DEFLATE stream. Mirrors Go's behavior of returning
/// whatever could be read on error (croc ignores decompression errors), but
/// aborts with an error once output would exceed [`MAX_DECOMPRESSED_SIZE`]
/// rather than growing the output buffer without bound.
pub fn decompress(src: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let d = DeflateDecoder::new(src);
    let mut limited = d.take(MAX_DECOMPRESSED_SIZE as u64 + 1);
    let mut out = Vec::new();
    let _ = limited.read_to_end(&mut out);
    if out.len() > MAX_DECOMPRESSED_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("decompressed message exceeds {MAX_DECOMPRESSED_SIZE} bytes"),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let data = b"the quick brown fox jumps over the lazy dog".repeat(20);
        let c = compress(&data);
        assert!(c.len() < data.len());
        assert_eq!(decompress(&c).unwrap(), data);
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
            decompress(&go_compressed).unwrap(),
            b"the quick brown fox jumps over the lazy dog"
        );
    }

    #[test]
    fn levels() {
        let data = b"aaaaaaaaaabbbbbbbbbbcccccccccc".repeat(10);
        for level in [1, 6, 9] {
            assert_eq!(
                decompress(&compress_with_option(&data, level)).unwrap(),
                data
            );
        }
    }

    // A classic zip-bomb shape: highly repetitive plaintext compresses to a
    // tiny DEFLATE stream but would inflate to far more than
    // `MAX_DECOMPRESSED_SIZE`. `decompress` must abort once the cap is
    // exceeded instead of growing its output buffer without bound — this
    // path is reachable pre-authentication via `message::decode` on the
    // first control-channel frame (see `croc.rs::transfer_loop_inner`).
    #[test]
    fn decompress_rejects_bomb() {
        let bomb_size = MAX_DECOMPRESSED_SIZE + 16 * 1024 * 1024;
        let data = vec![0u8; bomb_size];
        let compressed = compress_with_option(&data, 9);
        assert!(
            compressed.len() < 1024 * 1024,
            "test input did not compress to a tiny stream: {} bytes",
            compressed.len()
        );
        let err = decompress(&compressed).expect_err("bomb must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
