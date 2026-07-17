//! Kerberos AES encryption profiles (RFC 3962), std-only.
//!
//! Implements `aes128-cts-hmac-sha1-96` (etype 17) and
//! `aes256-cts-hmac-sha1-96` (etype 18): the `n-fold` and `DK`/`DR` key
//! derivation (RFC 3961), the PBKDF2 string-to-key (RFC 3962), AES in
//! CBC-CTS mode, and the HMAC-SHA1-96 integrity checksum. These are the
//! encryption types modern KDCs prefer.
//!
//! Building blocks are validated against published vectors (FIPS-197 AES,
//! RFC 6070 PBKDF2, RFC 3961 n-fold, RFC 3962 string-to-key). The full-message
//! CBC-CTS path is checked by round trip.

use crate::crypto::aes::Aes;
use crate::crypto::hmac::hmac_sha1;
use crate::crypto::pbkdf2::pbkdf2_hmac_sha1;
use crate::error::{Error, Result};

/// `aes128-cts-hmac-sha1-96` encryption type.
pub const ETYPE_AES128_CTS_HMAC_SHA1_96: i32 = 17;
/// `aes256-cts-hmac-sha1-96` encryption type.
pub const ETYPE_AES256_CTS_HMAC_SHA1_96: i32 = 18;
/// `hmac-sha1-96-aes128` checksum type.
pub const CKSUMTYPE_HMAC_SHA1_96_AES128: i32 = 15;
/// `hmac-sha1-96-aes256` checksum type.
pub const CKSUMTYPE_HMAC_SHA1_96_AES256: i32 = 16;

/// Confounder length (one AES block).
pub const CONFOUNDER_LEN: usize = 16;
/// Truncated HMAC-SHA1 tag length (96 bits).
const MAC_LEN: usize = 12;
/// Default string-to-key iteration count (RFC 3962).
pub const DEFAULT_ITERATIONS: u32 = 4096;

// Key-derivation "kind" bytes appended to the usage in the DK constant.
const KIND_ENCRYPT: u8 = 0xAA;
const KIND_INTEGRITY: u8 = 0x55;
const KIND_CHECKSUM: u8 = 0x99;

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// The RFC 3961 `n-fold` operation: fold `input` into `out_len` bytes.
pub fn nfold(input: &[u8], out_len: usize) -> Vec<u8> {
    let in_len = input.len();
    if in_len == 0 || out_len == 0 {
        return vec![0u8; out_len];
    }
    let lcm = out_len / gcd(out_len, in_len) * in_len;
    let mut out = vec![0u8; out_len];
    let in_bits = in_len * 8;
    let mut carry: u32 = 0;

    for i in (0..lcm).rev() {
        let msbit =
            ((in_bits - 1) + ((in_bits + 13) * (i / in_len)) + ((in_len - (i % in_len)) * 8))
                % in_bits;
        let hi = input[((in_len - 1) - (msbit / 8)) % in_len] as u32;
        let lo = input[(in_len - (msbit / 8)) % in_len] as u32;
        let val = (((hi << 8) | lo) >> ((msbit & 7) + 1)) & 0xFF;

        carry += val + out[i % out_len] as u32;
        out[i % out_len] = (carry & 0xFF) as u8;
        carry >>= 8;
    }
    if carry != 0 {
        for i in (0..out_len).rev() {
            carry += out[i] as u32;
            out[i] = (carry & 0xFF) as u8;
            carry >>= 8;
        }
    }
    out
}

/// The RFC 3961 `DR` (derive-random) function for AES: iterate the cipher over
/// the n-folded `constant` until `key_len` bytes are produced.
fn dr(base_key: &[u8], constant: &[u8], key_len: usize) -> Vec<u8> {
    let aes = Aes::new(base_key);
    let mut block = [0u8; 16];
    let folded = if constant.len() == 16 {
        constant.to_vec()
    } else {
        nfold(constant, 16)
    };
    block.copy_from_slice(&folded);

    let mut out = Vec::with_capacity(key_len + 16);
    while out.len() < key_len {
        aes.encrypt_block(&mut block);
        out.extend_from_slice(&block);
    }
    out.truncate(key_len);
    out
}

/// The RFC 3961 `DK` (derive-key). For AES, random-to-key is the identity, so
/// this is just `DR`.
pub fn dk(base_key: &[u8], constant: &[u8], key_len: usize) -> Vec<u8> {
    dr(base_key, constant, key_len)
}

