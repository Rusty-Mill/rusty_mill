//! GSS-API / SPNEGO token wrapping (RFC 2743, RFC 4178, RFC 4121), std-only.
//!
//! RDP's NLA carries the Kerberos exchange inside SPNEGO tokens in the CredSSP
//! `negoTokens` field. This module builds and parses that wrapping:
//!
//! * DER `OBJECT IDENTIFIER` encoding ([`encode_oid`] / [`decode_oid`]) and the
//!   well-known mechanism OIDs.
//! * The GSS-API `InitialContextToken` framing ([`wrap_initial_context_token`])
//!   — `[APPLICATION 0] { mech OID, inner token }`.
//! * The GSS Kerberos mechanism token ([`gss_krb5_ap_req`]) — the `AP-REQ`
//!   prefixed with its GSS token id.
//! * SPNEGO [`NegTokenInit`] and [`NegTokenResp`] (RFC 4178).
//!
//! The per-message confidentiality tokens (RFC 4121 Wrap/CFX) that seal the
//! CredSSP public key and credentials with the Kerberos session key are a
//! separate, later step.

use crate::ber::{expect_tag, read_enumerated, write_enumerated, write_tlv, TAG_SEQUENCE};
use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

use super::asn1::{
    context_tag, expect_context, read_octet_string, write_context, write_context_octet_string,
};

/// DER `OBJECT IDENTIFIER` tag.
const TAG_OID: &[u8] = &[0x06];
/// GSS-API `InitialContextToken` tag (`[APPLICATION 0]`, constructed).
const TAG_APP0: &[u8] = &[0x60];

/// SPNEGO mechanism OID (`1.3.6.1.5.5.2`).
pub const SPNEGO_OID: &[u64] = &[1, 3, 6, 1, 5, 5, 2];
/// Kerberos v5 mechanism OID (`1.2.840.113554.1.2.2`).
pub const KRB5_OID: &[u64] = &[1, 2, 840, 113554, 1, 2, 2];
/// Microsoft Kerberos v5 mechanism OID (`1.2.840.48018.1.2.2`).
pub const MS_KRB5_OID: &[u64] = &[1, 2, 840, 48018, 1, 2, 2];

/// GSS token id prefixing a Kerberos `AP-REQ` (`KRB_AP_REQ`, RFC 4121 §4.1).
const TOK_ID_AP_REQ: [u8; 2] = [0x01, 0x00];

// SPNEGO negState values (RFC 4178 4.2.2).
/// `accept-completed`.
pub const NEG_STATE_ACCEPT_COMPLETED: u8 = 0;
/// `accept-incomplete`.
pub const NEG_STATE_ACCEPT_INCOMPLETE: u8 = 1;
/// `reject`.
pub const NEG_STATE_REJECT: u8 = 2;
/// `request-mic`.
pub const NEG_STATE_REQUEST_MIC: u8 = 3;

// ---------------------------------------------------------------------------
// OBJECT IDENTIFIER
// ---------------------------------------------------------------------------

/// Encode an OID (as a list of arcs) into a complete DER TLV.
pub fn encode_oid(arcs: &[u64]) -> Vec<u8> {
    let mut content = Vec::new();
    // First two arcs combine into one byte group: 40*arc0 + arc1.
    let first = 40 * arcs.first().copied().unwrap_or(0) + arcs.get(1).copied().unwrap_or(0);
    encode_base128(first, &mut content);
    for &arc in arcs.iter().skip(2) {
        encode_base128(arc, &mut content);
    }
    let mut w = Writer::new();
    write_tlv(&mut w, TAG_OID, &content);
    w.into_vec()
}

