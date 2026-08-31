//! Sovereign Binary Buffer serialization engine.

use alloc::vec::Vec;

/// Serializes raw byte slices into compact binary format.
pub fn serialize(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 4);
    let len = data.len() as u32;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// Deserializes compact binary format into raw byte slice.
pub fn deserialize(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    if bytes.len() < 4 {
        return Err("Buffer too short");
    }
    let mut len_bytes = [0u8; 4];
    len_bytes.copy_from_slice(&bytes[..4]);
    let len = u32::from_le_bytes(len_bytes) as usize;

    if bytes.len() < 4 + len {
        return Err("Payload incomplete");
    }
    Ok(bytes[4..4 + len].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_binary() {
        let original = b"Hello Rusty Mill Sovereign Binary Codec!";
        let encoded = serialize(original);
        let decoded = deserialize(&encoded).unwrap();
        assert_eq!(decoded, original);
    }
}
