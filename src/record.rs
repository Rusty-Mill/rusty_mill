//! On-disk record framing for a segment: `[len: u32 LE][crc32: u32 LE][payload]`.
//!
//! The length/checksum pair is what recovery uses to detect a torn write
//! (ADR-0002 D4, minimal DST test 2): a record whose declared length runs
//! past the end of the file, or whose payload doesn't match its checksum,
//! is incomplete or corrupt and must be truncated away, not served.
//!
//! No `crc32fast`/`crc32c` dependency: this is a small, well-known,
//! table-based algorithm (CRC-32/ISO-HDLC, the same polynomial `zlib`/`gzip`
//! use), and this project's own runtime dependency (`rusty_tokio`, 28 crates
//! against compio's 231 — see ADR-0002 D3) treats every additional
//! dependency as audit surface to justify, not a default. Hand-rolling a
//! documented, standard checksum is cheaper to audit than a new crate.

/// Fixed framing overhead before the payload: 4 bytes length + 4 bytes CRC32.
pub const HEADER_LEN: usize = 8;

const CRC32_POLY: u32 = 0xEDB8_8320;

fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut j = 0;
        while j < 8 {
            c = if c & 1 != 0 {
                CRC32_POLY ^ (c >> 1)
            } else {
                c >> 1
            };
            j += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

/// CRC-32/ISO-HDLC checksum of `data`, matching `zlib::crc32`/`gzip`'s output
/// for the same input — useful if this ever needs cross-checking against an
/// external tool, not just internal consistency.
pub fn crc32(data: &[u8]) -> u32 {
    let table = crc32_table();
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = table[idx] ^ (crc >> 8);
    }
    !crc
}

/// Encodes `payload` as a framed record: `[len][crc32][payload]`.
pub fn encode(payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEADER_LEN + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(&crc32(payload).to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// Why a framed record at `buf[..]` failed to decode — the caller (segment
/// recovery) turns this into "truncate the log tail here", not a hard error;
/// see `crate::segment` and ADR-0002 D4's minimal DST tests.
#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Fewer than [`HEADER_LEN`] bytes available — a torn write cut off
    /// mid-header.
    HeaderTruncated,
    /// The header claims a payload longer than what's actually present — a
    /// torn write cut off mid-payload.
    PayloadTruncated { declared: u32, available: usize },
    /// The full record was present but its checksum doesn't match — silent
    /// corruption, not a truncation; still not safe to serve.
    ChecksumMismatch,
}

/// Decodes one framed record from the start of `buf`. Returns the payload
/// slice and the total number of bytes the record occupied (header +
/// payload), so the caller can advance past it and decode the next one.
pub fn decode(buf: &[u8]) -> Result<(&[u8], usize), DecodeError> {
    if buf.len() < HEADER_LEN {
        return Err(DecodeError::HeaderTruncated);
    }
    let declared_len = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let declared_crc = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    let available = buf.len() - HEADER_LEN;
    if (declared_len as usize) > available {
        return Err(DecodeError::PayloadTruncated {
            declared: declared_len,
            available,
        });
    }
    let payload = &buf[HEADER_LEN..HEADER_LEN + declared_len as usize];
    if crc32(payload) != declared_crc {
        return Err(DecodeError::ChecksumMismatch);
    }
    Ok((payload, HEADER_LEN + declared_len as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_payload() {
        let encoded = encode(b"hello segment log");
        let (payload, len) = decode(&encoded).unwrap();
        assert_eq!(payload, b"hello segment log");
        assert_eq!(len, encoded.len());
    }

    #[test]
    fn empty_payload_round_trips() {
        let encoded = encode(b"");
        let (payload, len) = decode(&encoded).unwrap();
        assert_eq!(payload, b"");
        assert_eq!(len, HEADER_LEN);
    }

    #[test]
    fn header_truncated_is_reported_not_panicked() {
        assert_eq!(decode(&[1, 2, 3]), Err(DecodeError::HeaderTruncated));
        assert_eq!(decode(&[]), Err(DecodeError::HeaderTruncated));
    }

    #[test]
    fn payload_truncated_is_reported_not_panicked() {
        let mut encoded = encode(b"hello segment log");
        encoded.truncate(HEADER_LEN + 3); // cut the payload short
        assert_eq!(
            decode(&encoded),
            Err(DecodeError::PayloadTruncated {
                declared: 17, // "hello segment log" is 17 bytes
                available: 3,
            })
        );
    }

    #[test]
    fn corrupted_payload_is_a_checksum_mismatch() {
        let mut encoded = encode(b"hello segment log");
        let last = encoded.len() - 1;
        encoded[last] ^= 0xFF; // flip a payload byte without touching the header
        assert_eq!(decode(&encoded), Err(DecodeError::ChecksumMismatch));
    }

    #[test]
    fn crc32_matches_known_vector() {
        // Standard CRC-32/ISO-HDLC test vector: CRC32("123456789") == 0xCBF43926.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn multiple_records_decode_in_sequence() {
        let mut buf = encode(b"first");
        buf.extend(encode(b"second"));
        let (first, n1) = decode(&buf).unwrap();
        assert_eq!(first, b"first");
        let (second, _n2) = decode(&buf[n1..]).unwrap();
        assert_eq!(second, b"second");
    }
}
