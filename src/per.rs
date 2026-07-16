//! ALIGNED BASIC-PER helpers (ITU-T X.691) — only the slice used by MCS.
//!
//! The MCS domain PDUs ([`crate::mcs`]) are encoded with the ALIGNED variant
//! of Packed Encoding Rules. RDP uses a very small subset: length
//! determinants, a couple of integer shapes, single-byte enumerations, and a
//! single-byte CHOICE index. Only those are implemented here.
//!
//! None of these helpers deal with the bit-packing that full PER can require;
//! every value in this subset happens to land on a byte boundary, which is
//! why the encodings are so compact.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// Write a PER length determinant (short form up to 0x7F, else two bytes).
///
/// Values of 16384 or more would require the fragmented form, which RDP does
/// not use here; callers must keep lengths below that.
pub fn write_length(w: &mut Writer, length: usize) -> Result<()> {
    if length <= 0x7F {
        w.write_u8(length as u8);
    } else if length < 0x4000 {
        w.write_u16_be(0x8000 | length as u16);
    } else {
        return Err(Error::Overflow {
            field: "PER length",
        });
    }
    Ok(())
}

/// Read a PER length determinant written by [`write_length`].
pub fn read_length(r: &mut Reader<'_>) -> Result<usize> {
    let first = r.read_u8()?;
    if first & 0x80 == 0 {
        Ok(first as usize)
    } else if first & 0x40 == 0 {
        let second = r.read_u8()?;
        Ok((((first & 0x3F) as usize) << 8) | second as usize)
    } else {
        Err(Error::InvalidValue {
            field: "PER length",
            value: format!("fragmented (0x{first:02X})"),
        })
    }
}

/// Write an unconstrained INTEGER as a length-prefixed big-endian value.
///
/// Uses the minimum number of bytes (1, 2, or 4); e.g. `0` encodes as
/// `01 00`.
pub fn write_integer(w: &mut Writer, value: u32) -> Result<()> {
    if value <= 0xFF {
        write_length(w, 1)?;
        w.write_u8(value as u8);
    } else if value <= 0xFFFF {
        write_length(w, 2)?;
        w.write_u16_be(value as u16);
    } else {
        write_length(w, 4)?;
        w.write_u32_be(value);
    }
    Ok(())
}

/// Read an INTEGER written by [`write_integer`].
pub fn read_integer(r: &mut Reader<'_>) -> Result<u32> {
    let len = read_length(r)?;
    match len {
        1 => Ok(r.read_u8()? as u32),
        2 => Ok(r.read_u16_be()? as u32),
        3 => {
            let b = r.read_bytes(3)?;
            Ok(((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32)
        }
        4 => Ok(r.read_u32_be()?),
        other => Err(Error::InvalidLength {
            field: "PER integer",
            length: other,
        }),
    }
}

/// Write a constrained 16-bit INTEGER offset by `min` (two bytes, big-endian).
///
/// Used for MCS `UserId` and `ChannelId`, whose ranges make the encoding
/// exactly two bytes with no length prefix.
pub fn write_integer16(w: &mut Writer, value: u16, min: u16) -> Result<()> {
    let offset = value.checked_sub(min).ok_or(Error::InvalidValue {
        field: "PER integer16",
        value: format!("{value} < min {min}"),
    })?;
    w.write_u16_be(offset);
    Ok(())
}

/// Read a constrained 16-bit INTEGER, adding back `min`.
pub fn read_integer16(r: &mut Reader<'_>, min: u16) -> Result<u16> {
    let raw = r.read_u16_be()?;
    raw.checked_add(min).ok_or(Error::InvalidValue {
        field: "PER integer16",
        value: format!("{raw} + min {min} overflows"),
    })
}

/// Write a small ENUMERATED value (single byte in this subset).
pub fn write_enumerated(w: &mut Writer, value: u8) {
    w.write_u8(value);
}

/// Read a single-byte ENUMERATED value.
pub fn read_enumerated(r: &mut Reader<'_>) -> Result<u8> {
    r.read_u8()
}

/// Write a pre-computed CHOICE index / MCS domain-PDU header byte.
pub fn write_choice(w: &mut Writer, choice: u8) {
    w.write_u8(choice);
}

/// Read a CHOICE index / MCS domain-PDU header byte.
pub fn read_choice(r: &mut Reader<'_>) -> Result<u8> {
    r.read_u8()
}

/// Write a single-byte SELECTION (bit map of present optional fields).
pub fn write_selection(w: &mut Writer, selection: u8) {
    w.write_u8(selection);
}

/// Read a single-byte SELECTION.
pub fn read_selection(r: &mut Reader<'_>) -> Result<u8> {
    r.read_u8()
}

/// Write the "number of sets" count that precedes a `SET OF` (single byte in
/// this subset).
pub fn write_number_of_sets(w: &mut Writer, count: u8) {
    w.write_u8(count);
}

/// Read the "number of sets" count.
pub fn read_number_of_sets(r: &mut Reader<'_>) -> Result<u8> {
    r.read_u8()
}

/// Write `n` zero padding bytes.
pub fn write_padding(w: &mut Writer, n: usize) {
    for _ in 0..n {
        w.write_u8(0);
    }
}

/// Skip `n` padding bytes.
pub fn read_padding(r: &mut Reader<'_>, n: usize) -> Result<()> {
    r.skip(n)
}

/// Write a constrained OCTET STRING whose length is at least `min`.
///
/// Only the amount above `min` is encoded in the length determinant, so a
/// fixed-size 4-byte field with `min == 4` emits a zero length byte followed
/// by the four content bytes.
pub fn write_octet_string(w: &mut Writer, data: &[u8], min: usize) -> Result<()> {
    write_length(w, data.len().saturating_sub(min))?;
    w.write_bytes(data);
    Ok(())
}

/// Read a constrained OCTET STRING written by [`write_octet_string`].
pub fn read_octet_string<'a>(r: &mut Reader<'a>, min: usize) -> Result<&'a [u8]> {
    let mlength = read_length(r)?;
    r.read_bytes(mlength + min)
}

