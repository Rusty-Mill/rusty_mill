//! Port of `src/crypt` — key derivation and authenticated encryption.
//!
//! Wire-compatible with croc v10:
//! * `new_key`: PBKDF2-HMAC-SHA256, 100 iterations, 32-byte key, 8-byte salt.
//! * `encrypt`/`decrypt`: AES-256-GCM, output is `12-byte nonce || ciphertext || tag`.
//! * `new_argon2` + `encrypt_chacha`/`decrypt_chacha`: Argon2id (t=1, m=64 MiB, p=4)
//!   keyed XChaCha20-Poly1305, output is `24-byte nonce || ciphertext || tag`.

use aes_gcm::aead::{Aead, KeyInit, Nonce, Payload};
use aes_gcm::Aes256Gcm;
use chacha20poly1305::XChaCha20Poly1305;
use rand::RngCore;
use sha2::Sha256;

#[derive(Debug)]
pub enum CryptError {
    EmptyPassphrase,
    TooShort,
    Decryption,
    KeyDerivation,
}

impl std::fmt::Display for CryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptError::EmptyPassphrase => write!(f, "need more than that for passphrase"),
            CryptError::TooShort => write!(f, "incorrect passphrase"),
            CryptError::Decryption => write!(f, "decryption failed"),
            CryptError::KeyDerivation => write!(f, "key derivation failed"),
        }
    }
}

impl std::error::Error for CryptError {}

/// Derive a 32-byte key from a passphrase with PBKDF2-HMAC-SHA256 (100 rounds).
/// When `user_salt` is `None` a random 8-byte salt is generated, mirroring
/// `crypt.New` in Go. Returns `(key, salt)`.
pub fn new_key(
    passphrase: &[u8],
    user_salt: Option<&[u8]>,
) -> Result<(Vec<u8>, Vec<u8>), CryptError> {
    if passphrase.is_empty() {
        return Err(CryptError::EmptyPassphrase);
    }
    let salt = match user_salt {
        Some(s) => s.to_vec(),
        None => {
            let mut s = vec![0u8; 8];
            rand::thread_rng().fill_bytes(&mut s);
            s
        }
    };
    let mut key = vec![0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(passphrase, &salt, 100, &mut key);
    Ok((key, salt))
}

/// AES-256-GCM encrypt with a random 12-byte nonce prepended to the output.
pub fn encrypt(plaintext: &[u8], key: &[u8]) -> Result<Vec<u8>, CryptError> {
    let mut iv = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut iv);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CryptError::KeyDerivation)?;
    let nonce: &Nonce<Aes256Gcm> = (&iv[..]).into();
    let ct = cipher
        .encrypt(nonce, Payload::from(plaintext))
        .map_err(|_| CryptError::Decryption)?;
    let mut out = iv.to_vec();
    out.extend_from_slice(&ct);
    Ok(out)
}

/// AES-256-GCM decrypt of `12-byte nonce || ciphertext || tag`.
pub fn decrypt(encrypted: &[u8], key: &[u8]) -> Result<Vec<u8>, CryptError> {
    if encrypted.len() < 13 {
        return Err(CryptError::TooShort);
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CryptError::KeyDerivation)?;
    let nonce: &Nonce<Aes256Gcm> = (&encrypted[..12]).into();
    cipher
        .decrypt(nonce, Payload::from(&encrypted[12..]))
        .map_err(|_| CryptError::Decryption)
}

/// Derive an XChaCha20-Poly1305 cipher from a passphrase using Argon2id with
/// croc's parameters (t=1, m=64 MiB, p=4, 32-byte output). Returns the cipher
/// and the salt used, mirroring `crypt.NewArgon2`.
pub fn new_argon2(
    passphrase: &[u8],
    user_salt: Option<&[u8]>,
) -> Result<(XChaCha20Poly1305, Vec<u8>), CryptError> {
    if passphrase.is_empty() {
        return Err(CryptError::EmptyPassphrase);
    }
    let salt = match user_salt {
        Some(s) => s.to_vec(),
        None => {
            let mut s = vec![0u8; 8];
            rand::thread_rng().fill_bytes(&mut s);
            s
        }
    };
    let params =
        argon2::Params::new(64 * 1024, 1, 4, Some(32)).map_err(|_| CryptError::KeyDerivation)?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(passphrase, &salt, &mut key)
        .map_err(|_| CryptError::KeyDerivation)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|_| CryptError::KeyDerivation)?;
    Ok((cipher, salt))
}

/// XChaCha20-Poly1305 encrypt with a random 24-byte nonce prepended.
pub fn encrypt_chacha(plaintext: &[u8], cipher: &XChaCha20Poly1305) -> Result<Vec<u8>, CryptError> {
    let mut nonce = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut nonce);
    let xnonce: &chacha20poly1305::XNonce = (&nonce[..]).into();
    let ct = cipher
        .encrypt(xnonce, Payload::from(plaintext))
        .map_err(|_| CryptError::Decryption)?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    Ok(out)
}

/// XChaCha20-Poly1305 decrypt of `24-byte nonce || ciphertext || tag`.
pub fn decrypt_chacha(encrypted: &[u8], cipher: &XChaCha20Poly1305) -> Result<Vec<u8>, CryptError> {
    if encrypted.len() < 24 {
        return Err(CryptError::TooShort);
    }
    let xnonce: &chacha20poly1305::XNonce = (&encrypted[..24]).into();
    cipher
        .decrypt(xnonce, Payload::from(&encrypted[24..]))
        .map_err(|_| CryptError::Decryption)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_aes() {
        let (key, salt) = new_key(b"passphrase", None).unwrap();
        assert_eq!(key.len(), 32);
        assert_eq!(salt.len(), 8);
        let enc = encrypt(b"hello, world", &key).unwrap();
        assert_eq!(decrypt(&enc, &key).unwrap(), b"hello, world");
    }

    #[test]
    fn known_salt_derives_same_key() {
        let (k1, s1) = new_key(b"pass123", Some(b"saltsalt")).unwrap();
        let (k2, _) = new_key(b"pass123", Some(&s1)).unwrap();
        assert_eq!(k1, k2);
    }

    // Vector generated with croc's Go implementation:
    //   key, _, _ := crypt.New([]byte("pass123"), []byte("saltsalt"))
    #[test]
    fn go_pbkdf2_vector() {
        let (key, _) = new_key(b"pass123", Some(b"saltsalt")).unwrap();
        assert_eq!(
            hex::encode(&key),
            "394f0fd1e6e81e49e275148937e4d56640afa2ef682eb4f3265b1476a2a0624e"
        );
    }

    // Ciphertext generated with croc's Go crypt.Encrypt using the key above;
    // proves the Rust side can open Go-sealed AES-GCM messages.
    #[test]
    fn go_aes_gcm_vector() {
        let (key, _) = new_key(b"pass123", Some(b"saltsalt")).unwrap();
        let enc = hex::decode(
            "4d9375cf1254a2ea60577bef1603f136295df5cb929f3398988692db7fb64691c15228a00f15ded8",
        )
        .unwrap();
        assert_eq!(decrypt(&enc, &key).unwrap(), b"hello, world");
    }

    #[test]
    fn round_trip_chacha() {
        let (cipher, salt) = new_argon2(b"passphrase", None).unwrap();
        let (cipher2, _) = new_argon2(b"passphrase", Some(&salt)).unwrap();
        let enc = encrypt_chacha(b"big secret", &cipher).unwrap();
        assert_eq!(decrypt_chacha(&enc, &cipher2).unwrap(), b"big secret");
    }
}
