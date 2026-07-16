//! RDP standard security (MS-RDPBCGR §2.2.1.10, §5.3), std-only.
//!
//! This module implements the classic (non-TLS) RDP security path:
//!
//! 1. Parse the server's RSA public key out of the `TS_UD_SC_SEC1`
//!    certificate ([`parse_server_certificate`]).
//! 2. RSA-encrypt a freshly generated 32-byte client random and send it in
//!    the **Security Exchange PDU** ([`encode_security_exchange`]).
//! 3. Derive the session keys from the client and server randoms
//!    ([`derive_session_keys`]) and use them to RC4-encrypt and MAC every
//!    subsequent PDU ([`Rc4Session`]).
//!
//! ## Security warning
//!
//! Standard security is obsolete and weak (RC4, MD5/SHA-1 MACs, no forward
//! secrecy). It is implemented for interoperability with servers that still
//! offer it; new deployments should negotiate TLS/CredSSP instead. See
//! [`crate::crypto`] for the caveats on the primitives.

use crate::crypto::bignum::BigUint;
use crate::crypto::{md5::Md5, rc4::Rc4, sha1::Sha1};
use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// Length of the client/server random values in bytes.
pub const RANDOM_LEN: usize = 32;

// Basic Security Header flags (MS-RDPBCGR 2.2.8.1.1.2.1).
/// `SEC_EXCHANGE_PKT` — this PDU is a Security Exchange.
pub const SEC_EXCHANGE_PKT: u16 = 0x0001;
/// `SEC_ENCRYPT` — the payload is RC4-encrypted and carries a MAC.
pub const SEC_ENCRYPT: u16 = 0x0008;
/// `SEC_INFO_PKT` — this PDU is the Client Info PDU.
pub const SEC_INFO_PKT: u16 = 0x0040;
/// `SEC_LICENSE_PKT` — this PDU is part of the licensing exchange.
pub const SEC_LICENSE_PKT: u16 = 0x0080;

// Encryption methods come from the GCC layer (MS-RDPBCGR 2.2.1.4.3); the
// weakening logic only needs the 40- and 56-bit ones.
use crate::gcc::{ENCRYPTION_METHOD_40BIT, ENCRYPTION_METHOD_56BIT};

const RSA_MAGIC: u32 = 0x3141_5352; // "RSA1"
const CERT_TYPE_PROPRIETARY: u32 = 1;
const CERT_TYPE_X509: u32 = 2;

const PAD1: [u8; 40] = [0x36; 40];
const PAD2: [u8; 48] = [0x5C; 48];

// ---------------------------------------------------------------------------
// RSA public key + server certificate
// ---------------------------------------------------------------------------

/// An RSA public key extracted from a server certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsaPublicKey {
    /// Modulus `n`, little-endian, with the trailing zero padding stripped.
    pub modulus_le: Vec<u8>,
    /// Public exponent `e` (conventionally 65537).
    pub exponent: u32,
}

impl RsaPublicKey {
    /// The modulus length in bytes, which is also the ciphertext length.
    pub fn key_length(&self) -> usize {
        self.modulus_le.len()
    }

    /// RSA-encrypt `data` (`c = m^e mod n`), returning `key_length()`
    /// little-endian bytes. `data` is interpreted as a little-endian integer
    /// and must be smaller than the modulus.
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        let m = BigUint::from_bytes_le(data);
        let n = BigUint::from_bytes_le(&self.modulus_le);
        let e = BigUint::from_bytes_le(&self.exponent.to_le_bytes());
        let c = m.modpow(&e, &n);
        c.to_bytes_le(self.key_length()).ok_or(Error::InvalidValue {
            field: "RSA ciphertext",
            value: "exceeds modulus length".to_string(),
        })
    }
}

/// Parse a `TS_UD_SC_SEC1` server certificate and return its RSA public key.
///
/// Only the Proprietary Server Certificate (`dwVersion` type 1,
/// MS-RDPBCGR 2.2.1.4.3.1.1) is supported; X.509 certificate chains return an
/// error for now. The certificate signature is not verified.
pub fn parse_server_certificate(cert: &[u8]) -> Result<RsaPublicKey> {
    let mut r = Reader::new(cert);
    let dw_version = r.read_u32_le()?;
    let cert_type = dw_version & 0x7FFF_FFFF;
    match cert_type {
        CERT_TYPE_PROPRIETARY => parse_proprietary_certificate(&mut r),
        CERT_TYPE_X509 => Err(Error::InvalidValue {
            field: "server certificate",
            value: "X.509 chain not yet supported".to_string(),
        }),
        other => Err(Error::InvalidValue {
            field: "server certificate version",
            value: other.to_string(),
        }),
    }
}

