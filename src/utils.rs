//! Port of the parts of `src/utils` needed so far.
//!
//! The Go package is a grab-bag (hashing, filesystem walking, IP discovery,
//! progress helpers); pieces are ported as the modules that need them arrive.

use crate::mnemonicode;
use md5::{Digest, Md5};
use rand::Rng;
use std::io::Read;
use std::path::Path;

/// Matches Go's `utils.NbPinNumbers`.
pub const NB_PIN_NUMBERS: usize = 4;
/// Matches Go's `utils.NbBytesWords`.
pub const NB_BYTES_WORDS: usize = 4;

/// Random numeric pin, mirroring `utils.GenerateRandomPin`
/// (Go draws each digit from `[0, 9)`).
pub fn generate_random_pin() -> String {
    let mut rng = rand::thread_rng();
    (0..NB_PIN_NUMBERS)
        .map(|_| char::from(b'0' + rng.gen_range(0..9u8)))
        .collect()
}

/// Random code phrase like `1234-quiet-lion-daisy`, mirroring
/// `utils.GetRandomName`.
pub fn get_random_name() -> String {
    let mut bs = [0u8; NB_BYTES_WORDS];
    rand::thread_rng().fill(&mut bs);
    let words = mnemonicode::encode_word_list(&bs);
    format!("{}-{}", generate_random_pin(), words.join("-"))
}

/// Hash a file with the named algorithm, mirroring `utils.HashFile`.
/// `xxhash` (croc's default) produces the 8-byte big-endian XXH64 sum, like
/// Go's `cespare/xxhash` `Sum(nil)`. `imohash`/`highway` are not yet ported.
pub fn hash_file(path: &Path, algorithm: &str) -> std::io::Result<Vec<u8>> {
    match algorithm {
        "xxhash" => {
            let mut hasher = xxhash_rust::xxh64::Xxh64::new(0);
            let mut f = std::fs::File::open(path)?;
            let mut buf = vec![0u8; 1 << 16];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hasher.digest().to_be_bytes().to_vec())
        }
        "md5" => {
            let mut hasher = Md5::new();
            let mut f = std::fs::File::open(path)?;
            let mut buf = vec![0u8; 1 << 16];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hasher.finalize().to_vec())
        }
        other => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("hash algorithm '{other}' not yet ported (use xxhash or md5)"),
        )),
    }
}

/// Find all-zero chunks of an on-disk file, encoded as croc chunk ranges:
/// `[chunkSize, start1, count1, start2, count2, ...]`. Mirrors
/// `utils.MissingChunks` (used to resume partially transferred files).
pub fn missing_chunks(path: &Path, fsize: i64, chunk_size: usize) -> Vec<i64> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    match f.metadata() {
        Ok(m) if m.len() as i64 == fsize => {}
        _ => return Vec::new(),
    }

    let empty = vec![0u8; chunk_size];
    let mut chunks: Vec<i64> = Vec::new();
    let mut current_location: i64 = 0;
    let mut buffer = vec![0u8; chunk_size];
    loop {
        // Mirror Go's f.Read: one read per chunk, stopping at EOF/short file.
        let n = match f.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if buffer[..n] == empty[..n] {
            chunks.push(current_location);
        }
        current_location += n as i64;
    }

    if chunks.is_empty() {
        return Vec::new();
    }
    let mut ranges = vec![chunk_size as i64, chunks[0]];
    let mut cur_count = 0i64;
    for i in 1..chunks.len() {
        cur_count += 1;
        if chunks[i] - chunks[i - 1] > chunk_size as i64 {
            ranges.push(cur_count);
            ranges.push(chunks[i]);
            cur_count = 0;
        }
    }
    ranges.push(cur_count + 1);
    ranges
}

/// Expand chunk ranges back into chunk start positions. Mirrors
/// `utils.ChunkRangesToChunks`.
pub fn chunk_ranges_to_chunks(chunk_ranges: &[i64]) -> Vec<i64> {
    if chunk_ranges.is_empty() {
        return Vec::new();
    }
    let chunk_size = chunk_ranges[0];
    let mut chunks = Vec::new();
    let mut i = 1;
    while i + 1 < chunk_ranges.len() {
        for j in 0..chunk_ranges[i + 1] {
            chunks.push(chunk_ranges[i] + j * chunk_size);
        }
        i += 2;
    }
    chunks
}

/// Human-readable byte count, mirroring `utils.ByteCountDecimal`
/// (which despite its name uses 1024 units).
pub fn byte_count_decimal(b: i64) -> String {
    const UNIT: i64 = 1024;
    if b < UNIT {
        return format!("{b} B");
    }
    let mut div = UNIT;
    let mut exp = 0;
    let mut n = b / UNIT;
    while n >= UNIT {
        div *= UNIT;
        exp += 1;
        n /= UNIT;
    }
    format!("{:.1} {}B", b as f64 / div as f64, ['k', 'M', 'G', 'T', 'P', 'E'][exp])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_shape() {
        let pin = generate_random_pin();
        assert_eq!(pin.len(), NB_PIN_NUMBERS);
        assert!(pin.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn name_shape() {
        let name = get_random_name();
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 1 + mnemonicode::words_required(NB_BYTES_WORDS));
    }

    // Vectors from Go: utils.XXHashFile / utils.MD5HashFile of
    // "the quick brown fox jumps over the lazy dog".
    #[test]
    fn hash_vectors_match_go() {
        let dir = std::env::temp_dir().join("rusty-croc-hash-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("xxtest.bin");
        std::fs::write(&p, b"the quick brown fox jumps over the lazy dog").unwrap();
        assert_eq!(
            hex::encode(hash_file(&p, "xxhash").unwrap()),
            "ed714233c5a9a792"
        );
        assert_eq!(
            hex::encode(hash_file(&p, "md5").unwrap()),
            "77add1d5f41223d5582fca736a5cb335"
        );
    }

    #[test]
    fn chunk_ranges_round_trip() {
        // ranges: chunkSize=4, runs starting at 0 (2 chunks) and 16 (1 chunk)
        let ranges = vec![4, 0, 2, 16, 1];
        assert_eq!(chunk_ranges_to_chunks(&ranges), vec![0, 4, 16]);
        assert_eq!(chunk_ranges_to_chunks(&[]), Vec::<i64>::new());
    }

    #[test]
    fn missing_chunks_detects_zero_runs() {
        let dir = std::env::temp_dir().join("rusty-croc-chunk-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("partial.bin");
        // 4 chunks of 4 bytes: data, zeros, zeros, data
        let mut content = Vec::new();
        content.extend_from_slice(b"aaaa");
        content.extend_from_slice(&[0u8; 8]);
        content.extend_from_slice(b"bbbb");
        std::fs::write(&p, &content).unwrap();
        let ranges = missing_chunks(&p, 16, 4);
        assert_eq!(chunk_ranges_to_chunks(&ranges), vec![4, 8]);
    }
}
