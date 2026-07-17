//! Kerberos ASN.1 DER building blocks (RFC 4120), std-only.
//!
//! Kerberos encodes its PDUs in DER with heavy use of `[APPLICATION n]` and
//! context `[n]` tags, signed `Int32`s, `GeneralString`, and `KerberosTime`
//! (a `GeneralizedTime`). This module provides those primitives plus the
//! small reusable structures — [`PrincipalName`], [`EncryptedData`],
//! [`EncryptionKey`], [`Checksum`] — that the message-level PDUs build on. It
//! layers on the definite-length TLV helpers in [`crate::ber`].

use crate::ber::{expect_tag, write_tlv, TAG_INTEGER, TAG_SEQUENCE};
use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// `GeneralString` universal tag.
pub const TAG_GENERAL_STRING: &[u8] = &[0x1B];
/// `GeneralizedTime` universal tag (used by `KerberosTime`).
pub const TAG_GENERALIZED_TIME: &[u8] = &[0x18];
/// `OCTET STRING` universal tag.
pub const TAG_OCTET_STRING: &[u8] = &[0x04];

/// The `[APPLICATION n]` (constructed) tag byte for `n < 31`.
pub fn application_tag(n: u8) -> [u8; 1] {
    [0x60 | n]
}

/// The context `[n]` (constructed) tag byte for `n < 31`.
pub fn context_tag(n: u8) -> [u8; 1] {
    [0xA0 | n]
}

/// Write a context-tagged `[n]` wrapper around `inner` (an already-encoded
/// TLV).
pub fn write_context(w: &mut Writer, n: u8, inner: &[u8]) {
    write_tlv(w, &context_tag(n), inner);
}

/// Expect a context `[n]` tag and return its content length.
pub fn expect_context(r: &mut Reader<'_>, n: u8) -> Result<usize> {
    expect_tag(r, &context_tag(n))
}

// ---------------------------------------------------------------------------
// Signed / unsigned integers
// ---------------------------------------------------------------------------

/// Minimal two's-complement content bytes for a signed value.
fn int_contents(value: i64) -> Vec<u8> {
    let mut v = value;
    let mut bytes = Vec::new();
    loop {
        let byte = (v & 0xFF) as u8;
        bytes.push(byte);
        v >>= 8;
        let done = (v == 0 && byte & 0x80 == 0) || (v == -1 && byte & 0x80 != 0);
        if done {
            break;
        }
    }
    bytes.reverse();
    bytes
}

/// Write a signed `Int32` as a DER INTEGER.
pub fn write_int32(w: &mut Writer, value: i32) {
    write_tlv(w, TAG_INTEGER, &int_contents(value as i64));
}

/// Read a signed `Int32` DER INTEGER.
pub fn read_int32(r: &mut Reader<'_>) -> Result<i32> {
    let len = expect_tag(r, TAG_INTEGER)?;
    if len == 0 || len > 5 {
        return Err(Error::InvalidLength {
            field: "Kerberos Int32",
            length: len,
        });
    }
    let bytes = r.read_bytes(len)?;
    // Sign-extend from the top bit of the first byte.
    let mut value: i64 = if bytes[0] & 0x80 != 0 { -1 } else { 0 };
    for &b in bytes {
        value = (value << 8) | b as i64;
    }
    Ok(value as i32)
}

/// Write a context-tagged `[n]` signed integer.
pub fn write_context_int32(w: &mut Writer, n: u8, value: i32) {
    let mut inner = Writer::new();
    write_int32(&mut inner, value);
    write_context(w, n, inner.as_slice());
}

/// Read a context-tagged `[n]` signed integer.
pub fn read_context_int32(r: &mut Reader<'_>, n: u8) -> Result<i32> {
    expect_context(r, n)?;
    read_int32(r)
}

// ---------------------------------------------------------------------------
// GeneralString / KerberosTime / OCTET STRING
// ---------------------------------------------------------------------------

/// Write a `GeneralString`.
pub fn write_general_string(w: &mut Writer, s: &str) {
    write_tlv(w, TAG_GENERAL_STRING, s.as_bytes());
}

/// Read a `GeneralString` as owned UTF-8 (Kerberos strings are ASCII in
/// practice).
pub fn read_general_string(r: &mut Reader<'_>) -> Result<String> {
    let len = expect_tag(r, TAG_GENERAL_STRING)?;
    let bytes = r.read_bytes(len)?;
    String::from_utf8(bytes.to_vec()).map_err(|_| Error::InvalidValue {
        field: "Kerberos GeneralString",
        value: "not UTF-8".to_string(),
    })
}

/// Write a context-tagged `[n]` `GeneralString`.
pub fn write_context_general_string(w: &mut Writer, n: u8, s: &str) {
    let mut inner = Writer::new();
    write_general_string(&mut inner, s);
    write_context(w, n, inner.as_slice());
}

/// Write an `OCTET STRING`.
pub fn write_octet_string(w: &mut Writer, bytes: &[u8]) {
    write_tlv(w, TAG_OCTET_STRING, bytes);
}