fn parse_proprietary_certificate(r: &mut Reader<'_>) -> Result<RsaPublicKey> {
    let _sig_alg_id = r.read_u32_le()?;
    let _key_alg_id = r.read_u32_le()?;
    let _pubkey_blob_type = r.read_u16_le()?;
    let _pubkey_blob_len = r.read_u16_le()?;

    let magic = r.read_u32_le()?;
    if magic != RSA_MAGIC {
        return Err(Error::InvalidValue {
            field: "RSA public key magic",
            value: format!("0x{magic:08X}"),
        });
    }
    let keylen = r.read_u32_le()? as usize;
    let bitlen = r.read_u32_le()? as usize;
    let _datalen = r.read_u32_le()?;
    let exponent = r.read_u32_le()?;
    let modulus_field = r.read_bytes(keylen)?;

    // The real modulus is `bitlen / 8` bytes; the field is padded with 8
    // trailing zero bytes (`keylen == bitlen/8 + 8`).
    let mod_len = bitlen / 8;
    if mod_len == 0 || mod_len > modulus_field.len() {
        return Err(Error::InvalidLength {
            field: "RSA modulus",
            length: mod_len,
        });
    }
    Ok(RsaPublicKey {
        modulus_le: modulus_field[..mod_len].to_vec(),
        exponent,
    })
}

// ---------------------------------------------------------------------------
// Key derivation (MS-RDPBCGR 5.3.5)
// ---------------------------------------------------------------------------

/// The RC4 keys and MAC key derived for one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionKeys {
    /// Key used to compute and verify MAC signatures.
    pub mac_key: Vec<u8>,
    /// Key the client uses to encrypt outbound data.
    pub encrypt_key: Vec<u8>,
    /// Key the client uses to decrypt inbound data.
    pub decrypt_key: Vec<u8>,
}

/// `SaltedHash(S, I) = MD5(S + SHA1(I + S + ClientRandom + ServerRandom))`.
fn salted_hash(salt: &[u8], input: &[u8], client_random: &[u8], server_random: &[u8]) -> [u8; 16] {
    let mut sha = Sha1::new();
    sha.update(input);
    sha.update(salt);
    sha.update(client_random);
    sha.update(server_random);
    let inner = sha.finish();

    let mut md = Md5::new();
    md.update(salt);
    md.update(&inner);
    md.finish()
}

/// `FinalHash(K) = MD5(K + ClientRandom + ServerRandom)`.
fn final_hash(key: &[u8], client_random: &[u8], server_random: &[u8]) -> [u8; 16] {
    let mut md = Md5::new();
    md.update(key);
    md.update(client_random);
    md.update(server_random);
    md.finish()
}

/// Derive the session keys from the two randoms and the negotiated
/// `encryption_method` (from `TS_UD_SC_SEC1`).
///
/// Implements the master-secret / session-key-blob construction and the
/// 40/56-bit key weakening. The keys are derived from the *client's*
/// perspective (encrypt = client→server, decrypt = server→client).
pub fn derive_session_keys(
    client_random: &[u8; RANDOM_LEN],
    server_random: &[u8; RANDOM_LEN],
    encryption_method: u32,
) -> SessionKeys {
    // PreMasterSecret = First192Bits(client) + First192Bits(server).
    let mut pre_master = Vec::with_capacity(48);
    pre_master.extend_from_slice(&client_random[..24]);
    pre_master.extend_from_slice(&server_random[..24]);

    let master = concat3(
        salted_hash(&pre_master, b"A", client_random, server_random),
        salted_hash(&pre_master, b"BB", client_random, server_random),
        salted_hash(&pre_master, b"CCC", client_random, server_random),
    );
    let session = concat3(
        salted_hash(&master, b"X", client_random, server_random),
        salted_hash(&master, b"YY", client_random, server_random),
        salted_hash(&master, b"ZZZ", client_random, server_random),
    );

    let mac_key_full = &session[0..16];
    // Client encrypt key = FinalHash(third 128 bits); decrypt = second.
    let mut encrypt_key = final_hash(&session[32..48], client_random, server_random).to_vec();
    let mut decrypt_key = final_hash(&session[16..32], client_random, server_random).to_vec();

    let mac_key = match encryption_method {
        ENCRYPTION_METHOD_40BIT | ENCRYPTION_METHOD_56BIT => {
            weaken_key(&mut encrypt_key, encryption_method);
            weaken_key(&mut decrypt_key, encryption_method);
            // 40/56-bit sessions use an 8-byte MAC key.
            mac_key_full[..8].to_vec()
        }
        _ => mac_key_full.to_vec(),
    };

    SessionKeys {
        mac_key,
        encrypt_key,
        decrypt_key,
    }
}

