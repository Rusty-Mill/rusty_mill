//! AES block cipher (FIPS-197), std-only.
//!
//! Supports 128/192/256-bit keys, both directions. Needed for the Kerberos
//! AES encryption types (RFC 3962). The S-boxes and round constants are
//! computed from the Rijndael field arithmetic at key-setup time rather than
//! stored as literal tables, so there is nothing to transcribe incorrectly.
//!
//! This is a straightforward, non-constant-time implementation — fine for the
//! protocol work here, not hardened against timing side channels.

/// AES block size in bytes.
pub const BLOCK_LEN: usize = 16;

/// Multiply two elements of GF(2^8) (the Rijndael field, modulus 0x11B).
fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    for _ in 0..8 {
        if b & 1 != 0 {
            p ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1B;
        }
        b >>= 1;
    }
    p
}

/// Build the AES S-box and its inverse from the field inverse plus the affine
/// transform.
fn build_sboxes() -> ([u8; 256], [u8; 256]) {
    // Multiplicative inverse in GF(2^8), with inv(0) = 0.
    let mut inv = [0u8; 256];
    for a in 1u16..256 {
        for b in 1u16..256 {
            if gmul(a as u8, b as u8) == 1 {
                inv[a as usize] = b as u8;
                break;
            }
        }
    }

    let mut sbox = [0u8; 256];
    for (a, s) in sbox.iter_mut().enumerate() {
        let x = inv[a];
        // Affine transform: x ^ (x<<<1) ^ (x<<<2) ^ (x<<<3) ^ (x<<<4) ^ 0x63.
        let r =
            x ^ x.rotate_left(1) ^ x.rotate_left(2) ^ x.rotate_left(3) ^ x.rotate_left(4) ^ 0x63;
        *s = r;
    }

    let mut inv_sbox = [0u8; 256];
    for (i, &s) in sbox.iter().enumerate() {
        inv_sbox[s as usize] = i as u8;
    }
    (sbox, inv_sbox)
}

/// An AES cipher initialized with a key schedule.
pub struct Aes {
    round_keys: Vec<[u8; 16]>,
    rounds: usize,
    sbox: [u8; 256],
    inv_sbox: [u8; 256],
}

