//! Unsigned LEB128.

use rusty_wire::{Reader, Writer};

use crate::Error;

pub(crate) fn write(w: &mut Writer, mut value: u64) {
    while value >= 0x80 {
        w.write_u8(value as u8 | 0x80);
        value >>= 7;
    }
    w.write_u8(value as u8);
}

pub(crate) fn read(r: &mut Reader<'_>) -> Result<u64, Error> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = r.read_u8()?;
        let bits = u64::from(byte & 0x7f);
        // The tenth byte may only carry the top bit of a u64.
        if shift == 63 && bits > 1 {
            return Err(Error::VarintOverflow);
        }
        value |= bits << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(Error::VarintOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(value: u64) -> (u64, usize) {
        let mut w = Writer::new();
        write(&mut w, value);
        let bytes = w.into_vec();
        let mut r = Reader::new(&bytes);
        (read(&mut r).unwrap(), bytes.len())
    }

    #[test]
    fn boundaries_round_trip() {
        for (value, len) in [
            (0, 1),
            (127, 1),
            (128, 2),
            (16_383, 2),
            (16_384, 3),
            (u64::MAX, 10),
        ] {
            assert_eq!(round_trip(value), (value, len), "{value}");
        }
    }

    #[test]
    fn overlong_and_overflowing_are_rejected() {
        let eleven = [0x80u8; 10]
            .iter()
            .copied()
            .chain([0x00])
            .collect::<Vec<_>>();
        assert_eq!(read(&mut Reader::new(&eleven)), Err(Error::VarintOverflow));
        let mut too_big = [0xffu8; 10];
        too_big[9] = 0x02;
        assert_eq!(read(&mut Reader::new(&too_big)), Err(Error::VarintOverflow));
    }

    #[test]
    fn truncated_is_rejected() {
        assert_eq!(read(&mut Reader::new(&[0x80])), Err(Error::Truncated));
    }
}
