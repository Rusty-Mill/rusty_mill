//! Kerberos v5 message PDUs (RFC 4120), std-only.
//!
//! The application-tagged messages of the Kerberos exchange, built on the DER
//! primitives in [`crate::krb5::asn1`]:
//!
//! * [`Authenticator`] and [`Ticket`], carried inside [`ApReq`] — the token
//!   RDP's NLA sends to the server.
//! * [`KdcReqBody`] and [`KdcReq`] (AS-REQ / TGS-REQ) and [`KdcRep`]
//!   (AS-REP / TGS-REP) with [`PaData`] — the KDC exchange.
//! * [`EncKdcRepPart`] — the encrypted reply part the client decrypts to get
//!   the session key.
//! * [`KrbError`] — the KDC / server error reply.
//!
//! Encrypted parts (`enc-part`, the authenticator inside AP-REQ) are carried
//! as [`asn1::EncryptedData`]; sealing them is the crypto profile's job
//! ([`crate::krb5::crypto`]).

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

use super::asn1::{
    self, expect_application, expect_context, read_context_flags, read_context_int32,
    read_context_octet_string, read_context_time, read_context_uint32, read_general_string,
    write_application, write_context, write_context_flags, write_context_general_string,
    write_context_int32, write_context_octet_string, write_context_time, write_context_uint32,
    Checksum, EncryptedData, EncryptionKey, KerberosTime, PrincipalName,
};
use crate::ber::{expect_tag, write_tlv, TAG_SEQUENCE};

/// Kerberos protocol version number.
pub const PVNO: i32 = 5;

// Application tag numbers (RFC 4120 5.10).
const APP_TICKET: u8 = 1;
const APP_AUTHENTICATOR: u8 = 2;
const APP_AS_REQ: u8 = 10;
const APP_AS_REP: u8 = 11;
const APP_TGS_REQ: u8 = 12;
const APP_TGS_REP: u8 = 13;
const APP_AP_REQ: u8 = 14;
const APP_ENC_AS_REP_PART: u8 = 25;
const APP_ENC_TGS_REP_PART: u8 = 26;
const APP_KRB_ERROR: u8 = 30;

// Message types.
/// `AS-REQ` message type.
pub const KRB_AS_REQ: i32 = 10;
/// `AS-REP` message type.
pub const KRB_AS_REP: i32 = 11;
/// `TGS-REQ` message type.
pub const KRB_TGS_REQ: i32 = 12;
/// `TGS-REP` message type.
pub const KRB_TGS_REP: i32 = 13;
/// `AP-REQ` message type.
pub const KRB_AP_REQ: i32 = 14;
/// `KRB-ERROR` message type.
pub const KRB_ERROR: i32 = 30;

// PA-DATA types.
/// `PA-TGS-REQ` (an AP-REQ used as pre-authentication to the TGS).
pub const PA_TGS_REQ: i32 = 1;
/// `PA-ENC-TIMESTAMP` pre-authentication.
pub const PA_ENC_TIMESTAMP: i32 = 2;
/// `PA-PAC-REQUEST`.
pub const PA_PAC_REQUEST: i32 = 128;

// A few common KDCOptions / APOptions bits (MSB = bit 0).
/// `forwardable` KDC option.
pub const KDC_OPT_FORWARDABLE: u32 = 0x4000_0000;
/// `renewable` KDC option.
pub const KDC_OPT_RENEWABLE: u32 = 0x0080_0000;
/// `canonicalize` KDC option.
pub const KDC_OPT_CANONICALIZE: u32 = 0x0001_0000;
/// `mutual-required` AP option.
pub const AP_OPT_MUTUAL_REQUIRED: u32 = 0x2000_0000;
/// `use-session-key` AP option.
pub const AP_OPT_USE_SESSION_KEY: u32 = 0x4000_0000;

/// Encode a `SEQUENCE` wrapping `body`.
fn sequence(body: &[u8]) -> Vec<u8> {
    let mut out = Writer::new();
    write_tlv(&mut out, TAG_SEQUENCE, body);
    out.into_vec()
}

/// Wrap `inner` (a complete SEQUENCE) in an `[APPLICATION n]` tag.
fn application(n: u8, inner: &[u8]) -> Vec<u8> {
    let mut out = Writer::new();
    write_application(&mut out, n, inner);
    out.into_vec()
}