/// Overwrite the leading key bytes with the fixed "salt" that reduces a
/// 128-bit key to 40- or 56-bit effective strength.
fn weaken_key(key: &mut [u8], method: u32) {
    if method == ENCRYPTION_METHOD_40BIT {
        key[0] = 0xD1;
        key[1] = 0x26;
        key[2] = 0x9E;
    } else if method == ENCRYPTION_METHOD_56BIT {
        key[0] = 0xD1;
    }
}

fn concat3(a: [u8; 16], b: [u8; 16], c: [u8; 16]) -> [u8; 48] {
    let mut out = [0u8; 48];
    out[..16].copy_from_slice(&a);
    out[16..32].copy_from_slice(&b);
    out[32..].copy_from_slice(&c);
    out
}

// ---------------------------------------------------------------------------
// MAC signature (MS-RDPBCGR 5.3.6.1.1)
// ---------------------------------------------------------------------------

/// Compute the 8-byte non-FIPS MAC signature over `data`.
///
/// `MACSignature = First64Bits(MD5(MACKey + PAD2 + SHA1(MACKey + PAD1 +
/// len(data) + data)))`.
pub fn mac_signature(mac_key: &[u8], data: &[u8]) -> [u8; 8] {
    let mut sha = Sha1::new();
    sha.update(mac_key);
    sha.update(&PAD1);
    sha.update(&(data.len() as u32).to_le_bytes());
    sha.update(data);
    let inner = sha.finish();

    let mut md = Md5::new();
    md.update(mac_key);
    md.update(&PAD2);
    md.update(&inner);
    let outer = md.finish();

    let mut sig = [0u8; 8];
    sig.copy_from_slice(&outer[..8]);
    sig
}

// ---------------------------------------------------------------------------
// RC4 session
// ---------------------------------------------------------------------------

/// A pair of RC4 cipher states plus the MAC key for one encrypted session.
///
/// Note this does not implement the periodic key update RDP performs every
/// 4096 packets; long-lived sessions will need that added.
pub struct Rc4Session {
    encrypt: Rc4,
    decrypt: Rc4,
    mac_key: Vec<u8>,
}

impl Rc4Session {
    /// Build a session from derived keys.
    pub fn new(keys: &SessionKeys) -> Self {
        Rc4Session {
            encrypt: Rc4::new(&keys.encrypt_key),
            decrypt: Rc4::new(&keys.decrypt_key),
            mac_key: keys.mac_key.clone(),
        }
    }

    /// MAC then encrypt `plaintext`, returning `(signature, ciphertext)`.
    ///
    /// The MAC is computed over the plaintext (per RDP) before the RC4 pass.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> ([u8; 8], Vec<u8>) {
        let signature = mac_signature(&self.mac_key, plaintext);
        let ciphertext = self.encrypt.applied(plaintext);
        (signature, ciphertext)
    }

    /// Decrypt `ciphertext` and verify its `signature`.
    ///
    /// Returns [`Error::InvalidValue`] if the recomputed MAC does not match.
    pub fn decrypt(&mut self, signature: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
        let plaintext = self.decrypt.applied(ciphertext);
        let expected = mac_signature(&self.mac_key, &plaintext);
        if expected != signature {
            return Err(Error::InvalidValue {
                field: "MAC signature",
                value: "verification failed".to_string(),
            });
        }
        Ok(plaintext)
    }
}

// ---------------------------------------------------------------------------
// Basic Security Header + Security Exchange PDU
// ---------------------------------------------------------------------------

/// The 4-byte Basic Security Header prefixing security-relevant PDUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasicSecurityHeader {
    /// `flags` field (`SEC_*` values).
    pub flags: u16,
    /// `flagsHi` field (reserved / FIPS use, usually 0).
    pub flags_hi: u16,
}

