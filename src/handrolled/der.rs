//! A strict DER reader.
//!
//! X.509 certificates are DER, and DER is a *canonical* encoding: for any
//! given value there is exactly one valid byte sequence. That property is
//! the entire reason this reader is as fussy as it is. A parser that accepts
//! non-canonical encodings accepts several byte sequences for the same
//! certificate, and "several byte sequences that mean the same thing" is the
//! shape of a whole family of real CVEs — a signature computed over one
//! encoding, a policy decision made from another.
//!
//! So this module rejects, rather than tolerates:
//!
//! | Rejected | Why |
//! | --- | --- |
//! | indefinite-length encoding (`0x80`) | BER, not DER; there is no length to check against |
//! | non-minimal length (long form for < 128, or leading zero length octets) | two encodings of one length |
//! | high-tag-number form (`tag & 0x1f == 0x1f`) | X.509 needs no tag above 30, and the multi-byte form is unbounded |
//! | non-minimal `INTEGER` (a leading `0x00` that is not a sign byte) | two encodings of one number |
//! | negative `INTEGER` where the field is unsigned | a negative serial number or path length is nonsense the caller should not have to think about |
//! | `BOOLEAN` other than `0x00`/`0xff` | BER permits any non-zero as true; DER does not |
//! | trailing data after a value | a second certificate hiding behind the first |
//!
//! Being strict is not free: a certificate a lenient parser would accept may
//! be rejected here. That is the intended trade. This code is for deciding
//! whether to trust a peer, and "I could not make sense of this" is always a
//! safe answer to that question, where "I made a guess" is not.
//!
//! # No recursion
//!
//! Nothing in this module or its callers recurses on input structure.
//! [`Reader::read_sequence`] returns a *sub-reader* over the contents, which
//! callers drive with a loop, so nesting depth in the input cannot become
//! stack depth in the parser. A hostile certificate with ten thousand nested
//! `SEQUENCE`s costs time proportional to its size and nothing else — there
//! is no depth limit here because there is no depth to limit.

use core::fmt;

/// An ASN.1 tag octet, restricted to the low-tag-number form.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tag(pub u8);

impl Tag {
    /// `BOOLEAN`.
    pub const BOOLEAN: Self = Self(0x01);
    /// `INTEGER`.
    pub const INTEGER: Self = Self(0x02);
    /// `BIT STRING`.
    pub const BIT_STRING: Self = Self(0x03);
    /// `OCTET STRING`.
    pub const OCTET_STRING: Self = Self(0x04);
    /// `NULL`.
    pub const NULL: Self = Self(0x05);
    /// `OBJECT IDENTIFIER`.
    pub const OID: Self = Self(0x06);
    /// `UTF8String`.
    pub const UTF8_STRING: Self = Self(0x0c);
    /// `PrintableString`.
    pub const PRINTABLE_STRING: Self = Self(0x13);
    /// `IA5String`.
    pub const IA5_STRING: Self = Self(0x16);
    /// `UTCTime`.
    pub const UTC_TIME: Self = Self(0x17);
    /// `GeneralizedTime`.
    pub const GENERALIZED_TIME: Self = Self(0x18);
    /// `SEQUENCE`, always constructed.
    pub const SEQUENCE: Self = Self(0x30);
    /// `SET`, always constructed.
    pub const SET: Self = Self(0x31);

    /// A context-specific tag, e.g. `[0]` in a certificate's `version` field.
    ///
    /// `constructed` distinguishes `[0] { ... }` (an explicit wrapper around
    /// another value) from `[0] <bytes>` (an implicit re-tagging of one).
    pub const fn context(number: u8, constructed: bool) -> Self {
        Self(0x80 | if constructed { 0x20 } else { 0 } | number)
    }

    /// True if bit 6 is set, meaning the contents are themselves TLVs.
    pub const fn is_constructed(self) -> bool {
        self.0 & 0x20 != 0
    }
}

impl fmt::Debug for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match *self {
            Self::BOOLEAN => "BOOLEAN",
            Self::INTEGER => "INTEGER",
            Self::BIT_STRING => "BIT STRING",
            Self::OCTET_STRING => "OCTET STRING",
            Self::NULL => "NULL",
            Self::OID => "OBJECT IDENTIFIER",
            Self::UTF8_STRING => "UTF8String",
            Self::PRINTABLE_STRING => "PrintableString",
            Self::IA5_STRING => "IA5String",
            Self::UTC_TIME => "UTCTime",
            Self::GENERALIZED_TIME => "GeneralizedTime",
            Self::SEQUENCE => "SEQUENCE",
            Self::SET => "SET",
            _ => return write!(f, "Tag(0x{:02x})", self.0),
        };
        f.write_str(name)
    }
}

