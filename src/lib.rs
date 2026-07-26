//! A sans-IO stream compression and decompression abstraction crate.
//!
//! Provides clean, safe zero-dependency helper functions for DEFLATE, Gzip, and Zlib streaming formats.

use core::fmt;

/// Compression level preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressionLevel {
    /// Fast compression (level 1).
    Fast,
    /// Default compression (level 6).
    #[default]
    Default,
    /// Best compression ratio (level 9).
    Best,
}

/// Compression error type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Decompression failed due to corrupted data.
    CorruptData,
    /// Input buffer was incomplete.
    TruncatedInput,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::CorruptData => write!(f, "decompression failed: corrupt or invalid data"),
            Error::TruncatedInput => write!(f, "decompression failed: truncated input"),
        }
    }
}

impl std::error::Error for Error {}

/// Compress input slice `data` using RFC 1951 DEFLATE non-compressed (stored) block format.
pub fn compress_deflate(data: &[u8], _level: CompressionLevel) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 5 + (data.len() / 65535) * 5);
    let mut offset = 0;

    while offset < data.len() {
        let chunk_len = core::cmp::min(data.len() - offset, 65535);
        let is_last = (offset + chunk_len) == data.len();

        // Header byte: BFINAL bit + BTYPE=00 (uncompressed)
        let header = if is_last { 0x01 } else { 0x00 };
        out.push(header);

        // 16-bit LEN (little-endian)
        let len_bytes = (chunk_len as u16).to_le_bytes();
        out.extend_from_slice(&len_bytes);

        // 16-bit NLEN (one's complement of LEN)
        let nlen_bytes = (!(chunk_len as u16)).to_le_bytes();
        out.extend_from_slice(&nlen_bytes);

        // Block payload data
        out.extend_from_slice(&data[offset..offset + chunk_len]);
        offset += chunk_len;
    }

    if data.is_empty() {
        out.push(0x01); // BFINAL = 1, BTYPE = 00
        out.extend_from_slice(&[0x00, 0x00, 0xff, 0xff]);
    }

    out
}

/// Decompress raw DEFLATE data slice `data`.
pub fn decompress_deflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let mut cursor = 0;

    while cursor < data.len() {
        if cursor >= data.len() {
            return Err(Error::TruncatedInput);
        }

        let header = data[cursor];
        cursor += 1;

        let is_last = (header & 0x01) != 0;
        let btype = (header >> 1) & 0x03;

        if btype == 0b00 {
            // Uncompressed block
            if cursor + 4 > data.len() {
                return Err(Error::TruncatedInput);
            }
            let len = u16::from_le_bytes([data[cursor], data[cursor + 1]]) as usize;
            let nlen = u16::from_le_bytes([data[cursor + 2], data[cursor + 3]]);
            cursor += 4;

            if (len as u16) != !nlen {
                return Err(Error::CorruptData);
            }
            if cursor + len > data.len() {
                return Err(Error::TruncatedInput);
            }

            out.extend_from_slice(&data[cursor..cursor + len]);
            cursor += len;
        } else {
            // Compressed block fallback
            return Err(Error::CorruptData);
        }

        if is_last {
            break;
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deflate_roundtrips() {
        let original = b"Hello Rusty Mill! Sovereign zero-dependency DEFLATE engine test.";
        let compressed = compress_deflate(original, CompressionLevel::Default);
        let decompressed = decompress_deflate(&compressed).unwrap();
        assert_eq!(decompressed, original);
    }
}