// ---------------------------------------------------------------------------
// PA-DATA
// ---------------------------------------------------------------------------

/// `PA-DATA ::= SEQUENCE { padata-type [1] Int32, padata-value [2] OCTET STRING }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaData {
    /// The `PA_*` type.
    pub padata_type: i32,
    /// The raw pre-authentication value.
    pub padata_value: Vec<u8>,
}

impl PaData {
    fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        write_context_int32(&mut body, 1, self.padata_type);
        write_context_octet_string(&mut body, 2, &self.padata_value);
        sequence(body.as_slice())
    }

    fn decode(buf: &[u8]) -> Result<PaData> {
        let mut r = Reader::new(buf);
        let len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(len)?;
        let mut r = Reader::new(body);
        let padata_type = read_context_int32(&mut r, 1)?;
        let padata_value = read_context_octet_string(&mut r, 2)?;
        Ok(PaData {
            padata_type,
            padata_value,
        })
    }
}

/// Encode a `SEQUENCE OF PA-DATA`.
fn encode_padata_seq(items: &[PaData]) -> Vec<u8> {
    let mut inner = Writer::new();
    for pa in items {
        inner.write_bytes(&pa.encode());
    }
    sequence(inner.as_slice())
}

/// Decode a `SEQUENCE OF PA-DATA`.
fn decode_padata_seq(buf: &[u8]) -> Result<Vec<PaData>> {
    let mut r = Reader::new(buf);
    let len = expect_tag(&mut r, TAG_SEQUENCE)?;
    let body = r.read_bytes(len)?;
    let mut r = Reader::new(body);
    let mut items = Vec::new();
    while !r.is_empty() {
        let start = r.position();
        let inner_len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let inner = r.read_bytes(inner_len)?;
        let _ = start;
        // Re-encode a standalone SEQUENCE for PaData::decode.
        items.push(PaData::decode(&sequence(inner))?);
    }
    Ok(items)
}

// ---------------------------------------------------------------------------
// Ticket
// ---------------------------------------------------------------------------

/// `Ticket ::= [APPLICATION 1] SEQUENCE { tkt-vno [0] INTEGER(5), realm [1]
/// Realm, sname [2] PrincipalName, enc-part [3] EncryptedData }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    /// Ticket realm.
    pub realm: String,
    /// Service principal name.
    pub sname: PrincipalName,
    /// The encrypted ticket part (opaque to the client).
    pub enc_part: EncryptedData,
}

impl Ticket {
    /// Encode to DER.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        write_context_int32(&mut body, 0, PVNO);
        write_context_general_string(&mut body, 1, &self.realm);
        write_context(&mut body, 2, &self.sname.encode());
        write_context(&mut body, 3, &self.enc_part.encode());
        application(APP_TICKET, &sequence(body.as_slice()))
    }

    /// Decode from DER.
    pub fn decode(buf: &[u8]) -> Result<Ticket> {
        let mut r = Reader::new(buf);
        let app_len = expect_application(&mut r, APP_TICKET)?;
        let app = r.read_bytes(app_len)?;
        let mut r = Reader::new(app);
        let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(seq_len)?;
        let mut r = Reader::new(body);

        let _vno = read_context_int32(&mut r, 0)?;
        expect_context(&mut r, 1)?;
        let realm = read_general_string(&mut r)?;
        let sname = read_context_struct(&mut r, 2, PrincipalName::decode)?;
        let enc_part = read_context_struct(&mut r, 3, EncryptedData::decode)?;
        Ok(Ticket {
            realm,
            sname,
            enc_part,
        })
    }
}

/// Read a context `[n]` wrapper and decode its inner structure with `f`.
fn read_context_struct<T>(r: &mut Reader<'_>, n: u8, f: impl Fn(&[u8]) -> Result<T>) -> Result<T> {
    let len = expect_context(r, n)?;
    let inner = r.read_bytes(len)?;
    f(inner)
}

// ---------------------------------------------------------------------------
// Authenticator
// ---------------------------------------------------------------------------

/// `Authenticator ::= [APPLICATION 2] SEQUENCE { ... }` (RFC 4120 5.5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authenticator {
    /// Client realm.
    pub crealm: String,
    /// Client principal name.
    pub cname: PrincipalName,
    /// Optional checksum (the GSS channel-binding checksum for NLA).
    pub cksum: Option<Checksum>,
    /// Microsecond part of the client time.
    pub cusec: i32,
    /// Client time.
    pub ctime: KerberosTime,
    /// Optional subsession key.
    pub subkey: Option<EncryptionKey>,
    /// Optional initial sequence number.
    pub seq_number: Option<u32>,
}