impl BasicSecurityHeader {
    /// Create a header with the given flags and `flagsHi == 0`.
    pub fn new(flags: u16) -> Self {
        BasicSecurityHeader { flags, flags_hi: 0 }
    }

    /// Encode the 4-byte header into `w`.
    pub fn encode(&self, w: &mut Writer) {
        w.write_u16_le(self.flags);
        w.write_u16_le(self.flags_hi);
    }

    /// Decode a 4-byte header from `r`.
    pub fn decode(r: &mut Reader<'_>) -> Result<BasicSecurityHeader> {
        Ok(BasicSecurityHeader {
            flags: r.read_u16_le()?,
            flags_hi: r.read_u16_le()?,
        })
    }
}

/// Encode a Security Exchange PDU (MS-RDPBCGR 2.2.1.10) carrying the
/// RSA-encrypted client random.
///
/// The eight bytes of trailing zero padding RDP mandates are appended here,
/// and the length field is set accordingly.
pub fn encode_security_exchange(encrypted_client_random: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    BasicSecurityHeader::new(SEC_EXCHANGE_PKT).encode(&mut w);
    w.write_u32_le((encrypted_client_random.len() + 8) as u32);
    w.write_bytes(encrypted_client_random);
    w.write_bytes(&[0u8; 8]);
    w.into_vec()
}

