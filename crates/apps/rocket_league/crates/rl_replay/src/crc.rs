//! The section CRC: CRC-32 with polynomial 0x04C11DB7, not reflected, initial
//! value 0x10340DFE and final XOR 0xFFFFFFFF (`replay-file-format.md` §1).
//! Byte at a time from a 256-entry table; a 1.3 MB content section takes a
//! few milliseconds.

const POLY: u32 = 0x04C1_1DB7;
const INIT: u32 = 0x1034_0DFE;
const XOR_OUT: u32 = 0xFFFF_FFFF;

const TABLE: [u32; 256] = table();

const fn table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ POLY
            } else {
                crc << 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// The CRC a replay stores for a section with this data.
pub fn crc(data: &[u8]) -> u32 {
    let crc = data.iter().fold(INIT, |crc, &byte| {
        (crc << 8) ^ TABLE[usize::from((crc >> 24) as u8 ^ byte)]
    });
    crc ^ XOR_OUT
}

#[cfg(test)]
mod tests {
    use super::*;

    /// boxcars' `single_byte_test`.
    #[test]
    fn one_byte() {
        assert_eq!(crc(&[0xA0]), 0x76CC_8C81);
    }

    #[test]
    fn empty_data_is_the_initial_value_inverted() {
        assert_eq!(crc(&[]), !INIT);
    }
}