impl Authenticator {
    /// Encode to DER.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        write_context_int32(&mut body, 0, PVNO);
        write_context_general_string(&mut body, 1, &self.crealm);
        write_context(&mut body, 2, &self.cname.encode());
        if let Some(cksum) = &self.cksum {
            write_context(&mut body, 3, &cksum.encode());
        }
        write_context_int32(&mut body, 4, self.cusec);
        write_context_time(&mut body, 5, &self.ctime);
        if let Some(subkey) = &self.subkey {
            write_context(&mut body, 6, &subkey.encode());
        }
        if let Some(seq) = self.seq_number {
            write_context_uint32(&mut body, 7, seq);
        }
        application(APP_AUTHENTICATOR, &sequence(body.as_slice()))
    }

    /// Decode from DER.
    pub fn decode(buf: &[u8]) -> Result<Authenticator> {
        let mut r = Reader::new(buf);
        let app_len = expect_application(&mut r, APP_AUTHENTICATOR)?;
        let app = r.read_bytes(app_len)?;
        let mut r = Reader::new(app);
        let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(seq_len)?;
        let mut r = Reader::new(body);

        let _vno = read_context_int32(&mut r, 0)?;
        expect_context(&mut r, 1)?;
        let crealm = read_general_string(&mut r)?;
        let cname = read_context_struct(&mut r, 2, PrincipalName::decode)?;
        let cksum = if peek_tag(&r) == Some(asn1::context_tag(3)[0]) {
            Some(read_context_struct(&mut r, 3, Checksum::decode)?)
        } else {
            None
        };
        let cusec = read_context_int32(&mut r, 4)?;
        let ctime = read_context_time(&mut r, 5)?;
        let subkey = if peek_tag(&r) == Some(asn1::context_tag(6)[0]) {
            Some(read_context_struct(&mut r, 6, EncryptionKey::decode)?)
        } else {
            None
        };
        let seq_number = if peek_tag(&r) == Some(asn1::context_tag(7)[0]) {
            Some(read_context_uint32(&mut r, 7)?)
        } else {
            None
        };
        Ok(Authenticator {
            crealm,
            cname,
            cksum,
            cusec,
            ctime,
            subkey,
            seq_number,
        })
    }
}

/// Peek the next tag byte without consuming, if any.
fn peek_tag(r: &Reader<'_>) -> Option<u8> {
    r.peek_remaining().first().copied()
}

// ---------------------------------------------------------------------------
// AP-REQ
// ---------------------------------------------------------------------------

/// `AP-REQ ::= [APPLICATION 14] SEQUENCE { pvno [0] INTEGER(5), msg-type [1]
/// INTEGER(14), ap-options [2] APOptions, ticket [3] Ticket, authenticator [4]
/// EncryptedData }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApReq {
    /// The `AP_OPT_*` option bits.
    pub ap_options: u32,
    /// The service ticket.
    pub ticket: Ticket,
    /// The encrypted authenticator.
    pub authenticator: EncryptedData,
}

impl ApReq {
    /// Encode to DER.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        write_context_int32(&mut body, 0, PVNO);
        write_context_int32(&mut body, 1, KRB_AP_REQ);
        write_context_flags(&mut body, 2, self.ap_options);
        write_context(&mut body, 3, &self.ticket.encode());
        write_context(&mut body, 4, &self.authenticator.encode());
        application(APP_AP_REQ, &sequence(body.as_slice()))
    }

    /// Decode from DER.
    pub fn decode(buf: &[u8]) -> Result<ApReq> {
        let mut r = Reader::new(buf);
        let app_len = expect_application(&mut r, APP_AP_REQ)?;
        let app = r.read_bytes(app_len)?;
        let mut r = Reader::new(app);
        let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(seq_len)?;
        let mut r = Reader::new(body);

        let _vno = read_context_int32(&mut r, 0)?;
        let msg_type = read_context_int32(&mut r, 1)?;
        if msg_type != KRB_AP_REQ {
            return Err(Error::InvalidValue {
                field: "AP-REQ msg-type",
                value: msg_type.to_string(),
            });
        }
        let ap_options = read_context_flags(&mut r, 2)?;
        let ticket = read_context_struct(&mut r, 3, Ticket::decode)?;
        let authenticator = read_context_struct(&mut r, 4, EncryptedData::decode)?;
        Ok(ApReq {
            ap_options,
            ticket,
            authenticator,
        })
    }
}

