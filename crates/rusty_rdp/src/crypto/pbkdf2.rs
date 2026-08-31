//! PBKDF2-HMAC-SHA1 (RFC 2898 / RFC 8018), std-only.
//!
//! Kerberos's AES string-to-key (RFC 3962) derives the base key material with
//! PBKDF2 over HMAC-SHA1. This is the only key-stretching primitive in the
//! crate.

use crate::crypto::hmac::hmac_sha1;

/// Derive `dk_len` bytes from `password` and `salt` with `iterations` rounds of
/// PBKDF2-HMAC-SHA1.
pub fn pbkdf2_hmac_sha1(password: &[u8], salt: &[u8], iterations: u32, dk_len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(dk_len);
    let mut block_index: u32 = 1;
    while out.len() < dk_len {
        // U1 = HMAC(password, salt || INT_BE(block_index)).
        let mut salted = Vec::with_capacity(salt.len() + 4);
        salted.extend_from_slice(salt);
        salted.extend_from_slice(&block_index.to_be_bytes());
        let mut u = hmac_sha1(password, &salted);
        let mut t = u;
        for _ in 1..iterations {
            u = hmac_sha1(password, &u);
            for (t_byte, u_byte) in t.iter_mut().zip(u.iter()) {
                *t_byte ^= *u_byte;
            }
        }
        out.extend_from_slice(&t);
        block_index += 1;
    }
    out.truncate(dk_len);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn rfc6070_vectors() {
        assert_eq!(
            hex(&pbkdf2_hmac_sha1(b"password", b"salt", 1, 20)),
            "0c60c80f961f0e71f3a9b524af6012062fe037a6"
        );
        assert_eq!(
            hex(&pbkdf2_hmac_sha1(b"password", b"salt", 2, 20)),
            "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957"
        );
        assert_eq!(
            hex(&pbkdf2_hmac_sha1(b"password", b"salt", 4096, 20)),
            "4b007901b765489abead49d926f721d065a429c1"
        );
        assert_eq!(
            hex(&pbkdf2_hmac_sha1(
                b"passwordPASSWORDpassword",
                b"saltSALTsaltSALTsaltSALTsaltSALTsalt",
                4096,
                25
            )),
            "3d2eec4fe41c849b80c8d83662c0e44a8b291a964cf2f07038"
        );
    }
}
