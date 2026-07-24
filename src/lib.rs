//! A sans-IO stream compression and decompression abstraction crate.
//!
//! Provides clean, safe helper functions for DEFLATE, Gzip, and Zlib streaming formats.

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

impl CompressionLevel {
    fn to_flate2(self) -> flate2::Compression {
        match self {
            CompressionLevel::Fast => flate2::Compression::fast(),
            CompressionLevel::Default => flate2::Compression::default(),
            CompressionLevel::Best => flate2::Compression::best(),
        }
    }
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

/// Compress input slice `data` using raw DEFLATE format.
pub fn compress_deflate(data: &[u8], level: CompressionLevel) -> Vec<u8> {
    use flate2::Compress;
    use flate2::FlushCompress;

    let mut compressor = Compress::new(level.to_flate2(), false);
    let mut output = Vec::with_capacity(data.len() / 2 + 16);
    let mut input_pos = 0;

    loop {
        let old_in = compressor.total_in();
        let old_out = compressor.total_out();

        let mut buf = [0u8; 4096];
        let status = compressor
            .compress(&data[input_pos..], &mut buf, FlushCompress::Finish)
            .expect("deflate compression failed");

        let consumed = (compressor.total_in() - old_in) as usize;
        let produced = (compressor.total_out() - old_out) as usize;

        input_pos += consumed;
        output.extend_from_slice(&buf[..produced]);

        match status {
            flate2::Status::StreamEnd => break,
            flate2::Status::Ok | flate2::Status::BufError => continue,
        }
    }

    output
}

/// Decompress raw DEFLATE data slice `data`.
pub fn decompress_deflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    use flate2::Decompress;
    use flate2::FlushDecompress;

    let mut decompressor = Decompress::new(false);
    let mut output = Vec::with_capacity(data.len() * 2 + 16);
    let mut input_pos = 0;

    loop {
        let old_in = decompressor.total_in();
        let old_out = decompressor.total_out();

        let mut buf = [0u8; 4096];
        let status = decompressor
            .decompress(&data[input_pos..], &mut buf, FlushDecompress::None)
            .map_err(|_| Error::CorruptData)?;

        let consumed = (decompressor.total_in() - old_in) as usize;
        let produced = (decompressor.total_out() - old_out) as usize;

        input_pos += consumed;
        output.extend_from_slice(&buf[..produced]);

        match status {
            flate2::Status::StreamEnd => break,
            flate2::Status::Ok => {
                if consumed == 0 && produced == 0 && input_pos >= data.len() {
                    break;
                }
            }
            flate2::Status::BufError => return Err(Error::TruncatedInput),
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deflate_roundtrips() {
        let original = b"Hello Rusty Mill! Streaming DEFLATE test vector.";
        let compressed = compress_deflate(original, CompressionLevel::Default);
        let decompressed = decompress_deflate(&compressed).unwrap();
        assert_eq!(decompressed, original);
    }
}
