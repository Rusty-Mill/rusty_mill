//! Base64 for the wire, and the serde glue that puts byte payloads
//! through it.
//!
//! # Why this exists at all
//!
//! Session output became `Vec<u8>` when the Phase 1 PTY spike showed a
//! real terminal is mandatory (see `docs/decisions/0002-*`), and PTY
//! output is not text: it carries ANSI and cursor-positioning sequences,
//! and a read can split a multi-byte character across a chunk boundary.
//!
//! The transport is line-delimited JSON, which has no byte type. Serde's
//! default for `Vec<u8>` is an array of numbers — `[104,105]` — which is
//! roughly **4 bytes of JSON per byte of output** and unreadable. Base64
//! costs 1.33x and stays on one line, which the framing requires.
//!
//! # Why hand-rolled
//!
//! It is forty lines of well-understood table lookup with exhaustive
//! tests, against a dependency this project would otherwise carry
//! forever. That matches the minimal-dependency line the sibling projects
//! hold, and unlike most hand-rolled crypto-adjacent code, base64 has no
//! security-sensitive failure mode: it is an encoding, not a cipher.

use serde::{Deserialize, Deserializer, Serializer};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const PAD: u8 = b'=';

pub fn encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3f] as char);
        // The last group is padded rather than truncated, so the encoded
        // length always reveals the decoded length.
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 0x3f] as char
        } else {
            PAD as char
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 0x3f] as char
        } else {
            PAD as char
        });
    }
    out
}

/// Decodes, rejecting anything malformed rather than guessing.
///
/// A peer that sends a corrupt payload is a bug or an attack; silently
/// decoding it to *something* would push the corruption downstream into
/// a terminal, which interprets what it is given.
pub fn decode(input: &str) -> Result<Vec<u8>, DecodeError> {
    let bytes = input.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err(DecodeError::BadLength { got: bytes.len() });
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for group in bytes.chunks(4) {
        let mut value = 0u32;
        let mut real = 0usize;
        for (index, byte) in group.iter().enumerate() {
            if *byte == PAD {
                // Padding is only ever the last one or two characters.
                if index < 2 || group[index..].iter().any(|b| *b != PAD) {
                    return Err(DecodeError::MisplacedPadding);
                }
                value <<= 6;
                continue;
            }
            let Some(position) = ALPHABET.iter().position(|c| c == byte) else {
                return Err(DecodeError::BadCharacter { got: *byte as char });
            };
            value = (value << 6) | position as u32;
            real += 1;
        }
        // Every group accumulates a full 24 bits (padding shifts in
        // zeroes), so the decoded bytes are always the **high** bytes,
        // taken most-significant first: 2 encoded characters carry 1
        // byte, 3 carry 2, 4 carry 3.
        //
        // Taking the low bytes instead is the easy mistake, and it is
        // invisible on any input whose length is a multiple of 3 --
        // which is most casual test data. The round-trip test covers all
        // three remainders precisely because of that.
        for index in 0..real.saturating_sub(1) {
            out.push((value >> (16 - 8 * index)) as u8);
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    BadLength { got: usize },
    BadCharacter { got: char },
    MisplacedPadding,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::BadLength { got } => {
                write!(f, "base64 length {got} is not a multiple of 4")
            }
            DecodeError::BadCharacter { got } => write!(f, "invalid base64 character `{got}`"),
            DecodeError::MisplacedPadding => f.write_str("base64 padding is misplaced"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// `#[serde(with = "crate::base64::bytes")]` on any `Vec<u8>` field.
pub mod bytes {
    use super::*;

    pub fn serialize<S: Serializer>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&encode(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        decode(&encoded).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_length_remainder() {
        // The three chunk cases (0, 1, 2 bytes left over) are where an
        // encoder gets it wrong, so cover all of them plus empty.
        for input in [
            &b""[..],
            &b"f"[..],
            &b"fo"[..],
            &b"foo"[..],
            &b"foob"[..],
            &b"fooba"[..],
            &b"foobar"[..],
        ] {
            let encoded = encode(input);
            assert_eq!(
                decode(&encoded).expect("own output must decode"),
                input,
                "round trip failed for {input:?} (encoded {encoded})"
            );
        }
    }

    #[test]
    fn matches_the_rfc_4648_vectors() {
        // Checked against the published vectors rather than only against
        // this decoder, which would happily agree with its own bugs.
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn round_trips_arbitrary_binary_including_control_bytes() {
        // The real payload: terminal output full of escape sequences and
        // every byte value, which is exactly why this stopped being a
        // String.
        let all_bytes: Vec<u8> = (0..=255u8).collect();
        assert_eq!(decode(&encode(&all_bytes)).expect("decode"), all_bytes);

        let ansi = b"\x1b[31mred\x1b[0m\r\n\x1b[2J";
        assert_eq!(decode(&encode(ansi)).expect("decode"), ansi);
    }

    #[test]
    fn encoded_output_never_contains_a_newline() {
        // Load-bearing for the line-delimited framing: a payload with a
        // newline in it would be read as two messages.
        let encoded = encode(&(0..=255u8).collect::<Vec<u8>>());
        assert!(!encoded.contains('\n'));
        assert!(!encoded.contains('\r'));
    }

    #[test]
    fn malformed_input_is_rejected_rather_than_guessed_at() {
        assert_eq!(decode("Zm9"), Err(DecodeError::BadLength { got: 3 }));
        assert!(matches!(
            decode("Zm9*"),
            Err(DecodeError::BadCharacter { got: '*' })
        ));
        // Padding in the middle of a group.
        assert_eq!(decode("Z=9v"), Err(DecodeError::MisplacedPadding));
        assert_eq!(decode("Zm=v"), Err(DecodeError::MisplacedPadding));
    }
}