/// The RFC 3962 string-to-key: `DK(random-to-key(PBKDF2(...)), "kerberos")`.
pub fn string_to_key(password: &str, salt: &[u8], iterations: u32, key_len: usize) -> Vec<u8> {
    let tkey = pbkdf2_hmac_sha1(password.as_bytes(), salt, iterations, key_len);
    dk(&tkey, b"kerberos", key_len)
}

/// Derive the per-usage key of `kind` from `base_key`.
fn derive(base_key: &[u8], usage: u32, kind: u8, key_len: usize) -> Vec<u8> {
    let mut constant = usage.to_be_bytes().to_vec();
    constant.push(kind);
    dk(base_key, &constant, key_len)
}

fn xor_into(dst: &mut [u8; 16], src: &[u8; 16]) {
    for (d, s) in dst.iter_mut().zip(src.iter()) {
        *d ^= *s;
    }
}

/// AES-CBC encryption with ciphertext stealing (IV = 0), RFC 3962 §5.
fn cbc_cts_encrypt(aes: &Aes, data: &[u8]) -> Vec<u8> {
    let n = data.len();
    debug_assert!(n >= 16);
    let nblocks = n / 16 + usize::from(n % 16 != 0);
    if nblocks == 1 {
        let mut b = [0u8; 16];
        b.copy_from_slice(data);
        aes.encrypt_block(&mut b);
        return b.to_vec();
    }

    let mut out = vec![0u8; n];
    let mut prev = [0u8; 16];
    // All blocks except the last two.
    for i in 0..(nblocks - 2) {
        let mut blk = [0u8; 16];
        blk.copy_from_slice(&data[i * 16..i * 16 + 16]);
        xor_into(&mut blk, &prev);
        aes.encrypt_block(&mut blk);
        out[i * 16..i * 16 + 16].copy_from_slice(&blk);
        prev = blk;
    }
    // Second-to-last (full) block → intermediate E.
    let i = nblocks - 2;
    let mut e = [0u8; 16];
    e.copy_from_slice(&data[i * 16..i * 16 + 16]);
    xor_into(&mut e, &prev);
    aes.encrypt_block(&mut e);
    // Last (possibly short) block, zero-padded, XOR E, encrypt → C[n-1].
    let last = &data[(nblocks - 1) * 16..];
    let m = last.len();
    let mut cn1 = [0u8; 16];
    cn1[..m].copy_from_slice(last);
    xor_into(&mut cn1, &e);
    aes.encrypt_block(&mut cn1);
    // Output the last two ciphertext blocks swapped: C[n-1] then truncated E.
    out[i * 16..i * 16 + 16].copy_from_slice(&cn1);
    out[(nblocks - 1) * 16..].copy_from_slice(&e[..m]);
    out
}

/// AES-CBC decryption with ciphertext stealing (IV = 0).
fn cbc_cts_decrypt(aes: &Aes, data: &[u8]) -> Result<Vec<u8>> {
    let n = data.len();
    if n < 16 {
        return Err(Error::InvalidLength {
            field: "AES-CTS ciphertext",
            length: n,
        });
    }
    let nblocks = n / 16 + usize::from(n % 16 != 0);
    if nblocks == 1 {
        let mut b = [0u8; 16];
        b.copy_from_slice(data);
        aes.decrypt_block(&mut b);
        return Ok(b.to_vec());
    }

    let mut out = vec![0u8; n];
    let mut prev = [0u8; 16];
    for i in 0..(nblocks - 2) {
        let mut blk = [0u8; 16];
        blk.copy_from_slice(&data[i * 16..i * 16 + 16]);
        let saved = blk;
        aes.decrypt_block(&mut blk);
        xor_into(&mut blk, &prev);
        out[i * 16..i * 16 + 16].copy_from_slice(&blk);
        prev = saved;
    }
    // Last two ciphertext blocks: C[n-1] (full) and C[n] (m bytes).
    let cn1 = &data[(nblocks - 2) * 16..(nblocks - 2) * 16 + 16];
    let cn = &data[(nblocks - 1) * 16..];
    let m = cn.len();

    let mut dn1 = [0u8; 16];
    dn1.copy_from_slice(cn1);
    aes.decrypt_block(&mut dn1); // = Pn_pad XOR E
                                 // E = C[n] (first m) || Dn1[m..].
    let mut e = [0u8; 16];
    e[..m].copy_from_slice(cn);
    e[m..].copy_from_slice(&dn1[m..]);
    // P[n] = first m bytes of (Dn1 XOR E).
    let mut pn_pad = dn1;
    xor_into(&mut pn_pad, &e);
    // P[n-1] = decrypt(E) XOR prev.
    let mut pn1 = e;
    aes.decrypt_block(&mut pn1);
    xor_into(&mut pn1, &prev);

    out[(nblocks - 2) * 16..(nblocks - 2) * 16 + 16].copy_from_slice(&pn1);
    out[(nblocks - 1) * 16..].copy_from_slice(&pn_pad[..m]);
    Ok(out)
}