/// Everything the reader can refuse.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DerError {
    /// The input ended in the middle of a value.
    UnexpectedEnd,
    /// A value had a tag other than the one required here.
    UnexpectedTag {
        /// What the grammar called for.
        expected: Tag,
        /// The tag octet actually present.
        found: u8,
    },
    /// A tag used the multi-byte high-tag-number form, which X.509 never
    /// needs and which has no bound on its length.
    HighTagNumberForm,
    /// A length used BER's indefinite form (`0x80`), which DER forbids.
    IndefiniteLength,
    /// A length was encoded in more octets than necessary.
    NonMinimalLength,
    /// A length did not fit in a `usize`, on a platform where that matters.
    LengthTooLarge,
    /// Bytes remained after the value that was supposed to be the last one.
    TrailingData {
        /// How many bytes were left over.
        remaining: usize,
    },
    /// An `INTEGER` had a redundant leading `0x00` or `0xff` octet.
    NonMinimalInteger,
    /// An `INTEGER` was negative where the field is unsigned.
    NegativeInteger,
    /// An `INTEGER` was too large for the type it was being read into.
    IntegerTooLarge,
    /// An `INTEGER` had no content octets at all.
    EmptyInteger,
    /// A `BOOLEAN` was neither `0x00` nor `0xff`.
    NonCanonicalBoolean(u8),
    /// A `BIT STRING` was empty, or claimed an impossible number of unused
    /// bits, or claimed unused bits with no content.
    MalformedBitString,
    /// An `OBJECT IDENTIFIER` was empty, or had a non-minimal or unterminated
    /// subidentifier.
    MalformedOid,
    /// A string was not valid for its ASN.1 type.
    MalformedString,
}

impl fmt::Display for DerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd => f.write_str("input ended mid-value"),
            Self::UnexpectedTag { expected, found } => {
                write!(f, "expected {expected:?}, found tag 0x{found:02x}")
            }
            Self::HighTagNumberForm => f.write_str("high-tag-number form is not allowed"),
            Self::IndefiniteLength => f.write_str("indefinite-length encoding is not DER"),
            Self::NonMinimalLength => f.write_str("length is not minimally encoded"),
            Self::LengthTooLarge => f.write_str("length does not fit in this platform's usize"),
            Self::TrailingData { remaining } => {
                write!(f, "{remaining} trailing bytes after the final value")
            }
            Self::NonMinimalInteger => f.write_str("INTEGER is not minimally encoded"),
            Self::NegativeInteger => f.write_str("INTEGER is negative where unsigned is required"),
            Self::IntegerTooLarge => f.write_str("INTEGER does not fit in the requested type"),
            Self::EmptyInteger => f.write_str("INTEGER has no content octets"),
            Self::NonCanonicalBoolean(v) => {
                write!(f, "BOOLEAN octet 0x{v:02x} is neither 0x00 nor 0xff")
            }
            Self::MalformedBitString => f.write_str("malformed BIT STRING"),
            Self::MalformedOid => f.write_str("malformed OBJECT IDENTIFIER"),
            Self::MalformedString => f.write_str("string is invalid for its ASN.1 type"),
        }
    }
}

impl std::error::Error for DerError {}

type Result<T> = core::result::Result<T, DerError>;

/// One value, kept as both its contents and the bytes it was encoded from.
///
/// The full encoding matters more often than it looks like it should: a
/// certificate's signature is computed over the encoded `tbsCertificate`, and
/// RFC 5280 §7.1 name chaining compares encoded `Name`s. Re-encoding a parsed
/// value to recover those bytes is how implementations end up with two
/// disagreeing encodings of the same structure, so this reader never does —
/// it keeps a borrow of the original.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Value<'a> {
    /// The tag this value carried.
    pub tag: Tag,
    /// The contents, excluding tag and length.
    pub contents: &'a [u8],
    /// Tag, length, and contents, exactly as they appeared in the input.
    pub encoded: &'a [u8],
}

impl fmt::Debug for Value<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Value")
            .field("tag", &self.tag)
            .field("len", &self.contents.len())
            .finish_non_exhaustive()
    }
}