/// Decode a Security Exchange PDU, returning the encrypted client random
/// (including its 8-byte trailing padding).
pub fn decode_security_exchange(buf: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(buf);
    let header = BasicSecurityHeader::decode(&mut r)?;
    if header.flags & SEC_EXCHANGE_PKT == 0 {
        return Err(Error::InvalidValue {
            field: "Security Exchange flags",
            value: format!("0x{:04X}", header.flags),
        });
    }
    let length = r.read_u32_le()? as usize;
    Ok(r.read_bytes(length)?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gcc::ENCRYPTION_METHOD_128BIT;

    #[test]
    fn rsa_encrypt_matches_manual_modpow() {
        // n = 3233 = 0x0CA1, e = 17, d = 413.
        let key = RsaPublicKey {
            modulus_le: vec![0xA1, 0x0C],
            exponent: 17,
        };
        assert_eq!(key.key_length(), 2);
        let cipher = key.encrypt(&[65]).unwrap(); // m = 65
                                                  // 65^17 mod 3233 = 2790 = 0x0AE6 -> little-endian [E6, 0A].
        assert_eq!(cipher, [0xE6, 0x0A]);
        // Decrypt with the private exponent to recover m.
        let c = BigUint::from_bytes_le(&cipher);
        let n = BigUint::from_bytes_le(&key.modulus_le);
        let d = BigUint::from_bytes_le(&413u32.to_le_bytes());
        assert_eq!(c.modpow(&d, &n).to_bytes_le(1).unwrap(), [65]);
    }

    #[test]
    fn parse_proprietary_certificate_key() {
        // Hand-build a minimal proprietary certificate with a 64-bit modulus.
        let modulus = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let mut w = Writer::new();
        w.write_u32_le(CERT_TYPE_PROPRIETARY);
        w.write_u32_le(1); // dwSigAlgId
        w.write_u32_le(1); // dwKeyAlgId
        w.write_u16_le(0x0006); // wPublicKeyBlobType
        w.write_u16_le(0); // wPublicKeyBlobLen (unchecked)
        w.write_u32_le(RSA_MAGIC);
        w.write_u32_le(modulus.len() as u32 + 8); // keylen = modlen + 8 pad
        w.write_u32_le(modulus.len() as u32 * 8); // bitlen
        w.write_u32_le(modulus.len() as u32 - 1); // datalen
        w.write_u32_le(65537); // pubExp
        w.write_bytes(&modulus);
        w.write_bytes(&[0u8; 8]); // modulus padding
        w.write_u16_le(0x0008); // signature blob type
        w.write_u16_le(0); // signature blob len

        let key = parse_server_certificate(w.as_slice()).unwrap();
        assert_eq!(key.exponent, 65537);
        assert_eq!(key.modulus_le, modulus);
    }

    #[test]
    fn x509_certificate_is_rejected_clearly() {
        let mut w = Writer::new();
        w.write_u32_le(CERT_TYPE_X509);
        assert!(matches!(
            parse_server_certificate(w.as_slice()).unwrap_err(),
            Error::InvalidValue {
                field: "server certificate",
                ..
            }
        ));
    }

    #[test]
    fn key_derivation_is_deterministic_and_sized() {
        let client = [0x11u8; RANDOM_LEN];
        let server = [0x22u8; RANDOM_LEN];
        let a = derive_session_keys(&client, &server, ENCRYPTION_METHOD_128BIT);
        let b = derive_session_keys(&client, &server, ENCRYPTION_METHOD_128BIT);
        assert_eq!(a, b);
        assert_eq!(a.mac_key.len(), 16);
        assert_eq!(a.encrypt_key.len(), 16);
        assert_eq!(a.decrypt_key.len(), 16);
        // Encrypt and decrypt keys differ (they hash different key blocks).
        assert_ne!(a.encrypt_key, a.decrypt_key);
    }

    #[test]
    fn weak_keys_have_fixed_prefix() {
        let client = [0x11u8; RANDOM_LEN];
        let server = [0x22u8; RANDOM_LEN];
        let k40 = derive_session_keys(&client, &server, ENCRYPTION_METHOD_40BIT);
        assert_eq!(&k40.encrypt_key[..3], &[0xD1, 0x26, 0x9E]);
        assert_eq!(k40.mac_key.len(), 8);
        let k56 = derive_session_keys(&client, &server, ENCRYPTION_METHOD_56BIT);
        assert_eq!(k56.encrypt_key[0], 0xD1);
    }

    #[test]
    fn session_encrypt_decrypt_roundtrip() {
        // Two peers derive mirror-image keys: the client's encrypt key is the
        // server's decrypt key and vice versa.
        let client = [0x33u8; RANDOM_LEN];
        let server = [0x44u8; RANDOM_LEN];
        let client_keys = derive_session_keys(&client, &server, ENCRYPTION_METHOD_128BIT);
        let server_keys = SessionKeys {
            mac_key: client_keys.mac_key.clone(),
            encrypt_key: client_keys.decrypt_key.clone(),
            decrypt_key: client_keys.encrypt_key.clone(),
        };

        let mut client_session = Rc4Session::new(&client_keys);
        let mut server_session = Rc4Session::new(&server_keys);

        let message = b"TS_INFO_PACKET payload";
        let (sig, ciphertext) = client_session.encrypt(message);
        assert_ne!(&ciphertext[..], &message[..]);
        let recovered = server_session.decrypt(&sig, &ciphertext).unwrap();
        assert_eq!(recovered, message);
    }

    #[test]
    fn tampered_mac_is_rejected() {
        let keys = derive_session_keys(
            &[1u8; RANDOM_LEN],
            &[2u8; RANDOM_LEN],
            ENCRYPTION_METHOD_128BIT,
        );
        let mut enc = Rc4Session::new(&keys);
        let (mut sig, ciphertext) = enc.encrypt(b"hello");
        sig[0] ^= 0xFF;
        // A fresh decrypt session with the same keys must reject the bad MAC.
        let dec_keys = SessionKeys {
            mac_key: keys.mac_key.clone(),
            encrypt_key: keys.decrypt_key.clone(),
            decrypt_key: keys.encrypt_key.clone(),
        };
        let mut dec = Rc4Session::new(&dec_keys);
        assert!(dec.decrypt(&sig, &ciphertext).is_err());
    }

    #[test]
    fn security_exchange_roundtrip() {
        let encrypted = vec![0xAB; 64];
        let pdu = encode_security_exchange(&encrypted);
        // flags = SEC_EXCHANGE_PKT, then flagsHi, then length = 64 + 8.
        assert_eq!(&pdu[..4], &[0x01, 0x00, 0x00, 0x00]);
        assert_eq!(&pdu[4..8], &72u32.to_le_bytes());
        let payload = decode_security_exchange(&pdu).unwrap();
        assert_eq!(payload.len(), 72); // 64 + 8 padding
        assert_eq!(&payload[..64], &encrypted[..]);
    }

    #[test]
    fn basic_security_header_roundtrip() {
        let h = BasicSecurityHeader::new(SEC_ENCRYPT | SEC_INFO_PKT);
        let mut w = Writer::new();
        h.encode(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(BasicSecurityHeader::decode(&mut r).unwrap(), h);
    }
}