/// Write the six-tuple OBJECT IDENTIFIER form GCC uses (`{0 0 20 124 0 1}`).
///
/// The first two arcs pack into a single byte; the remaining four are one
/// byte each, giving a fixed five-byte body.
pub fn write_object_identifier(w: &mut Writer, oid: &[u8; 6]) -> Result<()> {
    write_length(w, 5)?;
    w.write_u8((oid[0] << 4) | (oid[1] & 0x0F));
    w.write_u8(oid[2]);
    w.write_u8(oid[3]);
    w.write_u8(oid[4]);
    w.write_u8(oid[5]);
    Ok(())
}

/// Read the OBJECT IDENTIFIER form written by [`write_object_identifier`].
pub fn read_object_identifier(r: &mut Reader<'_>) -> Result<[u8; 6]> {
    let len = read_length(r)?;
    if len != 5 {
        return Err(Error::InvalidLength {
            field: "PER object identifier",
            length: len,
        });
    }
    let b = r.read_bytes(5)?;
    Ok([b[0] >> 4, b[0] & 0x0F, b[1], b[2], b[3], b[4]])
}

/// Write a NumericString (ASCII digits) constrained with lower bound `min`.
///
/// Each digit is stored as its value in a nibble, two digits per byte; an odd
/// final digit is padded with a zero nibble.
pub fn write_numeric_string(w: &mut Writer, digits: &[u8], min: usize) -> Result<()> {
    write_length(w, digits.len().saturating_sub(min))?;
    let mut i = 0;
    while i < digits.len() {
        let hi = digits[i].wrapping_sub(b'0');
        let lo = if i + 1 < digits.len() {
            digits[i + 1].wrapping_sub(b'0')
        } else {
            0
        };
        w.write_u8((hi << 4) | (lo & 0x0F));
        i += 2;
    }
    Ok(())
}

/// Skip a NumericString written by [`write_numeric_string`], given the same
/// lower bound `min`. The value itself is not needed by RDP.
pub fn read_numeric_string(r: &mut Reader<'_>, min: usize) -> Result<()> {
    let mlength = read_length(r)?;
    let chars = mlength + min;
    r.skip((chars + 1) / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_short_and_long() {
        let mut w = Writer::new();
        write_length(&mut w, 0x7F).unwrap();
        write_length(&mut w, 0x80).unwrap();
        write_length(&mut w, 0x3FFF).unwrap();
        let bytes = w.into_vec();
        assert_eq!(bytes, [0x7F, 0x80, 0x80, 0xBF, 0xFF]);
        let mut r = Reader::new(&bytes);
        assert_eq!(read_length(&mut r).unwrap(), 0x7F);
        assert_eq!(read_length(&mut r).unwrap(), 0x80);
        assert_eq!(read_length(&mut r).unwrap(), 0x3FFF);
    }

    #[test]
    fn length_too_large_errors() {
        let mut w = Writer::new();
        assert!(write_length(&mut w, 0x4000).is_err());
    }

    #[test]
    fn integer_minimal_width() {
        for (val, expected) in [
            (0u32, vec![0x01, 0x00]),
            (0xFF, vec![0x01, 0xFF]),
            (0x0100, vec![0x02, 0x01, 0x00]),
            (0x0001_0000, vec![0x04, 0x00, 0x01, 0x00, 0x00]),
        ] {
            let mut w = Writer::new();
            write_integer(&mut w, val).unwrap();
            assert_eq!(w.as_slice(), expected.as_slice(), "value {val:#x}");
            let mut r = Reader::new(&expected);
            assert_eq!(read_integer(&mut r).unwrap(), val);
        }
    }

    #[test]
    fn integer16_offsets_by_min() {
        let mut w = Writer::new();
        write_integer16(&mut w, 1007, 1001).unwrap();
        assert_eq!(w.as_slice(), &[0x00, 0x06]);
        let mut r = Reader::new(&[0x00, 0x06]);
        assert_eq!(read_integer16(&mut r, 1001).unwrap(), 1007);
    }
}