/// Append `value` as base-128 (big-endian, high bit set on all but the last).
fn encode_base128(value: u64, out: &mut Vec<u8>) {
    let mut stack = Vec::new();
    let mut v = value;
    loop {
        stack.push((v & 0x7F) as u8);
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    while let Some(byte) = stack.pop() {
        if stack.is_empty() {
            out.push(byte);
        } else {
            out.push(byte | 0x80);
        }
    }
}

/// Decode a DER OID TLV into its arcs.
pub fn decode_oid(tlv: &[u8]) -> Result<Vec<u64>> {
    let mut r = Reader::new(tlv);
    let len = expect_tag(&mut r, TAG_OID)?;
    let content = r.read_bytes(len)?;
    if content.is_empty() {
        return Err(Error::InvalidLength {
            field: "OID",
            length: 0,
        });
    }
    let mut arcs = Vec::new();
    // Decode base-128 groups.
    let mut groups = Vec::new();
    let mut value: u64 = 0;
    for &b in content {
        value = (value << 7) | (b & 0x7F) as u64;
        if b & 0x80 == 0 {
            groups.push(value);
            value = 0;
        }
    }
    // First group splits into the first two arcs.
    let first = groups[0];
    arcs.push(first / 40);
    arcs.push(first % 40);
    arcs.extend_from_slice(&groups[1..]);
    Ok(arcs)
}

// ---------------------------------------------------------------------------
// GSS-API InitialContextToken
// ---------------------------------------------------------------------------

/// Wrap `inner` in a GSS-API `InitialContextToken`: `[APPLICATION 0] { mech
/// OID, inner }`. `mech_oid` is a complete OID TLV (from [`encode_oid`]).
pub fn wrap_initial_context_token(mech_oid: &[u8], inner: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(mech_oid.len() + inner.len());
    body.extend_from_slice(mech_oid);
    body.extend_from_slice(inner);
    let mut w = Writer::new();
    write_tlv(&mut w, TAG_APP0, &body);
    w.into_vec()
}

/// Unwrap an `InitialContextToken`, returning `(mech_oid_arcs, inner_token)`.
pub fn unwrap_initial_context_token(token: &[u8]) -> Result<(Vec<u64>, Vec<u8>)> {
    let mut r = Reader::new(token);
    let len = expect_tag(&mut r, TAG_APP0)?;
    let body = r.read_bytes(len)?;
    // The mech OID is the first TLV; the rest is the inner token.
    let mut br = Reader::new(body);
    let oid_len = expect_tag(&mut br, TAG_OID)?;
    let oid_content = br.read_bytes(oid_len)?;
    let mut oid_tlv = Writer::new();
    write_tlv(&mut oid_tlv, TAG_OID, oid_content);
    let arcs = decode_oid(oid_tlv.as_slice())?;
    let inner = br.peek_remaining().to_vec();
    Ok((arcs, inner))
}

/// Wrap a Kerberos `AP-REQ` DER as a GSS Kerberos mechanism token.
pub fn gss_krb5_ap_req(ap_req_der: &[u8]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(2 + ap_req_der.len());
    inner.extend_from_slice(&TOK_ID_AP_REQ);
    inner.extend_from_slice(ap_req_der);
    wrap_initial_context_token(&encode_oid(KRB5_OID), &inner)
}

// ---------------------------------------------------------------------------
// SPNEGO
// ---------------------------------------------------------------------------

/// A SPNEGO `NegTokenInit` (the initiator's first token).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegTokenInit {
    /// The mechanism OIDs the initiator supports, in preference order.
    pub mech_types: Vec<Vec<u64>>,
    /// The optimistic mechanism token (e.g. the GSS Kerberos AP-REQ).
    pub mech_token: Option<Vec<u8>>,
}

impl NegTokenInit {
    /// Encode as a full GSS-wrapped SPNEGO initial token.
    pub fn to_vec(&self) -> Vec<u8> {
        // mechTypes [0] MechTypeList (SEQUENCE OF OID).
        let mut list = Writer::new();
        for arcs in &self.mech_types {
            list.write_bytes(&encode_oid(arcs));
        }
        let mut mech_type_list = Writer::new();
        write_tlv(&mut mech_type_list, TAG_SEQUENCE, list.as_slice());

        let mut body = Writer::new();
        write_context(&mut body, 0, mech_type_list.as_slice());
        if let Some(token) = &self.mech_token {
            write_context_octet_string(&mut body, 2, token);
        }
        // NegTokenInit SEQUENCE, wrapped in the negTokenInit [0] choice.
        let mut neg_init = Writer::new();
        write_tlv(&mut neg_init, TAG_SEQUENCE, body.as_slice());
        let mut choice = Writer::new();
        write_context(&mut choice, 0, neg_init.as_slice());

        // GSS InitialContextToken with the SPNEGO OID.
        wrap_initial_context_token(&encode_oid(SPNEGO_OID), choice.as_slice())
    }

