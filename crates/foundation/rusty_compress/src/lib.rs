//! A sans-IO stream compression and decompression abstraction crate.
//!
//! Currently implements only raw stored-block RFC 1951 DEFLATE (non-compressed
//! blocks): [`compress_deflate`] always emits stored blocks and
//! [`decompress_deflate`] only accepts stored blocks, returning
//! [`Error::CorruptData`] for any other DEFLATE block type. Gzip and Zlib
//! wrapper support is not yet implemented.

use core::fmt;

/// Compression level preference.
///
/// This parameter currently has no effect: [`compress_deflate`] always emits
/// raw stored (uncompressed) DEFLATE blocks regardless of the selected level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressionLevel {
    /// Fast compression (level 1). Currently unused.
    Fast,
    /// Default compression (level 6). Currently unused.
    #[default]
    Default,
    /// Best compression ratio (level 9). Currently unused.
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
///
/// `level` is currently ignored; only stored blocks are produced.
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
///
/// Only raw stored-block data (as produced by [`compress_deflate`]) is
/// supported; any other DEFLATE block type (fixed/dynamic Huffman) returns
/// [`Error::CorruptData`]. An input that is empty or ends before a block with
/// the BFINAL bit set has been observed returns [`Error::TruncatedInput`].
pub fn decompress_deflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let mut cursor = 0;
    let mut final_block_seen = false;

    while cursor < data.len() {
        let header = data[cursor];
        cursor += 1;

        let is_last = (header & 0x01) != 0;
        let btype = (header >> 1) & 0x03;

        if btype == 0b00 {
            // Uncompressed (stored) block
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
            // Compressed block types (BTYPE 01/10/11) are not implemented by
            // this crate: only raw stored-block DEFLATE is supported today.
            return Err(Error::CorruptData);
        }

        if is_last {
            final_block_seen = true;
            break;
        }
    }

    if !final_block_seen {
        return Err(Error::TruncatedInput);
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

    #[test]
    fn deflate_roundtrips_empty_input() {
        let compressed = compress_deflate(b"", CompressionLevel::Default);
        let decompressed = decompress_deflate(&compressed).unwrap();
        assert_eq!(decompressed, b"");
    }

    #[test]
    fn decompress_rejects_empty_slice_as_truncated() {
        let result = decompress_deflate(&[]);
        assert_eq!(result, Err(Error::TruncatedInput));
    }

    #[test]
    fn decompress_rejects_non_final_stored_block_with_no_successor() {
        // A single non-final (BFINAL=0), empty stored block with no following
        // block: the stream ends without ever setting BFINAL, so this must be
        // treated as truncated input rather than a successful empty decode.
        let truncated = [0x00, 0x00, 0x00, 0xff, 0xff];
        let result = decompress_deflate(&truncated);
        assert_eq!(result, Err(Error::TruncatedInput));
    }

    #[test]
    fn decompress_distinguishes_unsupported_block_type_from_truncated_input() {
        // BFINAL=0, BTYPE=01 (fixed Huffman compressed block): unsupported by
        // this crate, and must be reported distinctly from truncated input.
        let compressed_block_type = [0x02];
        let result = decompress_deflate(&compressed_block_type);
        assert_eq!(result, Err(Error::CorruptData));
        assert_ne!(result, Err(Error::TruncatedInput));
    }
}
