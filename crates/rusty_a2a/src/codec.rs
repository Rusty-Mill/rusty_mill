//! Byte-string encodings used by the wire format.

/// Base64 (standard alphabet, padded) encoding for `bytes` proto fields,
/// matching ProtoJSON's convention for `bytes` (used by `Part.raw`).
pub mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&rusty_base64::encode_standard(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        rusty_base64::decode_standard(&s).map_err(serde::de::Error::custom)
    }
}