    /// Decode a GSS-wrapped SPNEGO initial token.
    pub fn decode(token: &[u8]) -> Result<NegTokenInit> {
        let (oid, inner) = unwrap_initial_context_token(token)?;
        if oid != SPNEGO_OID {
            return Err(Error::InvalidValue {
                field: "SPNEGO mech OID",
                value: format!("{oid:?}"),
            });
        }
        let mut r = Reader::new(&inner);
        // negTokenInit [0] NegTokenInit.
        let choice_len = expect_context(&mut r, 0)?;
        let choice = r.read_bytes(choice_len)?;
        let mut r = Reader::new(choice);
        let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(seq_len)?;
        let mut r = Reader::new(body);

        // [0] mechTypes.
        let list_len = expect_context(&mut r, 0)?;
        let list = r.read_bytes(list_len)?;
        let mut lr = Reader::new(list);
        let inner_len = expect_tag(&mut lr, TAG_SEQUENCE)?;
        let oids = lr.read_bytes(inner_len)?;
        let mut or = Reader::new(oids);
        let mut mech_types = Vec::new();
        while !or.is_empty() {
            let oid_len = expect_tag(&mut or, TAG_OID)?;
            let oid_content = or.read_bytes(oid_len)?;
            let mut tlv = Writer::new();
            write_tlv(&mut tlv, TAG_OID, oid_content);
            mech_types.push(decode_oid(tlv.as_slice())?);
        }

        // Optional [1] reqFlags (skipped), [2] mechToken, [3] mechListMIC.
        let mut mech_token = None;
        while !r.is_empty() {
            if r.peek_remaining()[0] == context_tag(2)[0] {
                expect_context(&mut r, 2)?;
                mech_token = Some(read_octet_string(&mut r)?.to_vec());
            } else {
                skip_one_tlv(&mut r)?;
            }
        }
        Ok(NegTokenInit {
            mech_types,
            mech_token,
        })
    }
}

/// A SPNEGO `NegTokenResp` (a subsequent negotiation token).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NegTokenResp {
    /// The negotiation state (`NEG_STATE_*`).
    pub neg_state: Option<u8>,
    /// The selected mechanism OID.
    pub supported_mech: Option<Vec<u64>>,
    /// The responder's mechanism token (e.g. the Kerberos AP-REP).
    pub response_token: Option<Vec<u8>>,
    /// The mechanism-list MIC.
    pub mech_list_mic: Option<Vec<u8>>,
}

impl NegTokenResp {
    /// Encode as a bare `negTokenResp [1]` negotiation token (not GSS-wrapped).
    pub fn to_vec(&self) -> Vec<u8> {
        let mut body = Writer::new();
        if let Some(state) = self.neg_state {
            let mut inner = Writer::new();
            write_enumerated(&mut inner, state);
            write_context(&mut body, 0, inner.as_slice());
        }
        if let Some(mech) = &self.supported_mech {
            write_context(&mut body, 1, &encode_oid(mech));
        }
        if let Some(token) = &self.response_token {
            write_context_octet_string(&mut body, 2, token);
        }
        if let Some(mic) = &self.mech_list_mic {
            write_context_octet_string(&mut body, 3, mic);
        }
        let mut seq = Writer::new();
        write_tlv(&mut seq, TAG_SEQUENCE, body.as_slice());
        let mut choice = Writer::new();
        write_context(&mut choice, 1, seq.as_slice());
        choice.into_vec()
    }

    /// Decode a `negTokenResp [1]` negotiation token.
    pub fn decode(token: &[u8]) -> Result<NegTokenResp> {
        let mut r = Reader::new(token);
        let choice_len = expect_context(&mut r, 1)?;
        let choice = r.read_bytes(choice_len)?;
        let mut r = Reader::new(choice);
        let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(seq_len)?;
        let mut r = Reader::new(body);

        let mut resp = NegTokenResp::default();
        while !r.is_empty() {
            let tag = r.peek_remaining()[0];
            match tag {
                t if t == context_tag(0)[0] => {
                    expect_context(&mut r, 0)?;
                    resp.neg_state = Some(read_enumerated(&mut r)?);
                }
                t if t == context_tag(1)[0] => {
                    let len = expect_context(&mut r, 1)?;
                    let oid = r.read_bytes(len)?;
                    resp.supported_mech = Some(decode_oid(oid)?);
                }
                t if t == context_tag(2)[0] => {
                    expect_context(&mut r, 2)?;
                    resp.response_token = Some(read_octet_string(&mut r)?.to_vec());
                }
                t if t == context_tag(3)[0] => {
                    expect_context(&mut r, 3)?;
                    resp.mech_list_mic = Some(read_octet_string(&mut r)?.to_vec());
                }
                other => {
                    return Err(Error::InvalidValue {
                        field: "NegTokenResp field tag",
                        value: format!("0x{other:02X}"),
                    });
                }
            }
        }
        Ok(resp)
    }
}