// ---------------------------------------------------------------------------
// KDC-REQ (AS-REQ / TGS-REQ)
// ---------------------------------------------------------------------------

/// `KDC-REQ-BODY` (RFC 4120 5.4.1), with the rarely-needed optional fields
/// (addresses, enc-authorization-data, additional-tickets) omitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KdcReqBody {
    /// The `KDC_OPT_*` option bits.
    pub kdc_options: u32,
    /// Optional client principal name.
    pub cname: Option<PrincipalName>,
    /// Request realm.
    pub realm: String,
    /// Optional service principal name.
    pub sname: Option<PrincipalName>,
    /// Optional desired start time.
    pub from: Option<KerberosTime>,
    /// Requested expiry time.
    pub till: KerberosTime,
    /// Optional requested renew-till time.
    pub rtime: Option<KerberosTime>,
    /// Anti-replay nonce.
    pub nonce: u32,
    /// Requested encryption types, in preference order.
    pub etypes: Vec<i32>,
}

impl KdcReqBody {
    fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        write_context_flags(&mut body, 0, self.kdc_options);
        if let Some(cname) = &self.cname {
            write_context(&mut body, 1, &cname.encode());
        }
        write_context_general_string(&mut body, 2, &self.realm);
        if let Some(sname) = &self.sname {
            write_context(&mut body, 3, &sname.encode());
        }
        if let Some(from) = &self.from {
            write_context_time(&mut body, 4, from);
        }
        write_context_time(&mut body, 5, &self.till);
        if let Some(rtime) = &self.rtime {
            write_context_time(&mut body, 6, rtime);
        }
        write_context_uint32(&mut body, 7, self.nonce);

        let mut etype_seq = Writer::new();
        for etype in &self.etypes {
            asn1::write_int32(&mut etype_seq, *etype);
        }
        let mut etype_wrapped = Writer::new();
        write_tlv(&mut etype_wrapped, TAG_SEQUENCE, etype_seq.as_slice());
        write_context(&mut body, 8, etype_wrapped.as_slice());

        sequence(body.as_slice())
    }

    fn decode(buf: &[u8]) -> Result<KdcReqBody> {
        let mut r = Reader::new(buf);
        let len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(len)?;
        let mut r = Reader::new(body);

        let kdc_options = read_context_flags(&mut r, 0)?;
        let cname = if peek_tag(&r) == Some(asn1::context_tag(1)[0]) {
            Some(read_context_struct(&mut r, 1, PrincipalName::decode)?)
        } else {
            None
        };
        expect_context(&mut r, 2)?;
        let realm = read_general_string(&mut r)?;
        let sname = if peek_tag(&r) == Some(asn1::context_tag(3)[0]) {
            Some(read_context_struct(&mut r, 3, PrincipalName::decode)?)
        } else {
            None
        };
        let from = if peek_tag(&r) == Some(asn1::context_tag(4)[0]) {
            Some(read_context_time(&mut r, 4)?)
        } else {
            None
        };
        let till = read_context_time(&mut r, 5)?;
        let rtime = if peek_tag(&r) == Some(asn1::context_tag(6)[0]) {
            Some(read_context_time(&mut r, 6)?)
        } else {
            None
        };
        let nonce = read_context_uint32(&mut r, 7)?;

        let etypes = read_context_struct(&mut r, 8, |inner| {
            let mut r = Reader::new(inner);
            let len = expect_tag(&mut r, TAG_SEQUENCE)?;
            let seq = r.read_bytes(len)?;
            let mut r = Reader::new(seq);
            let mut etypes = Vec::new();
            while !r.is_empty() {
                etypes.push(asn1::read_int32(&mut r)?);
            }
            Ok(etypes)
        })?;

        Ok(KdcReqBody {
            kdc_options,
            cname,
            realm,
            sname,
            from,
            till,
            rtime,
            nonce,
            etypes,
        })
    }
}

