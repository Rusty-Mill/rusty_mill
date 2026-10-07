//! Hex encode and decode, with no dependencies and no panics.
//!
//! [`encode`] writes lowercase; [`decode`] and [`decode_array`] accept either
//! case. Use [`decode_array`] for keys and digests of a known size. It is
//! the one place the workspace's hex helpers live: `ts-types`, `ts-key`,
//! `rsi-core`, `remind_me_remote`, `rusty_term` and others each used to carry
//! their own.

#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

const ALPHABET: &[u8; 16] = b"0123456789abcdef";

/// Why a hex string did not decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The input has an odd number of digits.
    OddLength {
        /// Length of the input in bytes.
        len: usize,
    },
    /// A byte that is not a hex digit.
    InvalidCharacter {
        /// The offending byte.
        byte: u8,
        /// Its position in the input.
        index: usize,
    },
    /// [`decode_array`] was given a different number of digits than the array
    /// needs.
    WrongLength {
        /// Digits the array needs (twice its size).
        expected: usize,
        /// Length of the input in bytes.
        found: usize,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::OddLength { len } => write!(f, "odd hex length {len}"),
            DecodeError::InvalidCharacter { byte, index } => {
                write!(
                    f,
                    "invalid hex character {:?} at index {index}",
                    char::from(*byte)
                )
            }
            DecodeError::WrongLength { expected, found } => {
                write!(f, "expected {expected} hex digits, found {found}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DecodeError {}

/// Lowercase hex encoding of `data`, two digits per byte.
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    for &byte in data {
        out.push(char::from(ALPHABET[usize::from(byte >> 4)]));
        out.push(char::from(ALPHABET[usize::from(byte & 0x0f)]));
    }
    out
}

/// Decodes an even number of hex digits (either case) into bytes.
pub fn decode(input: impl AsRef<[u8]>) -> Result<Vec<u8>, DecodeError> {
    let input = input.as_ref();
    if input.len() % 2 != 0 {
        return Err(DecodeError::OddLength { len: input.len() });
    }
    let mut out = Vec::with_capacity(input.len() / 2);
    for (pair, index) in input.chunks_exact(2).zip((0..).step_by(2)) {
        out.push((nibble(pair[0], index)? << 4) | nibble(pair[1], index + 1)?);
    }
    Ok(out)
}

/// Decodes exactly `2 * N` hex digits (either case) into an array.
pub fn decode_array<const N: usize>(input: impl AsRef<[u8]>) -> Result<[u8; N], DecodeError> {
    let input = input.as_ref();
    if input.len() != 2 * N {
        return Err(DecodeError::WrongLength {
            expected: 2 * N,
            found: input.len(),
        });
    }
    let mut out = [0u8; N];
    for (slot, (pair, index)) in out
        .iter_mut()
        .zip(input.chunks_exact(2).zip((0..).step_by(2)))
    {
        *slot = (nibble(pair[0], index)? << 4) | nibble(pair[1], index + 1)?;
    }
    Ok(out)
}

fn nibble(byte: u8, index: usize) -> Result<u8, DecodeError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(DecodeError::InvalidCharacter { byte, index }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_lowercase() {
        assert_eq!(encode(&[]), "");
        assert_eq!(encode(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
    }

    #[test]
    fn decodes_either_case() {
        assert_eq!(decode("000fa5ff").unwrap(), [0x00, 0x0f, 0xa5, 0xff]);
        assert_eq!(decode("000FA5FF").unwrap(), [0x00, 0x0f, 0xa5, 0xff]);
        assert_eq!(decode(b"ab".as_slice()).unwrap(), [0xab]);
        assert_eq!(decode("").unwrap(), []);
    }

    #[test]
    fn round_trips_every_byte() {
        let data: Vec<u8> = (0u8..=255).collect();
        assert_eq!(decode(encode(&data)).unwrap(), data);
    }

    #[test]
    fn rejects_odd_length_and_bad_digits() {
        assert_eq!(decode("abc"), Err(DecodeError::OddLength { len: 3 }));
        assert_eq!(
            decode("0g"),
            Err(DecodeError::InvalidCharacter {
                byte: b'g',
                index: 1
            })
        );
        assert_eq!(
            decode("zz00"),
            Err(DecodeError::InvalidCharacter {
                byte: b'z',
                index: 0
            })
        );
        // Multi-byte UTF-8 is an error, never a panic.
        assert!(decode("é0").is_err());
    }

    #[test]
    fn decodes_a_fixed_size_array() {
        let bytes: [u8; 32] = core::array::from_fn(|i| (i * 7 + 3) as u8);
        assert_eq!(decode_array::<32>(encode(&bytes)), Ok(bytes));
        assert_eq!(decode_array::<32>("AB".repeat(32)), Ok([0xab; 32]));
    }

    #[test]
    fn a_fixed_size_array_needs_exactly_its_length() {
        assert_eq!(
            decode_array::<32>(""),
            Err(DecodeError::WrongLength {
                expected: 64,
                found: 0
            })
        );
        assert_eq!(
            decode_array::<32>("0".repeat(63)),
            Err(DecodeError::WrongLength {
                expected: 64,
                found: 63
            })
        );
        assert_eq!(
            decode_array::<32>("0".repeat(65)),
            Err(DecodeError::WrongLength {
                expected: 64,
                found: 65
            })
        );
        assert!(matches!(
            decode_array::<32>("g".repeat(64)),
            Err(DecodeError::InvalidCharacter { index: 0, .. })
        ));
        assert!(decode_array::<32>("é".repeat(32)).is_err());
    }
}
