//! Hand-rolled HMAC-SHA256 (RFC 2104 / FIPS 198-1).

use super::sha256::{Digest, Sha256};

const BLOCK_SIZE: usize = 64;

/// Computes HMAC-SHA256(key, message).
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> Digest {
    let mut block_key = [0u8; BLOCK_SIZE];

    if key.len() > BLOCK_SIZE {
        let hashed = super::sha256::sha256(key);
        block_key[..32].copy_from_slice(&hashed);
    } else {
        block_key[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= block_key[i];
        opad[i] ^= block_key[i];
    }

    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(message);
    let inner_digest = inner.finish();

    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(&inner_digest);
    outer.finish()
}

/// Constant-time comparison, to avoid leaking equality via timing side channels
/// when verifying MACs or comparing secrets (e.g. `state`, PKCE verifiers).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    #[test]
    fn rfc4231_case_1() {
        let key = [0x0b; 20];
        let data = b"Hi There";
        let mac = hmac_sha256(&key, data);
        assert_eq!(
            hex(&mac),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn rfc4231_case_2() {
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let mac = hmac_sha256(key, data);
        assert_eq!(
            hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn constant_time_eq_basic() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
