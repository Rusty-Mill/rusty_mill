//! Kerberos GSS-API per-message tokens (RFC 4121 "CFX"), std-only.
//!
//! Once a Kerberos context is established, CredSSP protects the public-key
//! confirmation and the delegated credentials with GSS per-message tokens
//! sealed by the Kerberos session key — the same role NTLM's
//! `EncryptMessage` plays. This module implements the version 2 ("CFX")
//! [`wrap`] / [`unwrap`] (confidentiality) and [`mic`] / [`verify_mic`]
//! (integrity-only) tokens over the AES encryption profile.
//!
//! Only the sealed Wrap form is produced (CredSSP always seals). The Extra
//! Count is emitted as zero; a non-zero Right Rotation Count is handled on
//! receive.

use super::aes::AesKey;
use crate::error::{Error, Result};

/// `KG_USAGE_ACCEPTOR_SEAL` — key usage for acceptor→initiator Wrap.
pub const KG_USAGE_ACCEPTOR_SEAL: u32 = 22;
/// `KG_USAGE_ACCEPTOR_SIGN` — key usage for acceptor→initiator MIC.
pub const KG_USAGE_ACCEPTOR_SIGN: u32 = 23;
/// `KG_USAGE_INITIATOR_SEAL` — key usage for initiator→acceptor Wrap.
pub const KG_USAGE_INITIATOR_SEAL: u32 = 24;
/// `KG_USAGE_INITIATOR_SIGN` — key usage for initiator→acceptor MIC.
pub const KG_USAGE_INITIATOR_SIGN: u32 = 25;

const TOK_ID_WRAP: [u8; 2] = [0x05, 0x04];
const TOK_ID_MIC: [u8; 2] = [0x04, 0x04];

// Token flags (RFC 4121 4.2.2).
const FLAG_SENT_BY_ACCEPTOR: u8 = 0x01;
const FLAG_SEALED: u8 = 0x02;
const FLAG_ACCEPTOR_SUBKEY: u8 = 0x04;

/// The fixed length of both the Wrap and MIC token headers.
const HEADER_LEN: usize = 16;

/// A key that can seal/sign GSS per-message tokens (the AES profile).
pub trait CfxKey {
    /// Encrypt `plaintext` for `usage`, prepending `confounder`.
    fn seal(&self, usage: u32, plaintext: &[u8], confounder: &[u8]) -> Vec<u8>;
    /// Decrypt and verify a token for `usage`.
    fn unseal(&self, usage: u32, token: &[u8]) -> Result<Vec<u8>>;
    /// The keyed checksum of `data` for `usage`.
    fn mac(&self, usage: u32, data: &[u8]) -> Vec<u8>;
    /// Confounder length for this profile.
    fn confounder_len(&self) -> usize;
}

impl CfxKey for AesKey {
    fn seal(&self, usage: u32, plaintext: &[u8], confounder: &[u8]) -> Vec<u8> {
        self.encrypt(usage, plaintext, confounder)
    }
    fn unseal(&self, usage: u32, token: &[u8]) -> Result<Vec<u8>> {
        self.decrypt(usage, token)
    }
    fn mac(&self, usage: u32, data: &[u8]) -> Vec<u8> {
        self.checksum(usage, data)
    }
    fn confounder_len(&self) -> usize {
        super::aes::CONFOUNDER_LEN
    }
}

/// Build a 16-byte Wrap token header.
fn wrap_header(flags: u8, ec: u16, rrc: u16, seq: u64) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[0..2].copy_from_slice(&TOK_ID_WRAP);
    h[2] = flags;
    h[3] = 0xFF;
    h[4..6].copy_from_slice(&ec.to_be_bytes());
    h[6..8].copy_from_slice(&rrc.to_be_bytes());
    h[8..16].copy_from_slice(&seq.to_be_bytes());
    h
}

/// Rotate `data` right by `count` octets (RFC 4121 RRC). We emit RRC = 0, so
/// this inverse of [`rotate_left`] is only exercised by tests.
#[cfg(test)]
fn rotate_right(data: &[u8], count: usize) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }
    let r = count % data.len();
    let split = data.len() - r;
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[split..]);
    out.extend_from_slice(&data[..split]);
    out
}

