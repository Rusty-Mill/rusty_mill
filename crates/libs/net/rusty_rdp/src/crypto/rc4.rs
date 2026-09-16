//! RC4 stream cipher, std-only.
//!
//! RC4 is insecure and appears here solely because RDP standard security
//! (MS-RDPBCGR §5.3) encrypts I/O with it. Do not use it for anything new.

/// RC4 keystream generator / cipher state.
///
/// Encryption and decryption are the same operation (XOR with the
/// keystream), so [`Rc4::apply`] serves both directions. The state is
/// mutable and advances with every byte, matching RDP's per-direction
/// continuous cipher.
#[derive(Clone)]
pub struct Rc4 {
    s: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4 {
    /// Initialise the cipher with `key` (the key-scheduling algorithm).
    pub fn new(key: &[u8]) -> Self {
        let mut s = [0u8; 256];
        for (i, slot) in s.iter_mut().enumerate() {
            *slot = i as u8;
        }
        let mut j = 0u8;
        for i in 0..256 {
            j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
            s.swap(i, j as usize);
        }
        Rc4 { s, i: 0, j: 0 }
    }

    /// XOR the keystream into `data` in place, advancing the cipher state.
    pub fn apply(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            self.i = self.i.wrapping_add(1);
            self.j = self.j.wrapping_add(self.s[self.i as usize]);
            self.s.swap(self.i as usize, self.j as usize);
            let k =
                self.s[(self.s[self.i as usize].wrapping_add(self.s[self.j as usize])) as usize];
            *byte ^= k;
        }
    }

    /// Return a freshly-XORed copy of `data`, leaving the input untouched but
    /// still advancing the cipher state.
    pub fn applied(&mut self, data: &[u8]) -> Vec<u8> {
        let mut out = data.to_vec();
        self.apply(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn known_vectors() {
        // Classic RC4 test vectors.
        let mut c = Rc4::new(b"Key");
        assert_eq!(hex(&c.applied(b"Plaintext")), "bbf316e8d940af0ad3");

        let mut c = Rc4::new(b"Wiki");
        assert_eq!(hex(&c.applied(b"pedia")), "1021bf0420");

        let mut c = Rc4::new(b"Secret");
        assert_eq!(
            hex(&c.applied(b"Attack at dawn")),
            "45a01f645fc35b383552544b9bf5"
        );
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let plaintext = b"the quick brown fox";
        let mut enc = Rc4::new(b"session-key");
        let ciphertext = enc.applied(plaintext);
        assert_ne!(&ciphertext[..], &plaintext[..]);
        let mut dec = Rc4::new(b"session-key");
        assert_eq!(dec.applied(&ciphertext), plaintext);
    }

    #[test]
    fn state_advances_across_calls() {
        // Applying in two chunks matches applying in one.
        let mut whole = Rc4::new(b"k");
        let whole_out = whole.applied(b"abcdefgh");

        let mut split = Rc4::new(b"k");
        let mut part = split.applied(b"abcd");
        part.extend(split.applied(b"efgh"));
        assert_eq!(part, whole_out);
    }
}