/// Read an `OCTET STRING`.
pub fn read_octet_string<'a>(r: &mut Reader<'a>) -> Result<&'a [u8]> {
    let len = expect_tag(r, TAG_OCTET_STRING)?;
    r.read_bytes(len)
}

/// Write a context-tagged `[n]` `OCTET STRING`.
pub fn write_context_octet_string(w: &mut Writer, n: u8, bytes: &[u8]) {
    let mut inner = Writer::new();
    write_octet_string(&mut inner, bytes);
    write_context(w, n, inner.as_slice());
}

/// Read a context-tagged `[n]` `OCTET STRING` as owned bytes.
pub fn read_context_octet_string(r: &mut Reader<'_>, n: u8) -> Result<Vec<u8>> {
    expect_context(r, n)?;
    Ok(read_octet_string(r)?.to_vec())
}

/// A `KerberosTime`, stored as its 15-character `YYYYMMDDHHMMSSZ` form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KerberosTime(pub String);

impl KerberosTime {
    /// Build from broken-down UTC components.
    pub fn from_utc(year: u32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> Self {
        KerberosTime(format!(
            "{year:04}{month:02}{day:02}{hour:02}{min:02}{sec:02}Z"
        ))
    }

    /// Write as a `GeneralizedTime`.
    pub fn write(&self, w: &mut Writer) {
        write_tlv(w, TAG_GENERALIZED_TIME, self.0.as_bytes());
    }

    /// Read a `GeneralizedTime`.
    pub fn read(r: &mut Reader<'_>) -> Result<KerberosTime> {
        let len = expect_tag(r, TAG_GENERALIZED_TIME)?;
        let bytes = r.read_bytes(len)?;
        let s = String::from_utf8(bytes.to_vec()).map_err(|_| Error::InvalidValue {
            field: "KerberosTime",
            value: "not UTF-8".to_string(),
        })?;
        Ok(KerberosTime(s))
    }
}

// ---------------------------------------------------------------------------
// PrincipalName
// ---------------------------------------------------------------------------

/// `PrincipalName ::= SEQUENCE { name-type [0] Int32, name-string [1] SEQUENCE OF GeneralString }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalName {
    /// The `NT_*` name type.
    pub name_type: i32,
    /// The name components.
    pub name_string: Vec<String>,
}

/// `NT-PRINCIPAL` name type.
pub const NT_PRINCIPAL: i32 = 1;
/// `NT-SRV-INST` name type (used for `krbtgt` and service principals).
pub const NT_SRV_INST: i32 = 2;

impl PrincipalName {
    /// Encode to DER.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        write_context_int32(&mut body, 0, self.name_type);

        let mut seq_of = Writer::new();
        for part in &self.name_string {
            write_general_string(&mut seq_of, part);
        }
        let mut seq = Writer::new();
        write_tlv(&mut seq, TAG_SEQUENCE, seq_of.as_slice());
        write_context(&mut body, 1, seq.as_slice());

        let mut out = Writer::new();
        write_tlv(&mut out, TAG_SEQUENCE, body.as_slice());
        out.into_vec()
    }

    /// Decode from DER.
    pub fn decode(buf: &[u8]) -> Result<PrincipalName> {
        let mut r = Reader::new(buf);
        let len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(len)?;
        let mut r = Reader::new(body);

        let name_type = read_context_int32(&mut r, 0)?;

        let seq_len = expect_context(&mut r, 1)?;
        let seq = r.read_bytes(seq_len)?;
        let mut sr = Reader::new(seq);
        let inner_len = expect_tag(&mut sr, TAG_SEQUENCE)?;
        let inner = sr.read_bytes(inner_len)?;
        let mut ir = Reader::new(inner);
        let mut name_string = Vec::new();
        while !ir.is_empty() {
            name_string.push(read_general_string(&mut ir)?);
        }
        Ok(PrincipalName {
            name_type,
            name_string,
        })
    }
}

// ---------------------------------------------------------------------------
// EncryptedData / EncryptionKey / Checksum
// ---------------------------------------------------------------------------

/// `EncryptedData ::= SEQUENCE { etype [0] Int32, kvno [1] UInt32 OPTIONAL, cipher [2] OCTET STRING }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedData {
    /// Encryption type.
    pub etype: i32,
    /// Key version number, if present.
    pub kvno: Option<i32>,
    /// The ciphertext.
    pub cipher: Vec<u8>,
}

impl EncryptedData {
    /// Encode to DER.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        write_context_int32(&mut body, 0, self.etype);
        if let Some(kvno) = self.kvno {
            write_context_int32(&mut body, 1, kvno);
        }
        write_context_octet_string(&mut body, 2, &self.cipher);
        let mut out = Writer::new();
        write_tlv(&mut out, TAG_SEQUENCE, body.as_slice());
        out.into_vec()
    }

    /// Decode from DER.
    pub fn decode(buf: &[u8]) -> Result<EncryptedData> {
        let mut r = Reader::new(buf);
        let len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(len)?;
        let mut r = Reader::new(body);

        let etype = read_context_int32(&mut r, 0)?;
        let mut kvno = None;
        if !r.is_empty() && r.peek_remaining()[0] == context_tag(1)[0] {
            kvno = Some(read_context_int32(&mut r, 1)?);
        }
        let cipher = read_context_octet_string(&mut r, 2)?;
        Ok(EncryptedData {
            etype,
            kvno,
            cipher,
        })
    }
}