/// Rotate `data` left by `count` octets (the inverse of [`rotate_right`]).
fn rotate_left(data: &[u8], count: usize) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }
    let r = count % data.len();
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[r..]);
    out.extend_from_slice(&data[..r]);
    out
}

/// Produce a sealed CFX Wrap token for `plaintext`.
///
/// `from_acceptor` sets the acceptor-origin flag; `acceptor_subkey` sets the
/// subkey flag. `seq` is the sequence number and `confounder` supplies the
/// per-message randomness (use the profile's [`CfxKey::confounder_len`] bytes).
pub fn wrap(
    key: &impl CfxKey,
    usage: u32,
    seq: u64,
    from_acceptor: bool,
    acceptor_subkey: bool,
    plaintext: &[u8],
    confounder: &[u8],
) -> Vec<u8> {
    let mut flags = FLAG_SEALED;
    if from_acceptor {
        flags |= FLAG_SENT_BY_ACCEPTOR;
    }
    if acceptor_subkey {
        flags |= FLAG_ACCEPTOR_SUBKEY;
    }
    // EC and RRC are emitted as zero; the header (RRC = 0) is appended to the
    // plaintext before encryption.
    let header = wrap_header(flags, 0, 0, seq);
    let mut tbe = Vec::with_capacity(plaintext.len() + HEADER_LEN);
    tbe.extend_from_slice(plaintext);
    tbe.extend_from_slice(&header);

    let sealed = key.seal(usage, &tbe, confounder);
    let mut token = header.to_vec();
    token.extend_from_slice(&sealed);
    token
}

/// Verify and open a sealed CFX Wrap token, returning the plaintext.
pub fn unwrap(key: &impl CfxKey, usage: u32, token: &[u8]) -> Result<Vec<u8>> {
    if token.len() < HEADER_LEN {
        return Err(Error::InvalidLength {
            field: "CFX Wrap token",
            length: token.len(),
        });
    }
    let header = &token[..HEADER_LEN];
    if header[0..2] != TOK_ID_WRAP {
        return Err(Error::InvalidValue {
            field: "CFX token id",
            value: format!("{:02X?}", &header[0..2]),
        });
    }
    if header[2] & FLAG_SEALED == 0 {
        return Err(Error::InvalidValue {
            field: "CFX Wrap flags",
            value: "not sealed".to_string(),
        });
    }
    let ec = u16::from_be_bytes([header[4], header[5]]) as usize;
    let rrc = u16::from_be_bytes([header[6], header[7]]) as usize;

    // Undo the right rotation, then decrypt.
    let rotated = rotate_left(&token[HEADER_LEN..], rrc);
    let dec = key.unseal(usage, &rotated)?;
    if dec.len() < HEADER_LEN + ec {
        return Err(Error::InvalidLength {
            field: "CFX Wrap plaintext",
            length: dec.len(),
        });
    }
    // Trailing: plaintext | EC filler | 16-byte header copy.
    let plain_len = dec.len() - HEADER_LEN - ec;
    if dec[plain_len + ec..plain_len + ec + 2] != TOK_ID_WRAP {
        return Err(Error::InvalidValue {
            field: "CFX Wrap trailer",
            value: "header mismatch".to_string(),
        });
    }
    Ok(dec[..plain_len].to_vec())
}

/// Build a 16-byte MIC token header.
fn mic_header(flags: u8, seq: u64) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[0..2].copy_from_slice(&TOK_ID_MIC);
    h[2] = flags;
    for b in h.iter_mut().take(8).skip(3) {
        *b = 0xFF;
    }
    h[8..16].copy_from_slice(&seq.to_be_bytes());
    h
}

/// Produce a CFX MIC token over `message` (integrity only).
pub fn mic(
    key: &impl CfxKey,
    usage: u32,
    seq: u64,
    from_acceptor: bool,
    message: &[u8],
) -> Vec<u8> {
    let flags = if from_acceptor {
        FLAG_SENT_BY_ACCEPTOR
    } else {
        0
    };
    let header = mic_header(flags, seq);
    let mut signed = Vec::with_capacity(message.len() + HEADER_LEN);
    signed.extend_from_slice(message);
    signed.extend_from_slice(&header);
    let checksum = key.mac(usage, &signed);

    let mut token = header.to_vec();
    token.extend_from_slice(&checksum);
    token
}