/// Build the full initial SPNEGO token that offers Kerberos and carries the
/// AP-REQ as the optimistic mechanism token — ready for CredSSP `negoTokens`.
pub fn spnego_init_kerberos(ap_req_der: &[u8]) -> Vec<u8> {
    NegTokenInit {
        mech_types: vec![KRB5_OID.to_vec(), MS_KRB5_OID.to_vec()],
        mech_token: Some(gss_krb5_ap_req(ap_req_der)),
    }
    .to_vec()
}

/// Skip one DER TLV (single-byte tag) at the cursor.
fn skip_one_tlv(r: &mut Reader<'_>) -> Result<()> {
    let _tag = r.read_u8()?;
    let len = crate::ber::read_length(r)?;
    r.read_bytes(len)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn oid_encoding_matches_known_der() {
        // SPNEGO 1.3.6.1.5.5.2.
        assert_eq!(hex(&encode_oid(SPNEGO_OID)), "06062b0601050502");
        // Kerberos 5 1.2.840.113554.1.2.2.
        assert_eq!(hex(&encode_oid(KRB5_OID)), "06092a864886f712010202");
    }

    #[test]
    fn oid_roundtrip() {
        for arcs in [SPNEGO_OID, KRB5_OID, MS_KRB5_OID] {
            let der = encode_oid(arcs);
            assert_eq!(decode_oid(&der).unwrap(), arcs);
        }
    }

    #[test]
    fn initial_context_token_roundtrip() {
        let inner = [0xDE, 0xAD, 0xBE, 0xEF];
        let token = wrap_initial_context_token(&encode_oid(KRB5_OID), &inner);
        assert_eq!(token[0], 0x60);
        let (oid, got) = unwrap_initial_context_token(&token).unwrap();
        assert_eq!(oid, KRB5_OID);
        assert_eq!(got, inner);
    }

    #[test]
    fn gss_krb5_token_prefixes_tok_id() {
        let ap_req = [0x6e, 0x05, 0x01, 0x02, 0x03]; // stand-in AP-REQ
        let token = gss_krb5_ap_req(&ap_req);
        let (oid, inner) = unwrap_initial_context_token(&token).unwrap();
        assert_eq!(oid, KRB5_OID);
        assert_eq!(&inner[..2], &[0x01, 0x00]);
        assert_eq!(&inner[2..], &ap_req);
    }

    #[test]
    fn neg_token_init_roundtrip() {
        let init = NegTokenInit {
            mech_types: vec![KRB5_OID.to_vec(), MS_KRB5_OID.to_vec()],
            mech_token: Some(vec![0x01, 0x00, 0xAA, 0xBB]),
        };
        let encoded = init.to_vec();
        assert_eq!(encoded[0], 0x60); // GSS InitialContextToken
        assert_eq!(NegTokenInit::decode(&encoded).unwrap(), init);
    }

    #[test]
    fn spnego_init_kerberos_shape() {
        let ap_req = [0x6e, 0x03, 0x01, 0x02, 0x03];
        let token = spnego_init_kerberos(&ap_req);
        let decoded = NegTokenInit::decode(&token).unwrap();
        assert_eq!(decoded.mech_types[0], KRB5_OID);
        // The mechToken is the GSS-wrapped AP-REQ.
        let (oid, inner) =
            unwrap_initial_context_token(decoded.mech_token.as_ref().unwrap()).unwrap();
        assert_eq!(oid, KRB5_OID);
        assert_eq!(&inner[..2], &[0x01, 0x00]);
    }

    #[test]
    fn neg_token_resp_roundtrip() {
        let resp = NegTokenResp {
            neg_state: Some(NEG_STATE_ACCEPT_COMPLETED),
            supported_mech: Some(KRB5_OID.to_vec()),
            response_token: Some(vec![0x02, 0x00, 0xCC, 0xDD]),
            mech_list_mic: Some(vec![0x11; 12]),
        };
        let encoded = resp.to_vec();
        assert_eq!(encoded[0], context_tag(1)[0]);
        assert_eq!(NegTokenResp::decode(&encoded).unwrap(), resp);
    }

    #[test]
    fn neg_token_resp_minimal() {
        let resp = NegTokenResp {
            neg_state: Some(NEG_STATE_ACCEPT_INCOMPLETE),
            response_token: Some(vec![0xAB, 0xCD]),
            ..Default::default()
        };
        assert_eq!(NegTokenResp::decode(&resp.to_vec()).unwrap(), resp);
    }
}