/// `EncryptionKey ::= SEQUENCE { keytype [0] Int32, keyvalue [1] OCTET STRING }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionKey {
    /// Key type (an encryption-type number).
    pub keytype: i32,
    /// Raw key bytes.
    pub keyvalue: Vec<u8>,
}

impl EncryptionKey {
    /// Encode to DER.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        write_context_int32(&mut body, 0, self.keytype);
        write_context_octet_string(&mut body, 1, &self.keyvalue);
        let mut out = Writer::new();
        write_tlv(&mut out, TAG_SEQUENCE, body.as_slice());
        out.into_vec()
    }

    /// Decode from DER.
    pub fn decode(buf: &[u8]) -> Result<EncryptionKey> {
        let mut r = Reader::new(buf);
        let len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(len)?;
        let mut r = Reader::new(body);
        let keytype = read_context_int32(&mut r, 0)?;
        let keyvalue = read_context_octet_string(&mut r, 1)?;
        Ok(EncryptionKey { keytype, keyvalue })
    }
}

/// `Checksum ::= SEQUENCE { cksumtype [0] Int32, checksum [1] OCTET STRING }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checksum {
    /// Checksum type.
    pub cksumtype: i32,
    /// Checksum bytes.
    pub checksum: Vec<u8>,
}

impl Checksum {
    /// Encode to DER.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        write_context_int32(&mut body, 0, self.cksumtype);
        write_context_octet_string(&mut body, 1, &self.checksum);
        let mut out = Writer::new();
        write_tlv(&mut out, TAG_SEQUENCE, body.as_slice());
        out.into_vec()
    }

    /// Decode from DER.
    pub fn decode(buf: &[u8]) -> Result<Checksum> {
        let mut r = Reader::new(buf);
        let len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(len)?;
        let mut r = Reader::new(body);
        let cksumtype = read_context_int32(&mut r, 0)?;
        let checksum = read_context_octet_string(&mut r, 1)?;
        Ok(Checksum {
            cksumtype,
            checksum,
        })
    }
}

/// Write an `[APPLICATION n]`-tagged wrapper around `inner`.
pub fn write_application(w: &mut Writer, n: u8, inner: &[u8]) {
    write_tlv(w, &application_tag(n), inner);
}

/// Expect an `[APPLICATION n]` tag, returning the content length.
pub fn expect_application(r: &mut Reader<'_>, n: u8) -> Result<usize> {
    expect_tag(r, &application_tag(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int32_signed_roundtrip() {
        for value in [
            0i32,
            1,
            23,
            127,
            128,
            255,
            256,
            -1,
            -138,
            -256,
            i32::MAX,
            i32::MIN,
        ] {
            let mut w = Writer::new();
            write_int32(&mut w, value);
            let mut r = Reader::new(w.as_slice());
            assert_eq!(read_int32(&mut r).unwrap(), value, "value {value}");
        }
    }

    #[test]
    fn negative_int_minimal_encoding() {
        // -138 encodes as two content bytes FF 76.
        let mut w = Writer::new();
        write_int32(&mut w, -138);
        assert_eq!(w.as_slice(), &[0x02, 0x02, 0xFF, 0x76]);
    }

    #[test]
    fn principal_name_roundtrip() {
        let name = PrincipalName {
            name_type: NT_SRV_INST,
            name_string: vec!["krbtgt".to_string(), "EXAMPLE.COM".to_string()],
        };
        let encoded = name.encode();
        assert_eq!(PrincipalName::decode(&encoded).unwrap(), name);
    }

    #[test]
    fn kerberos_time_roundtrip() {
        let t = KerberosTime::from_utc(2026, 7, 17, 12, 30, 5);
        assert_eq!(t.0, "20260717123005Z");
        let mut w = Writer::new();
        t.write(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(KerberosTime::read(&mut r).unwrap(), t);
    }

    #[test]
    fn encrypted_data_roundtrip_with_and_without_kvno() {
        let with = EncryptedData {
            etype: 23,
            kvno: Some(2),
            cipher: vec![0xDE, 0xAD, 0xBE, 0xEF],
        };
        assert_eq!(EncryptedData::decode(&with.encode()).unwrap(), with);

        let without = EncryptedData {
            etype: 18,
            kvno: None,
            cipher: vec![0x01; 32],
        };
        assert_eq!(EncryptedData::decode(&without.encode()).unwrap(), without);
    }

    #[test]
    fn encryption_key_and_checksum_roundtrip() {
        let key = EncryptionKey {
            keytype: 23,
            keyvalue: vec![0xAB; 16],
        };
        assert_eq!(EncryptionKey::decode(&key.encode()).unwrap(), key);

        let cksum = Checksum {
            cksumtype: -138,
            checksum: vec![0xCD; 16],
        };
        assert_eq!(Checksum::decode(&cksum.encode()).unwrap(), cksum);
    }
}