/// Verify a CFX MIC token against `message`.
pub fn verify_mic(key: &impl CfxKey, usage: u32, message: &[u8], token: &[u8]) -> Result<()> {
    if token.len() < HEADER_LEN {
        return Err(Error::InvalidLength {
            field: "CFX MIC token",
            length: token.len(),
        });
    }
    let header = &token[..HEADER_LEN];
    if header[0..2] != TOK_ID_MIC {
        return Err(Error::InvalidValue {
            field: "CFX token id",
            value: format!("{:02X?}", &header[0..2]),
        });
    }
    let received = &token[HEADER_LEN..];
    let mut signed = Vec::with_capacity(message.len() + HEADER_LEN);
    signed.extend_from_slice(message);
    signed.extend_from_slice(header);
    let expected = key.mac(usage, &signed);
    if expected != received {
        return Err(Error::InvalidValue {
            field: "CFX MIC",
            value: "verification failed".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::krb5::aes::{AesKey, ETYPE_AES256_CTS_HMAC_SHA1_96};

    fn key() -> AesKey {
        AesKey::from_password(ETYPE_AES256_CTS_HMAC_SHA1_96, "password", b"REALMuser").unwrap()
    }

    #[test]
    fn rotate_is_reversible() {
        let data = [1u8, 2, 3, 4, 5, 6, 7];
        for r in 0..20 {
            assert_eq!(rotate_left(&rotate_right(&data, r), r), data);
        }
    }

    #[test]
    fn wrap_unwrap_roundtrip() {
        let key = key();
        let conf = [0x11u8; 16];
        let token = wrap(
            &key,
            KG_USAGE_INITIATOR_SEAL,
            0,
            false,
            true,
            b"public key blob",
            &conf,
        );
        assert_eq!(&token[0..2], &TOK_ID_WRAP);
        assert!(token[2] & FLAG_SEALED != 0);
        assert!(token[2] & FLAG_ACCEPTOR_SUBKEY != 0);
        let out = unwrap(&key, KG_USAGE_INITIATOR_SEAL, &token).unwrap();
        assert_eq!(out, b"public key blob");
    }

    #[test]
    fn unwrap_handles_right_rotation() {
        // Emit a token, then re-rotate its encrypted part with a non-zero RRC
        // and set the header field to match: unwrap must still recover it.
        let key = key();
        let token = wrap(
            &key,
            KG_USAGE_INITIATOR_SEAL,
            5,
            false,
            false,
            b"rotate me please",
            &[0x22u8; 16],
        );
        let rrc = 7usize;
        let mut rotated = token[..HEADER_LEN].to_vec();
        rotated[6..8].copy_from_slice(&(rrc as u16).to_be_bytes());
        rotated.extend_from_slice(&rotate_right(&token[HEADER_LEN..], rrc));
        assert_eq!(
            unwrap(&key, KG_USAGE_INITIATOR_SEAL, &rotated).unwrap(),
            b"rotate me please"
        );
    }

    #[test]
    fn wrap_tamper_is_rejected() {
        let key = key();
        let mut token = wrap(
            &key,
            KG_USAGE_INITIATOR_SEAL,
            0,
            false,
            false,
            b"secret",
            &[0x33u8; 16],
        );
        let last = token.len() - 1;
        token[last] ^= 0xFF;
        assert!(unwrap(&key, KG_USAGE_INITIATOR_SEAL, &token).is_err());
    }

    #[test]
    fn wrong_usage_fails() {
        let key = key();
        let token = wrap(
            &key,
            KG_USAGE_INITIATOR_SEAL,
            0,
            false,
            false,
            b"data",
            &[0x44u8; 16],
        );
        // The acceptor-seal usage derives different keys, so verification fails.
        assert!(unwrap(&key, KG_USAGE_ACCEPTOR_SEAL, &token).is_err());
    }

    #[test]
    fn mic_roundtrip_and_tamper() {
        let key = key();
        let token = mic(
            &key,
            KG_USAGE_INITIATOR_SIGN,
            1,
            false,
            b"mechListMIC input",
        );
        assert_eq!(&token[0..2], &TOK_ID_MIC);
        assert!(verify_mic(&key, KG_USAGE_INITIATOR_SIGN, b"mechListMIC input", &token).is_ok());
        assert!(verify_mic(&key, KG_USAGE_INITIATOR_SIGN, b"different message", &token).is_err());
    }
}