/// `KDC-REQ` (AS-REQ / TGS-REQ) — the application tag depends on `msg_type`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KdcReq {
    /// `KRB_AS_REQ` or `KRB_TGS_REQ`.
    pub msg_type: i32,
    /// Pre-authentication data.
    pub padata: Vec<PaData>,
    /// The request body.
    pub req_body: KdcReqBody,
}

impl KdcReq {
    fn app_tag(&self) -> u8 {
        if self.msg_type == KRB_TGS_REQ {
            APP_TGS_REQ
        } else {
            APP_AS_REQ
        }
    }

    /// Encode to DER.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        write_context_int32(&mut body, 1, PVNO);
        write_context_int32(&mut body, 2, self.msg_type);
        if !self.padata.is_empty() {
            write_context(&mut body, 3, &encode_padata_seq(&self.padata));
        }
        write_context(&mut body, 4, &self.req_body.encode());
        application(self.app_tag(), &sequence(body.as_slice()))
    }

    /// Decode from DER (either AS-REQ or TGS-REQ).
    pub fn decode(buf: &[u8]) -> Result<KdcReq> {
        let tag = buf.first().copied().unwrap_or(0);
        let app = if tag == asn1::application_tag(APP_TGS_REQ)[0] {
            APP_TGS_REQ
        } else {
            APP_AS_REQ
        };
        let mut r = Reader::new(buf);
        let app_len = expect_application(&mut r, app)?;
        let inner = r.read_bytes(app_len)?;
        let mut r = Reader::new(inner);
        let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(seq_len)?;
        let mut r = Reader::new(body);

        let _vno = read_context_int32(&mut r, 1)?;
        let msg_type = read_context_int32(&mut r, 2)?;
        let padata = if peek_tag(&r) == Some(asn1::context_tag(3)[0]) {
            read_context_struct(&mut r, 3, decode_padata_seq)?
        } else {
            Vec::new()
        };
        let req_body = read_context_struct(&mut r, 4, KdcReqBody::decode)?;
        Ok(KdcReq {
            msg_type,
            padata,
            req_body,
        })
    }
}

// ---------------------------------------------------------------------------
// KDC-REP (AS-REP / TGS-REP)
// ---------------------------------------------------------------------------

/// `KDC-REP` (AS-REP / TGS-REP).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KdcRep {
    /// `KRB_AS_REP` or `KRB_TGS_REP`.
    pub msg_type: i32,
    /// Pre-authentication data.
    pub padata: Vec<PaData>,
    /// Client realm.
    pub crealm: String,
    /// Client principal name.
    pub cname: PrincipalName,
    /// The issued ticket.
    pub ticket: Ticket,
    /// The encrypted reply part (decrypt with the client key to get the
    /// session key).
    pub enc_part: EncryptedData,
}

impl KdcRep {
    fn app_tag(&self) -> u8 {
        if self.msg_type == KRB_TGS_REP {
            APP_TGS_REP
        } else {
            APP_AS_REP
        }
    }

    /// Encode to DER.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        write_context_int32(&mut body, 0, PVNO);
        write_context_int32(&mut body, 1, self.msg_type);
        if !self.padata.is_empty() {
            write_context(&mut body, 2, &encode_padata_seq(&self.padata));
        }
        write_context_general_string(&mut body, 3, &self.crealm);
        write_context(&mut body, 4, &self.cname.encode());
        write_context(&mut body, 5, &self.ticket.encode());
        write_context(&mut body, 6, &self.enc_part.encode());
        application(self.app_tag(), &sequence(body.as_slice()))
    }

    /// Decode from DER (either AS-REP or TGS-REP).
    pub fn decode(buf: &[u8]) -> Result<KdcRep> {
        let tag = buf.first().copied().unwrap_or(0);
        let app = if tag == asn1::application_tag(APP_TGS_REP)[0] {
            APP_TGS_REP
        } else {
            APP_AS_REP
        };
        let mut r = Reader::new(buf);
        let app_len = expect_application(&mut r, app)?;
        let inner = r.read_bytes(app_len)?;
        let mut r = Reader::new(inner);
        let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(seq_len)?;
        let mut r = Reader::new(body);

        let _vno = read_context_int32(&mut r, 0)?;
        let msg_type = read_context_int32(&mut r, 1)?;
        let padata = if peek_tag(&r) == Some(asn1::context_tag(2)[0]) {
            read_context_struct(&mut r, 2, decode_padata_seq)?
        } else {
            Vec::new()
        };
        expect_context(&mut r, 3)?;
        let crealm = read_general_string(&mut r)?;
        let cname = read_context_struct(&mut r, 4, PrincipalName::decode)?;
        let ticket = read_context_struct(&mut r, 5, Ticket::decode)?;
        let enc_part = read_context_struct(&mut r, 6, EncryptedData::decode)?;
        Ok(KdcRep {
            msg_type,
            padata,
            crealm,
            cname,
            ticket,
            enc_part,
        })
    }
}