/// An AES Kerberos key (etype 17 or 18) ready to encrypt / decrypt / checksum.
pub struct AesKey {
    base: Vec<u8>,
    key_len: usize,
    etype: i32,
}

impl AesKey {
    fn key_len_for(etype: i32) -> Result<usize> {
        match etype {
            ETYPE_AES128_CTS_HMAC_SHA1_96 => Ok(16),
            ETYPE_AES256_CTS_HMAC_SHA1_96 => Ok(32),
            other => Err(Error::InvalidValue {
                field: "AES etype",
                value: other.to_string(),
            }),
        }
    }

    /// Build from an already-derived base key (its length must match `etype`).
    pub fn from_key(etype: i32, base: Vec<u8>) -> Result<Self> {
        let key_len = Self::key_len_for(etype)?;
        if base.len() != key_len {
            return Err(Error::InvalidLength {
                field: "AES base key",
                length: base.len(),
            });
        }
        Ok(AesKey {
            base,
            key_len,
            etype,
        })
    }

    /// Derive the key from a password and salt using the default iteration
    /// count.
    pub fn from_password(etype: i32, password: &str, salt: &[u8]) -> Result<Self> {
        let key_len = Self::key_len_for(etype)?;
        let base = string_to_key(password, salt, DEFAULT_ITERATIONS, key_len);
        Ok(AesKey {
            base,
            key_len,
            etype,
        })
    }

    /// The encryption type number.
    pub fn etype(&self) -> i32 {
        self.etype
    }

    /// The raw base key bytes.
    pub fn key(&self) -> &[u8] {
        &self.base
    }

    /// Encrypt `plaintext` for key `usage`, prepending `confounder` (16 bytes).
    /// Output is `AES-CTS(Ke, confounder||plaintext) || HMAC-SHA1-96(Ki, ...)`.
    pub fn encrypt(&self, usage: u32, plaintext: &[u8], confounder: &[u8]) -> Vec<u8> {
        let ke = derive(&self.base, usage, KIND_ENCRYPT, self.key_len);
        let ki = derive(&self.base, usage, KIND_INTEGRITY, self.key_len);
        let mut p1 = Vec::with_capacity(confounder.len() + plaintext.len());
        p1.extend_from_slice(confounder);
        p1.extend_from_slice(plaintext);

        let aes = Aes::new(&ke);
        let mut out = cbc_cts_encrypt(&aes, &p1);
        let mac = hmac_sha1(&ki, &p1);
        out.extend_from_slice(&mac[..MAC_LEN]);
        out
    }

    /// Decrypt an AES token for key `usage`, verifying the MAC and stripping
    /// the confounder.
    pub fn decrypt(&self, usage: u32, token: &[u8]) -> Result<Vec<u8>> {
        if token.len() < CONFOUNDER_LEN + MAC_LEN {
            return Err(Error::InvalidLength {
                field: "AES token",
                length: token.len(),
            });
        }
        let split = token.len() - MAC_LEN;
        let ciphertext = &token[..split];
        let tag = &token[split..];

        let ke = derive(&self.base, usage, KIND_ENCRYPT, self.key_len);
        let ki = derive(&self.base, usage, KIND_INTEGRITY, self.key_len);
        let aes = Aes::new(&ke);
        let p1 = cbc_cts_decrypt(&aes, ciphertext)?;
        let mac = hmac_sha1(&ki, &p1);
        if mac[..MAC_LEN] != *tag {
            return Err(Error::InvalidValue {
                field: "AES token MAC",
                value: "verification failed".to_string(),
            });
        }
        Ok(p1[CONFOUNDER_LEN..].to_vec())
    }

