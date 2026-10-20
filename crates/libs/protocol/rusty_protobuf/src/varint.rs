use crate::Error;

/// Longest encoding of a 64-bit value.
pub(crate) const MAX_LEN: usize = 10;

/// The varint at the start of `buf` and how many bytes it took.
pub(crate) fn decode(buf: &[u8]) -> Result<(u64, usize), Error> {
    let mut value = 0u64;
    for (i, &byte) in buf.iter().take(MAX_LEN).enumerate() {
        // The 10th byte holds only bit 63; anything more does not fit.
        if i == MAX_LEN - 1 && byte > 1 {
            return Err(Error::VarintOverflow);
        }
        value |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok((value, i + 1));
        }
    }
    // Every byte continued and the buffer ended (a 10th byte would have returned above).
    Err(Error::Truncated)
}

/// Appends `v` as a varint.
pub(crate) fn encode(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// The zigzag form of an `i32`, as a `sint32` is sent.
pub fn zigzag32(v: i32) -> u32 {
    ((v << 1) ^ (v >> 31)) as u32
}

/// The zigzag form of an `i64`, as a `sint64` is sent.
pub fn zigzag64(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// The `i32` a `sint32` varint stands for.
pub fn unzigzag32(v: u32) -> i32 {
    ((v >> 1) as i32) ^ -((v & 1) as i32)
}

/// The `i64` a `sint64` varint stands for.
pub fn unzigzag64(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        encode(v, &mut out);
        out
    }

    #[test]
    fn encodes_the_documented_examples() {
        assert_eq!(enc(0), [0x00]);
        assert_eq!(enc(1), [0x01]);
        assert_eq!(enc(127), [0x7f]);
        assert_eq!(enc(128), [0x80, 0x01]);
        assert_eq!(enc(150), [0x96, 0x01]);
        assert_eq!(
            enc(u64::MAX),
            [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]
        );
    }

    #[test]
    fn decode_inverts_encode_at_every_length() {
        for shift in 0..64 {
            for v in [(1u64 << shift) - 1, 1u64 << shift, (1u64 << shift) + 1] {
                let bytes = enc(v);
                assert_eq!(decode(&bytes), Ok((v, bytes.len())), "{v}");
            }
        }
        assert_eq!(decode(&enc(u64::MAX)), Ok((u64::MAX, MAX_LEN)));
    }

    #[test]
    fn decode_stops_at_the_first_byte_without_a_continuation_bit() {
        assert_eq!(decode(&[0x96, 0x01, 0xff, 0xff]), Ok((150, 2)));
    }

    #[test]
    fn overlong_encodings_are_accepted() {
        assert_eq!(decode(&[0x81, 0x00]), Ok((1, 2)));
        assert_eq!(
            decode(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00]),
            Ok((0, 10))
        );
    }

    #[test]
    fn decode_rejects_truncated_overflowing_and_overlong_input() {
        assert_eq!(decode(&[]), Err(Error::Truncated));
        assert_eq!(decode(&[0x80]), Err(Error::Truncated));
        assert_eq!(decode(&[0xff; 9]), Err(Error::Truncated));
        // 10th byte may only carry bit 63
        let mut over = [0xff; 10];
        over[9] = 0x02;
        assert_eq!(decode(&over), Err(Error::VarintOverflow));
        // an 11th byte is never read
        assert_eq!(decode(&[0x80; 11]), Err(Error::VarintOverflow));
    }

    #[test]
    fn zigzag_maps_small_magnitudes_to_small_values() {
        for (n, z) in [
            (0i32, 0u32),
            (-1, 1),
            (1, 2),
            (-2, 3),
            (i32::MAX, u32::MAX - 1),
            (i32::MIN, u32::MAX),
        ] {
            assert_eq!(zigzag32(n), z);
            assert_eq!(unzigzag32(z), n);
        }
        for (n, z) in [
            (0i64, 0u64),
            (-1, 1),
            (1, 2),
            (i64::MAX, u64::MAX - 1),
            (i64::MIN, u64::MAX),
        ] {
            assert_eq!(zigzag64(n), z);
            assert_eq!(unzigzag64(z), n);
        }
    }
}
