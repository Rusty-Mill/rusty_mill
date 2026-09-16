//! Sovereign Binary Buffer serialization engine.

use alloc::vec::Vec;

/// Serializes raw byte slices into compact binary format.
///
/// Returns an error if `data` is longer than `u32::MAX` bytes, since the
/// length header is a 4-byte little-endian `u32` and can't represent a
/// larger length without silently truncating it.
pub fn serialize(data: &[u8]) -> Result<Vec<u8>, &'static str> {
    if data.len() > u32::MAX as usize {
        return Err("Payload too large to encode in a u32 length header");
    }
    let mut out = Vec::with_capacity(data.len() + 4);
    let len = data.len() as u32;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(data);
    Ok(out)
}

/// Deserializes compact binary format into raw byte slice.
pub fn deserialize(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    if bytes.len() < 4 {
        return Err("Buffer too short");
    }
    let mut len_bytes = [0u8; 4];
    len_bytes.copy_from_slice(&bytes[..4]);
    let len = u32::from_le_bytes(len_bytes) as usize;

    let end = 4usize.checked_add(len).ok_or("Payload incomplete")?;
    let payload = bytes.get(4..end).ok_or("Payload incomplete")?;
    Ok(payload.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_binary() {
        let original = b"Hello Rusty Mill Sovereign Binary Codec!";
        let encoded = serialize(original).unwrap();
        let decoded = deserialize(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn deserialize_rejects_a_length_header_past_the_end_of_the_buffer_instead_of_panicking() {
        // A 4-byte header claiming a near-`u32::MAX` payload with zero
        // trailing bytes must not overflow `4 + len` or panic on the
        // subsequent slice — it must report a clean error.
        let bytes = [0xFF, 0xFF, 0xFF, 0xFF];
        let result = deserialize(&bytes);
        assert_eq!(result, Err("Payload incomplete"));
    }

    #[test]
    fn serialize_returns_a_result_and_still_roundtrips_normal_sized_data() {
        // Actually allocating a >= 4GiB buffer to exercise the `u32::MAX`
        // length-header boundary directly is impractical in a unit test
        // (it would need multiple gigabytes of memory just to run); this
        // instead pins the new fallible signature contract — `serialize`
        // must return a `Result` rather than silently truncating an
        // oversized length into a `u32` header — and confirms normal-sized
        // data still serializes/deserializes correctly through it.
        let original = b"small payload";
        let encoded: Result<Vec<u8>, &'static str> = serialize(original);
        let bytes = encoded.unwrap();
        assert_eq!(&bytes[..4], &(original.len() as u32).to_le_bytes());
        assert_eq!(deserialize(&bytes).unwrap(), original);
    }
}