    /// Compute the HMAC-SHA1-96 keyed checksum of `data` for key `usage`.
    pub fn checksum(&self, usage: u32, data: &[u8]) -> Vec<u8> {
        let kc = derive(&self.base, usage, KIND_CHECKSUM, self.key_len);
        hmac_sha1(&kc, data)[..MAC_LEN].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn nfold_rfc3961_vectors() {
        assert_eq!(hex(&nfold(b"012345", 8)), "be072631276b1955");
        assert_eq!(hex(&nfold(b"password", 7)), "78a07b6caf85fa");
        assert_eq!(
            hex(&nfold(b"Rough Consensus, and Running Code", 8)),
            "bb6ed30870b7f0e0"
        );
        // 64-fold of an 8-byte string is the string itself.
        assert_eq!(hex(&nfold(b"kerberos", 8)), "6b65726265726f73");
        assert_eq!(
            hex(&nfold(b"kerberos", 16)),
            "6b65726265726f737b9b5b2b93132b93"
        );
        assert_eq!(
            hex(&nfold(b"password", 21)),
            "59e4a8ca7c0385c3c37b3f6d2000247cb6e6bd5b3e"
        );
    }

    #[test]
    fn string_to_key_rfc3962_vectors() {
        let salt = b"ATHENA.MIT.EDUraeburn";
        // iterations = 1.
        assert_eq!(
            hex(&string_to_key("password", salt, 1, 16)),
            "42263c6e89f4fc28b8df68ee09799f15"
        );
        assert_eq!(
            hex(&string_to_key("password", salt, 1, 32)),
            "fe697b52bc0d3ce14432ba036a92e65bbb52280990a2fa27883998d72af30161"
        );
        // iterations = 1200.
        assert_eq!(
            hex(&string_to_key("password", salt, 1200, 16)),
            "4c01cd46d632d01e6dbe230a01ed642a"
        );
        assert_eq!(
            hex(&string_to_key("password", salt, 1200, 32)),
            "55a6ac740ad17b4846941051e1e8b0a7548d93b0ab30a8bc3ff16280382b8c2a"
        );
    }

    #[test]
    fn encrypt_decrypt_roundtrip_aes128() {
        let key =
            AesKey::from_password(ETYPE_AES128_CTS_HMAC_SHA1_96, "password", b"salt").unwrap();
        // A plaintext that is not a block multiple, to exercise CTS.
        let plaintext = b"kerberos authenticator payload of odd length!";
        let conf = [0x11u8; CONFOUNDER_LEN];
        let token = key.encrypt(4, plaintext, &conf);
        assert_eq!(token.len(), CONFOUNDER_LEN + plaintext.len() + 12);
        assert_eq!(key.decrypt(4, &token).unwrap(), plaintext);
    }

    #[test]
    fn encrypt_decrypt_roundtrip_aes256_various_lengths() {
        let key =
            AesKey::from_password(ETYPE_AES256_CTS_HMAC_SHA1_96, "hunter2", b"REALMuser").unwrap();
        let conf = [0x22u8; CONFOUNDER_LEN];
        for len in [0usize, 1, 15, 16, 17, 31, 32, 100] {
            let plaintext = vec![0x5Au8; len];
            let token = key.encrypt(7, &plaintext, &conf);
            assert_eq!(key.decrypt(7, &token).unwrap(), plaintext);
        }
    }

    #[test]
    fn tampered_token_is_rejected() {
        let key = AesKey::from_password(ETYPE_AES128_CTS_HMAC_SHA1_96, "pw", b"salt").unwrap();
        let mut token = key.encrypt(2, b"secret data", &[0x33u8; CONFOUNDER_LEN]);
        let last = token.len() - 1;
        token[last] ^= 0xFF;
        assert!(key.decrypt(2, &token).is_err());
    }

    #[test]
    fn checksum_is_deterministic_and_usage_bound() {
        let key = AesKey::from_password(ETYPE_AES256_CTS_HMAC_SHA1_96, "pw", b"salt").unwrap();
        let a = key.checksum(6, b"authenticator");
        assert_eq!(a.len(), 12);
        assert_eq!(a, key.checksum(6, b"authenticator"));
        assert_ne!(a, key.checksum(7, b"authenticator"));
    }
}