// ---------------------------------------------------------------------------
// EncKDCRepPart
// ---------------------------------------------------------------------------

/// The decrypted `EncKDCRepPart` (RFC 4120 5.4.2), with only the fields a
/// client needs surfaced; the rest are parsed and skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncKdcRepPart {
    /// The session key.
    pub key: EncryptionKey,
    /// The nonce echoed from the request.
    pub nonce: u32,
    /// Ticket flags.
    pub flags: u32,
    /// Ticket expiry.
    pub endtime: KerberosTime,
    /// Server realm.
    pub srealm: String,
    /// Server principal name.
    pub sname: PrincipalName,
}

impl EncKdcRepPart {
    /// Decode from DER (the `[APPLICATION 25]`/`[26]` wrapper is accepted).
    pub fn decode(buf: &[u8]) -> Result<EncKdcRepPart> {
        let tag = buf.first().copied().unwrap_or(0);
        let app = if tag == asn1::application_tag(APP_ENC_TGS_REP_PART)[0] {
            APP_ENC_TGS_REP_PART
        } else {
            APP_ENC_AS_REP_PART
        };
        let mut r = Reader::new(buf);
        let app_len = expect_application(&mut r, app)?;
        let inner = r.read_bytes(app_len)?;
        let mut r = Reader::new(inner);
        let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(seq_len)?;
        let mut r = Reader::new(body);

        // [0] key, [1] last-req, [2] nonce, [3] key-expiration OPTIONAL,
        // [4] flags, [5] authtime, [6] starttime OPTIONAL, [7] endtime,
        // [8] renew-till OPTIONAL, [9] srealm, [10] sname, [11] caddr OPTIONAL.
        let key = read_context_struct(&mut r, 0, EncryptionKey::decode)?;
        skip_context(&mut r, 1)?; // last-req
        let nonce = read_context_uint32(&mut r, 2)?;
        skip_optional(&mut r, 3)?; // key-expiration
        let flags = read_context_flags(&mut r, 4)?;
        skip_context(&mut r, 5)?; // authtime
        skip_optional(&mut r, 6)?; // starttime
        let endtime = read_context_time(&mut r, 7)?;
        skip_optional(&mut r, 8)?; // renew-till
        expect_context(&mut r, 9)?;
        let srealm = read_general_string(&mut r)?;
        let sname = read_context_struct(&mut r, 10, PrincipalName::decode)?;
        Ok(EncKdcRepPart {
            key,
            nonce,
            flags,
            endtime,
            srealm,
            sname,
        })
    }
}

/// Consume a context `[n]` field and discard its content.
fn skip_context(r: &mut Reader<'_>, n: u8) -> Result<()> {
    let len = expect_context(r, n)?;
    r.read_bytes(len)?;
    Ok(())
}