/// A cursor over a DER-encoded byte slice.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Start reading at the beginning of `input`.
    pub const fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    /// True when every byte has been consumed.
    pub const fn is_empty(&self) -> bool {
        self.pos >= self.input.len()
    }

    /// How many bytes remain unread.
    pub const fn remaining(&self) -> usize {
        self.input.len() - self.pos
    }

    /// Consume the reader, erroring if anything is left.
    ///
    /// Call this at the end of every structure. It is what turns "I parsed
    /// the fields I wanted" into "this input was exactly that structure and
    /// nothing else."
    pub fn finish(self) -> Result<()> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(DerError::TrailingData {
                remaining: self.remaining(),
            })
        }
    }

    /// The tag octet of the next value, without consuming it.
    pub fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(DerError::LengthTooLarge)?;
        let slice = self
            .input
            .get(self.pos..end)
            .ok_or(DerError::UnexpectedEnd)?;
        self.pos = end;
        Ok(slice)
    }

    fn take_byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// Read the next value whatever its tag.
    pub fn read_any(&mut self) -> Result<Value<'a>> {
        let start = self.pos;

        let tag_octet = self.take_byte()?;
        if tag_octet & 0x1f == 0x1f {
            return Err(DerError::HighTagNumberForm);
        }

        let first = self.take_byte()?;
        let len = match first {
            0x80 => return Err(DerError::IndefiniteLength),
            0xff => return Err(DerError::NonMinimalLength),
            n if n < 0x80 => usize::from(n),
            n => {
                let count = usize::from(n & 0x7f);
                let bytes = self.take(count)?;
                if bytes[0] == 0 {
                    // A leading zero means a shorter encoding existed.
                    return Err(DerError::NonMinimalLength);
                }
                if count > core::mem::size_of::<usize>() {
                    return Err(DerError::LengthTooLarge);
                }
                let mut len = 0usize;
                for &b in bytes {
                    len = (len << 8) | usize::from(b);
                }
                if len < 0x80 {
                    // The short form could have carried this.
                    return Err(DerError::NonMinimalLength);
                }
                len
            }
        };

        let contents = self.take(len)?;
        Ok(Value {
            tag: Tag(tag_octet),
            contents,
            encoded: &self.input[start..self.pos],
        })
    }

    /// Read the next value, requiring `tag`.
    pub fn read(&mut self, tag: Tag) -> Result<Value<'a>> {
        let before = self.pos;
        let value = self.read_any()?;
        if value.tag != tag {
            self.pos = before;
            return Err(DerError::UnexpectedTag {
                expected: tag,
                found: value.tag.0,
            });
        }
        Ok(value)
    }

    /// Read the next value if it has `tag`, otherwise leave the cursor alone.
    ///
    /// This is how `OPTIONAL` fields are read; a wrong tag is not an error
    /// here, it just means the optional field is absent.
    pub fn read_optional(&mut self, tag: Tag) -> Result<Option<Value<'a>>> {
        match self.peek() {
            Some(octet) if octet == tag.0 => self.read(tag).map(Some),
            _ => Ok(None),
        }
    }

    /// Read a `SEQUENCE` and return a reader over its contents.
    ///
    /// The returned reader is independent: nesting in the input becomes
    /// another loop, never another stack frame.
    pub fn read_sequence(&mut self) -> Result<Reader<'a>> {
        Ok(Reader::new(self.read(Tag::SEQUENCE)?.contents))
    }

    /// Read a `SET` and return a reader over its contents.
    pub fn read_set(&mut self) -> Result<Reader<'a>> {
        Ok(Reader::new(self.read(Tag::SET)?.contents))
    }

    /// Read an unsigned `INTEGER`, returning its minimal big-endian
    /// magnitude with any DER sign octet removed.
    ///
    /// Certificate serial numbers are read this way: they are arbitrarily
    /// large (RFC 5280 permits up to 20 octets) so there is no integer type
    /// to decode into, and comparing them is a byte comparison anyway.
    pub fn read_unsigned_integer(&mut self) -> Result<&'a [u8]> {
        let contents = self.read(Tag::INTEGER)?.contents;
        match contents {
            [] => Err(DerError::EmptyInteger),
            [first, ..] if first & 0x80 != 0 => Err(DerError::NegativeInteger),
            // A single zero octet is the canonical encoding of zero.
            [0] => Ok(contents),
            // A leading zero is only permitted to clear the sign bit.
            [0, second, ..] if second & 0x80 == 0 => Err(DerError::NonMinimalInteger),
            [0, ..] => Ok(&contents[1..]),
            _ => Ok(contents),
        }
    }

    /// Read an unsigned `INTEGER` that must fit in a `u64`.
    pub fn read_u64(&mut self) -> Result<u64> {
        let magnitude = self.read_unsigned_integer()?;
        if magnitude.len() > 8 {
            return Err(DerError::IntegerTooLarge);
        }
        let mut value = 0u64;
        for &b in magnitude {
            value = (value << 8) | u64::from(b);
        }
        Ok(value)
    }

    /// Read a `BOOLEAN`, requiring DER's canonical `0x00`/`0xff`.
    pub fn read_bool(&mut self) -> Result<bool> {
        match self.read(Tag::BOOLEAN)?.contents {
            [0x00] => Ok(false),
            [0xff] => Ok(true),
            [other] => Err(DerError::NonCanonicalBoolean(*other)),
            _ => Err(DerError::MalformedBitString),
        }
    }

    /// Read a `BIT STRING` that uses a whole number of octets, returning them.
    ///
    /// Every `BIT STRING` this crate reads — signatures, public keys — is
    /// octet-aligned, so a non-zero unused-bits count is a malformed
    /// certificate rather than a case to handle.
    pub fn read_bit_string_octets(&mut self) -> Result<&'a [u8]> {
        let contents = self.read(Tag::BIT_STRING)?.contents;
        match contents {
            [] => Err(DerError::MalformedBitString),
            [0, rest @ ..] => Ok(rest),
            _ => Err(DerError::MalformedBitString),
        }
    }

    /// Read a `BIT STRING` used as a flag set, returning its bits and the
    /// number of unused trailing bits.
    ///
    /// This is the `KeyUsage` shape: a short string whose final octet is
    /// partly padding.
    pub fn read_bit_string_flags(&mut self) -> Result<(&'a [u8], u8)> {
        let contents = self.read(Tag::BIT_STRING)?.contents;
        let (&unused, bits) = contents.split_first().ok_or(DerError::MalformedBitString)?;
        if unused > 7 || (bits.is_empty() && unused != 0) {
            return Err(DerError::MalformedBitString);
        }
        Ok((bits, unused))
    }

    /// Read an `OBJECT IDENTIFIER`, returning its encoded body.
    ///
    /// The body is kept encoded rather than decoded into arcs because every
    /// use of an OID here is an equality test against a known constant, and
    /// comparing encoded bytes is both exact and free.
    pub fn read_oid(&mut self) -> Result<ObjectIdentifier<'a>> {
        let contents = self.read(Tag::OID)?.contents;
        if contents.is_empty() {
            return Err(DerError::MalformedOid);
        }
        // Each subidentifier is base-128, high bit set on all but the last
        // octet, and must not start with 0x80 (a non-minimal leading zero).
        let mut start_of_subidentifier = true;
        for (i, &octet) in contents.iter().enumerate() {
            if start_of_subidentifier && octet == 0x80 {
                return Err(DerError::MalformedOid);
            }
            start_of_subidentifier = octet & 0x80 == 0;
            if i == contents.len() - 1 && octet & 0x80 != 0 {
                // The last octet must terminate a subidentifier.
                return Err(DerError::MalformedOid);
            }
        }
        Ok(ObjectIdentifier(contents))
    }

    /// Read a `NULL`, which must have no contents.
    pub fn read_null(&mut self) -> Result<()> {
        if self.read(Tag::NULL)?.contents.is_empty() {
            Ok(())
        } else {
            Err(DerError::MalformedString)
        }
    }
}

/// An OID, kept in its encoded form for exact comparison.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectIdentifier<'a>(pub &'a [u8]);

impl ObjectIdentifier<'_> {
    /// The encoded body, excluding tag and length.
    pub const fn as_bytes(&self) -> &[u8] {
        self.0
    }
}

impl fmt::Debug for ObjectIdentifier<'_> {
    /// Renders dotted decimal, which is how OIDs appear in every spec and
    /// every other tool. Unparseable input falls back to hex rather than
    /// panicking — this is a `Debug` impl, not a validator.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some((&first, rest)) = self.0.split_first() else {
            return f.write_str("<empty OID>");
        };
        // X.690 §8.19.4: the first octet encodes two arcs as 40*a + b.
        write!(f, "{}.{}", first / 40, first % 40)?;

        let mut accumulator: u128 = 0;
        for &octet in rest {
            accumulator = match accumulator.checked_shl(7) {
                Some(shifted) => shifted | u128::from(octet & 0x7f),
                None => return write!(f, ".<overflow>"),
            };
            if octet & 0x80 == 0 {
                write!(f, ".{accumulator}")?;
                accumulator = 0;
            }
        }
        Ok(())
    }
}
