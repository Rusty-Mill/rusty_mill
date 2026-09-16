//! Kerberos RC4-HMAC encryption profile (etype 23, RFC 4757), std-only.
//!
//! RC4-HMAC is the one Kerberos encryption type that reuses primitives this
//! crate already has — MD4 (the string-to-key is just the NT hash), HMAC-MD5,
//! and RC4 — so it is the natural first Kerberos profile. Modern KDCs prefer
//! the AES profiles (etypes 17/18); those need an AES implementation and are a
//! later addition.
//!
//! ## Security warning
//!
//! RC4-HMAC is weak and deprecated. It is implemented for interoperability
//! with Kerberos deployments that still accept it, not for protection.

use crate::crypto::hmac::hmac_md5;
use crate::crypto::md4::md4;
use crate::crypto::md5::md5;
use crate::crypto::rc4::Rc4;
use crate::error::{Error, Result};

/// Encryption type number for RC4-HMAC (MS-KILE / RFC 4757).
pub const ETYPE_RC4_HMAC: i32 = 23;
/// Checksum type number for HMAC-MD5 (RFC 4757 §5).
pub const CKSUMTYPE_HMAC_MD5: i32 = -138;

/// The RC4-HMAC "signature key" derivation constant.
const SIGNATURE_KEY: &[u8] = b"signaturekey\0";

/// Length of an RC4-HMAC confounder in bytes.
pub const CONFOUNDER_LEN: usize = 8;

/// Derive the RC4-HMAC base key from a password: `MD4(UTF-16LE(password))`.
///
/// This is exactly the NTLM NT hash — RC4-HMAC uses no salt.
pub fn string_to_key(password: &str) -> [u8; 16] {
    let unicode: Vec<u8> = password
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    md4(&unicode)
}

/// Translate an RFC 4120 key usage to the "message type" value RC4-HMAC keys
/// its HMACs with (MS-KILE remaps a couple of AS/TGS-REP usages to 8).
fn ms_usage(usage: u32) -> u32 {
    match usage {
        3 | 9 => 8,
        other => other,
    }
}

/// Encrypt `plaintext` under `key` for key `usage`, prepending `confounder`
/// (which must be [`CONFOUNDER_LEN`] bytes; supply random bytes in production).
///
/// Output is `Checksum(16) || RC4(Confounder || plaintext)` (RFC 4757 §4).
pub fn encrypt(key: &[u8; 16], usage: u32, plaintext: &[u8], confounder: &[u8]) -> Vec<u8> {
    let t = ms_usage(usage).to_le_bytes();
    let k1 = hmac_md5(key, &t);
    // k2 == k1 for RC4-HMAC.
    let mut data = Vec::with_capacity(confounder.len() + plaintext.len());
    data.extend_from_slice(confounder);
    data.extend_from_slice(plaintext);
    let checksum = hmac_md5(&k1, &data);
    let k3 = hmac_md5(&k1, &checksum);
    let mut ciphertext = Rc4::new(&k3).applied(&data);

    let mut out = Vec::with_capacity(16 + ciphertext.len());
    out.extend_from_slice(&checksum);
    out.append(&mut ciphertext);
    out
}

/// Decrypt an RC4-HMAC token (`Checksum(16) || ciphertext`) under `key` for
/// key `usage`, verifying the checksum and stripping the confounder.
pub fn decrypt(key: &[u8; 16], usage: u32, token: &[u8]) -> Result<Vec<u8>> {
    if token.len() < 16 + CONFOUNDER_LEN {
        return Err(Error::InvalidLength {
            field: "RC4-HMAC token",
            length: token.len(),
        });
    }
    let checksum = &token[..16];
    let ciphertext = &token[16..];

    let t = ms_usage(usage).to_le_bytes();
    let k1 = hmac_md5(key, &t);
    let k3 = hmac_md5(&k1, checksum);
    let data = Rc4::new(&k3).applied(ciphertext);

    let expected = hmac_md5(&k1, &data);
    if expected != checksum {
        return Err(Error::InvalidValue {
            field: "RC4-HMAC checksum",
            value: "verification failed".to_string(),
        });
    }
    Ok(data[CONFOUNDER_LEN..].to_vec())
}

/// Compute the RC4-HMAC keyed checksum (cksumtype HMAC-MD5) of `data` for
/// key `usage` (RFC 4757 §5).
pub fn checksum(key: &[u8; 16], usage: u32, data: &[u8]) -> [u8; 16] {
    let ksign = hmac_md5(key, SIGNATURE_KEY);
    let mut tmp_input = Vec::with_capacity(4 + data.len());
    tmp_input.extend_from_slice(&ms_usage(usage).to_le_bytes());
    tmp_input.extend_from_slice(data);
    let tmp = md5(&tmp_input);
    hmac_md5(&ksign, &tmp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn string_to_key_is_nt_hash() {
        // Same NT hash as NTLM: MD4(UTF-16LE("Password")).
        assert_eq!(
            hex(&string_to_key("Password")),
            "a4f49c406510bdcab6824ee7c30fd852"
        );
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = string_to_key("Password");
        let confounder = [0x11u8; CONFOUNDER_LEN];
        let token = encrypt(&key, 4, b"kerberos authenticator", &confounder);
        // The checksum prefix and confounder make the token longer.
        assert!(token.len() > 16 + CONFOUNDER_LEN);
        let recovered = decrypt(&key, 4, &token).unwrap();
        assert_eq!(recovered, b"kerberos authenticator");
    }

    #[test]
    fn tampered_token_is_rejected() {
        let key = string_to_key("Password");
        let mut token = encrypt(&key, 4, b"payload", &[0x22u8; CONFOUNDER_LEN]);
        let last = token.len() - 1;
        token[last] ^= 0xFF;
        assert!(decrypt(&key, 4, &token).is_err());
    }

    #[test]
    fn usage_mapping_changes_keystream() {
        // Usage 3 maps to 8, usage 9 maps to 8, so those share a token; usage
        // 4 does not.
        let key = string_to_key("secret");
        let c = [0x33u8; CONFOUNDER_LEN];
        let t3 = encrypt(&key, 3, b"x", &c);
        let t9 = encrypt(&key, 9, b"x", &c);
        let t4 = encrypt(&key, 4, b"x", &c);
        assert_eq!(t3, t9);
        assert_ne!(t3, t4);
    }

    #[test]
    fn checksum_is_deterministic() {
        let key = string_to_key("Password");
        let a = checksum(&key, 6, b"authenticator body");
        let b = checksum(&key, 6, b"authenticator body");
        assert_eq!(a, b);
        assert_ne!(a, checksum(&key, 7, b"authenticator body"));
    }
}