/// Consume a context `[n]` field only if it is present next.
fn skip_optional(r: &mut Reader<'_>, n: u8) -> Result<()> {
    if peek_tag(r) == Some(asn1::context_tag(n)[0]) {
        skip_context(r, n)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// KRB-ERROR
// ---------------------------------------------------------------------------

/// `KRB-ERROR ::= [APPLICATION 30] SEQUENCE { ... }` (RFC 4120 5.9.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrbError {
    /// Server time.
    pub stime: KerberosTime,
    /// Server microseconds.
    pub susec: i32,
    /// The Kerberos error code.
    pub error_code: i32,
    /// Server realm.
    pub realm: String,
    /// Server principal name.
    pub sname: PrincipalName,
    /// Optional human-readable error text.
    pub e_text: Option<String>,
    /// Optional error data.
    pub e_data: Option<Vec<u8>>,
}

impl KrbError {
    /// Encode to DER.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        write_context_int32(&mut body, 0, PVNO);
        write_context_int32(&mut body, 1, KRB_ERROR);
        write_context_time(&mut body, 4, &self.stime);
        write_context_int32(&mut body, 5, self.susec);
        write_context_int32(&mut body, 6, self.error_code);
        write_context_general_string(&mut body, 9, &self.realm);
        write_context(&mut body, 10, &self.sname.encode());
        if let Some(text) = &self.e_text {
            write_context_general_string(&mut body, 11, text);
        }
        if let Some(data) = &self.e_data {
            write_context_octet_string(&mut body, 12, data);
        }
        application(APP_KRB_ERROR, &sequence(body.as_slice()))
    }

    /// Decode from DER.
    pub fn decode(buf: &[u8]) -> Result<KrbError> {
        let mut r = Reader::new(buf);
        let app_len = expect_application(&mut r, APP_KRB_ERROR)?;
        let inner = r.read_bytes(app_len)?;
        let mut r = Reader::new(inner);
        let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(seq_len)?;
        let mut r = Reader::new(body);

        let _vno = read_context_int32(&mut r, 0)?;
        let _msg_type = read_context_int32(&mut r, 1)?;
        skip_optional(&mut r, 2)?; // ctime
        skip_optional(&mut r, 3)?; // cusec
        let stime = read_context_time(&mut r, 4)?;
        let susec = read_context_int32(&mut r, 5)?;
        let error_code = read_context_int32(&mut r, 6)?;
        skip_optional(&mut r, 7)?; // crealm
        skip_optional(&mut r, 8)?; // cname
        expect_context(&mut r, 9)?;
        let realm = read_general_string(&mut r)?;
        let sname = read_context_struct(&mut r, 10, PrincipalName::decode)?;
        let e_text = if peek_tag(&r) == Some(asn1::context_tag(11)[0]) {
            expect_context(&mut r, 11)?;
            Some(read_general_string(&mut r)?)
        } else {
            None
        };
        let e_data = if peek_tag(&r) == Some(asn1::context_tag(12)[0]) {
            Some(read_context_octet_string(&mut r, 12)?)
        } else {
            None
        };
        Ok(KrbError {
            stime,
            susec,
            error_code,
            realm,
            sname,
            e_text,
            e_data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::krb5::asn1::{NT_PRINCIPAL, NT_SRV_INST};

    fn princ(name_type: i32, parts: &[&str]) -> PrincipalName {
        PrincipalName {
            name_type,
            name_string: parts.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn enc_data() -> EncryptedData {
        EncryptedData {
            etype: 23,
            kvno: Some(2),
            cipher: vec![0xDE, 0xAD, 0xBE, 0xEF],
        }
    }

    fn ticket() -> Ticket {
        Ticket {
            realm: "EXAMPLE.COM".to_string(),
            sname: princ(NT_SRV_INST, &["krbtgt", "EXAMPLE.COM"]),
            enc_part: enc_data(),
        }
    }

    #[test]
    fn ticket_roundtrip() {
        let t = ticket();
        assert_eq!(Ticket::decode(&t.encode()).unwrap(), t);
    }

    #[test]
    fn authenticator_roundtrip_full_and_minimal() {
        let full = Authenticator {
            crealm: "EXAMPLE.COM".to_string(),
            cname: princ(NT_PRINCIPAL, &["alice"]),
            cksum: Some(Checksum {
                cksumtype: -138,
                checksum: vec![0x11; 16],
            }),
            cusec: 123456,
            ctime: KerberosTime::from_utc(2026, 7, 17, 12, 0, 0),
            subkey: Some(EncryptionKey {
                keytype: 23,
                keyvalue: vec![0x22; 16],
            }),
            seq_number: Some(0x0102_0304),
        };
        assert_eq!(Authenticator::decode(&full.encode()).unwrap(), full);

        let minimal = Authenticator {
            crealm: "EXAMPLE.COM".to_string(),
            cname: princ(NT_PRINCIPAL, &["bob"]),
            cksum: None,
            cusec: 7,
            ctime: KerberosTime::from_utc(2026, 1, 1, 0, 0, 0),
            subkey: None,
            seq_number: None,
        };
        assert_eq!(Authenticator::decode(&minimal.encode()).unwrap(), minimal);
    }

    #[test]
    fn ap_req_roundtrip() {
        let ap = ApReq {
            ap_options: AP_OPT_MUTUAL_REQUIRED,
            ticket: ticket(),
            authenticator: enc_data(),
        };
        assert_eq!(ApReq::decode(&ap.encode()).unwrap(), ap);
    }

    #[test]
    fn as_req_roundtrip() {
        let req = KdcReq {
            msg_type: KRB_AS_REQ,
            padata: vec![PaData {
                padata_type: PA_PAC_REQUEST,
                padata_value: vec![0x30, 0x05, 0xA0, 0x03, 0x01, 0x01, 0xFF],
            }],
            req_body: KdcReqBody {
                kdc_options: KDC_OPT_FORWARDABLE | KDC_OPT_RENEWABLE,
                cname: Some(princ(NT_PRINCIPAL, &["alice"])),
                realm: "EXAMPLE.COM".to_string(),
                sname: Some(princ(NT_SRV_INST, &["krbtgt", "EXAMPLE.COM"])),
                from: None,
                till: KerberosTime::from_utc(2026, 7, 18, 0, 0, 0),
                rtime: None,
                nonce: 0xDEAD_BEEF,
                etypes: vec![18, 17, 23],
            },
        };
        assert_eq!(KdcReq::decode(&req.encode()).unwrap(), req);
    }

    #[test]
    fn as_rep_roundtrip() {
        let rep = KdcRep {
            msg_type: KRB_AS_REP,
            padata: Vec::new(),
            crealm: "EXAMPLE.COM".to_string(),
            cname: princ(NT_PRINCIPAL, &["alice"]),
            ticket: ticket(),
            enc_part: enc_data(),
        };
        assert_eq!(KdcRep::decode(&rep.encode()).unwrap(), rep);
    }

    #[test]
    fn krb_error_roundtrip() {
        let err = KrbError {
            stime: KerberosTime::from_utc(2026, 7, 17, 12, 0, 0),
            susec: 42,
            error_code: 25, // KDC_ERR_PREAUTH_REQUIRED
            realm: "EXAMPLE.COM".to_string(),
            sname: princ(NT_SRV_INST, &["krbtgt", "EXAMPLE.COM"]),
            e_text: Some("NEEDED_PREAUTH".to_string()),
            e_data: Some(vec![0x01, 0x02, 0x03]),
        };
        assert_eq!(KrbError::decode(&err.encode()).unwrap(), err);
    }

    #[test]
    fn enc_kdc_rep_part_decodes_session_key() {
        // Build an EncASRepPart by hand with the mandatory fields and a couple
        // of skipped optionals, and confirm the key/nonce/sname come back.
        let mut body = Writer::new();
        let key = EncryptionKey {
            keytype: 23,
            keyvalue: vec![0x5A; 16],
        };
        write_context(&mut body, 0, &key.encode());
        // last-req: SEQUENCE OF SEQUENCE { [0] Int32, [1] KerberosTime }
        let mut lr_item = Writer::new();
        write_context_int32(&mut lr_item, 0, 0);
        write_context_time(
            &mut lr_item,
            1,
            &KerberosTime::from_utc(2026, 7, 17, 0, 0, 0),
        );
        let lr_item = sequence(lr_item.as_slice());
        let lr_seq = sequence(&lr_item);
        write_context(&mut body, 1, &lr_seq);
        write_context_uint32(&mut body, 2, 0xDEAD_BEEF);
        write_context_flags(&mut body, 4, KDC_OPT_FORWARDABLE);
        write_context_time(&mut body, 5, &KerberosTime::from_utc(2026, 7, 17, 12, 0, 0));
        write_context_time(&mut body, 7, &KerberosTime::from_utc(2026, 7, 18, 12, 0, 0));
        write_context_general_string(&mut body, 9, "EXAMPLE.COM");
        write_context(
            &mut body,
            10,
            &princ(NT_SRV_INST, &["krbtgt", "EXAMPLE.COM"]).encode(),
        );

        let part = application(APP_ENC_AS_REP_PART, &sequence(body.as_slice()));
        let decoded = EncKdcRepPart::decode(&part).unwrap();
        assert_eq!(decoded.key, key);
        assert_eq!(decoded.nonce, 0xDEAD_BEEF);
        assert_eq!(decoded.srealm, "EXAMPLE.COM");
        assert_eq!(
            decoded.endtime,
            KerberosTime::from_utc(2026, 7, 18, 12, 0, 0)
        );
    }
}
