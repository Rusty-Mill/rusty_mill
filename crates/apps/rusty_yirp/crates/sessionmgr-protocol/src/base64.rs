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
//! # Why not the `base64` crate
//!
//! It is forty lines of well-understood table lookup, against a
//! dependency this project would otherwise carry forever. That matches
//! the minimal-dependency line the sibling projects hold, and unlike most
//! hand-rolled crypto-adjacent code, base64 has no security-sensitive
//! failure mode: it is an encoding, not a cipher. This module used to be
//! that hand-rolled copy; the encoding itself now comes from
//! `rusty_base64`, the workspace's one dependency-free implementation,
//! and what stays here is this wire format's own strictness rule and the
//! serde glue.

use serde::{Deserialize, Deserializer, Serializer};

pub use rusty_base64::DecodeError;

/// Standard base64 with `=` padding (RFC 4648 §4).
pub fn encode(input: &[u8]) -> String {
    rusty_base64::encode_standard(input)
}

/// Decodes, rejecting anything malformed rather than guessing.
///
/// A peer that sends a corrupt payload is a bug or an attack; silently
/// decoding it to *something* would push the corruption downstream into
/// a terminal, which interprets what it is given.
///
/// Stricter than `rusty_base64::decode_standard` in one respect: that
/// decoder tolerates missing padding (base64url consumers need it), but
/// [`encode`] always pads, so on this wire a length that is not a
/// multiple of 4 can only be a truncated or corrupt message.
pub fn decode(input: &str) -> Result<Vec<u8>, DecodeError> {
    if !input.len().is_multiple_of(4) {
        return Err(DecodeError::InvalidLength { len: input.len() });
    }
    rusty_base64::decode_standard(input)
}

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
        // Unpadded: fine for base64url elsewhere, truncation here.
        assert_eq!(decode("Zm9"), Err(DecodeError::InvalidLength { len: 3 }));
        assert!(matches!(
            decode("Zm9*"),
            Err(DecodeError::InvalidCharacter { byte: b'*', .. })
        ));
        // Padding in the middle of a group.
        assert_eq!(
            decode("Z=9v"),
            Err(DecodeError::MisplacedPadding { index: 1 })
        );
        assert_eq!(
            decode("Zm=v"),
            Err(DecodeError::MisplacedPadding { index: 2 })
        );
    }
}
