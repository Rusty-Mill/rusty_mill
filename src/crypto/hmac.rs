//! HMAC (RFC 2104 / RFC 2202), std-only.
//!
//! NTLMv2 (MS-NLMP) is defined entirely in terms of HMAC-MD5. Kerberos's AES
//! profiles (RFC 3962) use HMAC-SHA1 (for PBKDF2 and the message integrity
//! checksum). Both are provided here on the crate's own MD5/SHA-1.

use crate::crypto::md5::Md5;
use crate::crypto::sha1::Sha1;

const BLOCK_LEN: usize = 64;

/// Compute `HMAC-MD5(key, message)`.
pub fn hmac_md5(key: &[u8], message: &[u8]) -> [u8; 16] {
    // Keys longer than the block size are hashed down first.
    let mut key_block = [0u8; BLOCK_LEN];
    if key.len() > BLOCK_LEN {
        let digest = {
            let mut h = Md5::new();
            h.update(key);
            h.finish()
        };
        key_block[..digest.len()].copy_from_slice(&digest);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_LEN];
    let mut opad = [0x5cu8; BLOCK_LEN];
    for i in 0..BLOCK_LEN {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }

    let inner = {
        let mut h = Md5::new();
        h.update(&ipad);
        h.update(message);
        h.finish()
    };
    let mut h = Md5::new();
    h.update(&opad);
    h.update(&inner);
    h.finish()
}

/// Compute `HMAC-SHA1(key, message)`.
pub fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    let mut key_block = [0u8; BLOCK_LEN];
    if key.len() > BLOCK_LEN {
        let digest = {
            let mut h = Sha1::new();
            h.update(key);
            h.finish()
        };
        key_block[..digest.len()].copy_from_slice(&digest);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_LEN];
    let mut opad = [0x5cu8; BLOCK_LEN];
    for i in 0..BLOCK_LEN {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }

    let inner = {
        let mut h = Sha1::new();
        h.update(&ipad);
        h.update(message);
        h.finish()
    };
    let mut h = Sha1::new();
    h.update(&opad);
    h.update(&inner);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn rfc2202_vectors() {
        // RFC 2202, section 2 — HMAC-MD5 test cases.
        assert_eq!(
            hex(&hmac_md5(&[0x0b; 16], b"Hi There")),
            "9294727a3638bb1c13f48ef8158bfc9d"
        );
        assert_eq!(
            hex(&hmac_md5(b"Jefe", b"what do ya want for nothing?")),
            "750c783e6ab0b503eaa86e310a5db738"
        );
        assert_eq!(
            hex(&hmac_md5(&[0xaa; 16], &[0xdd; 50])),
            "56be34521d144c88dbb8c733f0e8b3f6"
        );
        // A key longer than the 64-byte block is hashed first.
        assert_eq!(
            hex(&hmac_md5(
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "6b1ab7fe4bd7bf8f0b62e6ce61b9d0cd"
        );
    }

    #[test]
    fn rfc2202_sha1_vectors() {
        // RFC 2202, section 3 — HMAC-SHA1 test cases.
        assert_eq!(
            hex(&hmac_sha1(&[0x0b; 20], b"Hi There")),
            "b617318655057264e28bc0b6fb378c8ef146be00"
        );
        assert_eq!(
            hex(&hmac_sha1(b"Jefe", b"what do ya want for nothing?")),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
        );
        assert_eq!(
            hex(&hmac_sha1(&[0xaa; 20], &[0xdd; 50])),
            "125d7342b9ac11cd91a39af48aa17b4f63f175d3"
        );
    }
}
