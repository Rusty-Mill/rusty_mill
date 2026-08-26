//! zlib (RFC 1950) framing around `rusty_compress`'s DEFLATE, for git's
//! loose-object storage format.
//!
//! **Read/write asymmetry, stated plainly:** [`compress`] produces a real,
//! valid zlib stream — real `git`/`zlib` can decompress it, verified in this
//! crate's interop tests by shelling out to the system `git cat-file`.
//! [`decompress`] can only read back zlib streams whose DEFLATE payload is
//! in "stored" (uncompressed) blocks — which is all [`compress`] ever
//! produces, but is *not* true of an arbitrary object in a real git
//! repository written by real git, which uses actual Huffman-coded blocks
//! `rusty_compress` doesn't implement yet. This crate can therefore write
//! objects real git can read, but cannot yet read arbitrary objects a real
//! git repository wrote.

use rusty_compress::{compress_deflate, decompress_deflate, CompressionLevel};

/// Errors from unwrapping a zlib stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZlibError {
    /// Stream is shorter than the minimum 2-byte header + 4-byte trailer.
    Truncated,
    /// The 2-byte `CMF`/`FLG` header failed the mod-31 check or isn't
    /// DEFLATE (`CM != 8`).
    BadHeader,
    /// The DEFLATE payload itself is corrupt or uses a block type this
    /// crate's decompressor doesn't support (see the module doc).
    BadDeflate,
    /// The trailing Adler-32 checksum didn't match the decompressed data.
    ChecksumMismatch,
}

impl core::fmt::Display for ZlibError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ZlibError::Truncated => write!(f, "zlib stream truncated"),
            ZlibError::BadHeader => write!(f, "zlib header invalid or not DEFLATE"),
            ZlibError::BadDeflate => write!(f, "DEFLATE payload corrupt or uses an unsupported block type"),
            ZlibError::ChecksumMismatch => write!(f, "zlib Adler-32 checksum mismatch"),
        }
    }
}

impl std::error::Error for ZlibError {}

/// Adler-32 checksum (RFC 1950 §9).
fn adler32(data: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % MOD_ADLER;
        b = (b + a) % MOD_ADLER;
    }
    (b << 16) | a
}

/// Compresses `data` into a real, spec-valid zlib stream (2-byte header +
/// DEFLATE stored-block payload + 4-byte big-endian Adler-32 trailer).
pub fn compress(data: &[u8]) -> Vec<u8> {
    let deflate = compress_deflate(data, CompressionLevel::Fast);
    let mut out = Vec::with_capacity(2 + deflate.len() + 4);
    // CMF = 0x78 (CM=8 DEFLATE, CINFO=7 -> 32K window); FLG = 0x01, chosen
    // so (CMF*256 + FLG) % 31 == 0 as RFC 1950 requires (FLEVEL=0/fastest,
    // FDICT=0 -- matches the well-known "fastest" zlib header).
    out.push(0x78);
    out.push(0x01);
    out.extend_from_slice(&deflate);
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Decompresses a zlib stream produced by [`compress`] (see the module doc
/// for why this can't read arbitrary real-git-written objects yet).
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, ZlibError> {
    if data.len() < 6 {
        return Err(ZlibError::Truncated);
    }
    let cmf = data[0];
    let flg = data[1];
    if (cmf & 0x0F) != 8 {
        return Err(ZlibError::BadHeader);
    }
    if (u16::from(cmf) * 256 + u16::from(flg)) % 31 != 0 {
        return Err(ZlibError::BadHeader);
    }

    let deflate = &data[2..data.len() - 4];
    let trailer = &data[data.len() - 4..];
    let expected_adler = u32::from_be_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);

    let decompressed = decompress_deflate(deflate).map_err(|_| ZlibError::BadDeflate)?;
    if adler32(&decompressed) != expected_adler {
        return Err(ZlibError::ChecksumMismatch);
    }
    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_then_decompress_round_trips() {
        let original = b"tree deadbeef\nparent cafef00d\nauthor A <a@b.c> 0 +0000\n\nmsg\n";
        let z = compress(original);
        assert_eq!(&z[..2], &[0x78, 0x01]);
        let back = decompress(&z).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn empty_input_round_trips() {
        let z = compress(b"");
        let back = decompress(&z).unwrap();
        assert_eq!(back, b"");
    }

    #[test]
    fn corrupted_checksum_is_rejected() {
        let mut z = compress(b"hello");
        let last = z.len() - 1;
        z[last] ^= 0xFF;
        assert_eq!(decompress(&z), Err(ZlibError::ChecksumMismatch));
    }

    #[test]
    fn bad_header_is_rejected() {
        let mut z = compress(b"hello");
        z[0] = 0x00;
        assert_eq!(decompress(&z), Err(ZlibError::BadHeader));
    }
}