impl Aes {
    /// Create a cipher from a 16-, 24-, or 32-byte key.
    ///
    /// # Panics
    /// Panics if the key length is not 16, 24, or 32.
    pub fn new(key: &[u8]) -> Self {
        let nk = match key.len() {
            16 => 4,
            24 => 6,
            32 => 8,
            other => panic!("invalid AES key length: {other}"),
        };
        let rounds = nk + 6;
        let (sbox, inv_sbox) = build_sboxes();

        // Key expansion into 4*(rounds+1) 32-bit words.
        let total_words = 4 * (rounds + 1);
        let mut words: Vec<[u8; 4]> = Vec::with_capacity(total_words);
        for chunk in key.chunks_exact(4) {
            words.push([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        let mut rcon = 1u8;
        for i in nk..total_words {
            let mut temp = words[i - 1];
            if i % nk == 0 {
                // RotWord + SubWord + Rcon.
                temp = [temp[1], temp[2], temp[3], temp[0]];
                for b in temp.iter_mut() {
                    *b = sbox[*b as usize];
                }
                temp[0] ^= rcon;
                rcon = gmul(rcon, 2);
            } else if nk > 6 && i % nk == 4 {
                for b in temp.iter_mut() {
                    *b = sbox[*b as usize];
                }
            }
            let prev = words[i - nk];
            words.push([
                prev[0] ^ temp[0],
                prev[1] ^ temp[1],
                prev[2] ^ temp[2],
                prev[3] ^ temp[3],
            ]);
        }

        let mut round_keys = Vec::with_capacity(rounds + 1);
        for r in 0..=rounds {
            let mut rk = [0u8; 16];
            for c in 0..4 {
                rk[c * 4..c * 4 + 4].copy_from_slice(&words[r * 4 + c]);
            }
            round_keys.push(rk);
        }

        Aes {
            round_keys,
            rounds,
            sbox,
            inv_sbox,
        }
    }

    /// Encrypt one 16-byte block in place.
    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        add_round_key(block, &self.round_keys[0]);
        for round in 1..self.rounds {
            self.sub_bytes(block);
            shift_rows(block);
            mix_columns(block);
            add_round_key(block, &self.round_keys[round]);
        }
        self.sub_bytes(block);
        shift_rows(block);
        add_round_key(block, &self.round_keys[self.rounds]);
    }

    /// Decrypt one 16-byte block in place.
    pub fn decrypt_block(&self, block: &mut [u8; 16]) {
        add_round_key(block, &self.round_keys[self.rounds]);
        for round in (1..self.rounds).rev() {
            inv_shift_rows(block);
            self.inv_sub_bytes(block);
            add_round_key(block, &self.round_keys[round]);
            inv_mix_columns(block);
        }
        inv_shift_rows(block);
        self.inv_sub_bytes(block);
        add_round_key(block, &self.round_keys[0]);
    }

    fn sub_bytes(&self, block: &mut [u8; 16]) {
        for b in block.iter_mut() {
            *b = self.sbox[*b as usize];
        }
    }

    fn inv_sub_bytes(&self, block: &mut [u8; 16]) {
        for b in block.iter_mut() {
            *b = self.inv_sbox[*b as usize];
        }
    }
}

fn add_round_key(block: &mut [u8; 16], key: &[u8; 16]) {
    for (b, k) in block.iter_mut().zip(key.iter()) {
        *b ^= k;
    }
}

/// ShiftRows on a column-major state (`block[c*4 + r]`).
fn shift_rows(block: &mut [u8; 16]) {
    let orig = *block;
    for r in 1..4 {
        for c in 0..4 {
            block[c * 4 + r] = orig[((c + r) % 4) * 4 + r];
        }
    }
}

fn inv_shift_rows(block: &mut [u8; 16]) {
    let orig = *block;
    for r in 1..4 {
        for c in 0..4 {
            block[c * 4 + r] = orig[((c + 4 - r) % 4) * 4 + r];
        }
    }
}

fn mix_columns(block: &mut [u8; 16]) {
    for c in 0..4 {
        let s = [
            block[c * 4],
            block[c * 4 + 1],
            block[c * 4 + 2],
            block[c * 4 + 3],
        ];
        block[c * 4] = gmul(s[0], 2) ^ gmul(s[1], 3) ^ s[2] ^ s[3];
        block[c * 4 + 1] = s[0] ^ gmul(s[1], 2) ^ gmul(s[2], 3) ^ s[3];
        block[c * 4 + 2] = s[0] ^ s[1] ^ gmul(s[2], 2) ^ gmul(s[3], 3);
        block[c * 4 + 3] = gmul(s[0], 3) ^ s[1] ^ s[2] ^ gmul(s[3], 2);
    }
}

fn inv_mix_columns(block: &mut [u8; 16]) {
    for c in 0..4 {
        let s = [
            block[c * 4],
            block[c * 4 + 1],
            block[c * 4 + 2],
            block[c * 4 + 3],
        ];
        block[c * 4] = gmul(s[0], 14) ^ gmul(s[1], 11) ^ gmul(s[2], 13) ^ gmul(s[3], 9);
        block[c * 4 + 1] = gmul(s[0], 9) ^ gmul(s[1], 14) ^ gmul(s[2], 11) ^ gmul(s[3], 13);
        block[c * 4 + 2] = gmul(s[0], 13) ^ gmul(s[1], 9) ^ gmul(s[2], 14) ^ gmul(s[3], 11);
        block[c * 4 + 3] = gmul(s[0], 11) ^ gmul(s[1], 13) ^ gmul(s[2], 9) ^ gmul(s[3], 14);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn fips197_aes128() {
        let key = unhex("000102030405060708090a0b0c0d0e0f");
        let mut block = [0u8; 16];
        block.copy_from_slice(&unhex("00112233445566778899aabbccddeeff"));
        let aes = Aes::new(&key);
        aes.encrypt_block(&mut block);
        assert_eq!(hex(&block), "69c4e0d86a7b0430d8cdb78070b4c55a");
        aes.decrypt_block(&mut block);
        assert_eq!(hex(&block), "00112233445566778899aabbccddeeff");
    }

    #[test]
    fn fips197_aes192() {
        let key = unhex("000102030405060708090a0b0c0d0e0f1011121314151617");
        let mut block = [0u8; 16];
        block.copy_from_slice(&unhex("00112233445566778899aabbccddeeff"));
        let aes = Aes::new(&key);
        aes.encrypt_block(&mut block);
        assert_eq!(hex(&block), "dda97ca4864cdfe06eaf70a0ec0d7191");
        aes.decrypt_block(&mut block);
        assert_eq!(hex(&block), "00112233445566778899aabbccddeeff");
    }

    #[test]
    fn fips197_aes256() {
        let key = unhex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
        let mut block = [0u8; 16];
        block.copy_from_slice(&unhex("00112233445566778899aabbccddeeff"));
        let aes = Aes::new(&key);
        aes.encrypt_block(&mut block);
        assert_eq!(hex(&block), "8ea2b7ca516745bfeafc49904b496089");
        aes.decrypt_block(&mut block);
        assert_eq!(hex(&block), "00112233445566778899aabbccddeeff");
    }
}
