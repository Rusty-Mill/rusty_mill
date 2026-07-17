//! CredSSP / Network Level Authentication (MS-CSSP), std-only.
//!
//! CredSSP runs *inside* the TLS tunnel that the enhanced-security negotiation
//! set up. It carries an SSPI authentication exchange (here, NTLMv2 from
//! [`crate::ntlm`]) wrapped in `TSRequest` ASN.1 structures, then proves the
//! TLS channel binding with the server's public key and finally delegates the
//! user's credentials — all encrypted with the NTLM session
//! ([`crate::ntlm::NtlmContext`]).
//!
//! [`CredSspClient`] drives the client half as a small state machine:
//!
//! 1. [`CredSspClient::negotiate_request`] → send the NTLM NEGOTIATE.
//! 2. [`CredSspClient::challenge_response`] ← server CHALLENGE, → NTLM
//!    AUTHENTICATE plus the sealed public-key token (`pubKeyAuth`).
//! 3. [`CredSspClient::finish`] ← server's public-key confirmation, → the
//!    sealed [`TSCredentials`](https://learn.microsoft.com/openspecs/windows_protocols/ms-cssp)
//!    (`authInfo`).
//!
//! The public-key binding uses the version 5+ nonce hash (SHA-256 of a client
//! nonce and the server key) when the server advertises CredSSP ≥ 5, and the
//! legacy "public key + 1" scheme otherwise.

use crate::ber::{
    expect_tag, read_integer, read_octet_string, write_integer, write_octet_string, write_tlv,
    TAG_SEQUENCE,
};
use crate::crypto::sha256::sha256;
use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};
use crate::ntlm::{NtlmClient, NtlmContext};

/// The CredSSP protocol version this client offers.
pub const CLIENT_VERSION: u32 = 6;

const CLIENT_TO_SERVER_MAGIC: &[u8] = b"CredSSP Client-To-Server Binding Hash\0";
const SERVER_TO_CLIENT_MAGIC: &[u8] = b"CredSSP Server-To-Client Binding Hash\0";

/// Context tag `[n]` (constructed, context-specific).
fn ctx_tag(n: u8) -> [u8; 1] {
    [0xA0 + n]
}

// ---------------------------------------------------------------------------
// TSRequest (MS-CSSP 2.2.1)
// ---------------------------------------------------------------------------

/// A decoded / to-be-encoded `TSRequest`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TsRequest {
    /// Protocol version.
    pub version: u32,
    /// SSPI tokens (`NegoData`); each entry is one `negoToken` OCTET STRING.
    pub nego_tokens: Vec<Vec<u8>>,
    /// Encrypted `TSCredentials` (`authInfo`).
    pub auth_info: Option<Vec<u8>>,
    /// Encrypted public-key confirmation (`pubKeyAuth`).
    pub pub_key_auth: Option<Vec<u8>>,
    /// Server error code, if any (`errorCode`).
    pub error_code: Option<u32>,
    /// Client nonce for the version 5+ binding (`clientNonce`).
    pub client_nonce: Option<Vec<u8>>,
}

impl TsRequest {
    /// Encode this request to DER.
    pub fn to_vec(&self) -> Vec<u8> {
        let mut body = Writer::new();

        // [0] version INTEGER
        let mut v = Writer::new();
        write_integer(&mut v, self.version);
        write_tlv(&mut body, &ctx_tag(0), v.as_slice());

        // [1] negoTokens NegoData ::= SEQUENCE OF SEQUENCE { negoToken [0] OCTET STRING }
        if !self.nego_tokens.is_empty() {
            let mut seq_of = Writer::new();
            for token in &self.nego_tokens {
                let mut inner = Writer::new();
                let mut os = Writer::new();
                write_octet_string(&mut os, token);
                write_tlv(&mut inner, &ctx_tag(0), os.as_slice());
                write_tlv(&mut seq_of, TAG_SEQUENCE, inner.as_slice());
            }
            let mut seq = Writer::new();
            write_tlv(&mut seq, TAG_SEQUENCE, seq_of.as_slice());
            write_tlv(&mut body, &ctx_tag(1), seq.as_slice());
        }

        // [2] authInfo OCTET STRING
        if let Some(auth) = &self.auth_info {
            let mut os = Writer::new();
            write_octet_string(&mut os, auth);
            write_tlv(&mut body, &ctx_tag(2), os.as_slice());
        }

        // [3] pubKeyAuth OCTET STRING
        if let Some(pk) = &self.pub_key_auth {
            let mut os = Writer::new();
            write_octet_string(&mut os, pk);
            write_tlv(&mut body, &ctx_tag(3), os.as_slice());
        }

        // [4] errorCode INTEGER
        if let Some(code) = self.error_code {
            let mut v = Writer::new();
            write_integer(&mut v, code);
            write_tlv(&mut body, &ctx_tag(4), v.as_slice());
        }

        // [5] clientNonce OCTET STRING
        if let Some(nonce) = &self.client_nonce {
            let mut os = Writer::new();
            write_octet_string(&mut os, nonce);
            write_tlv(&mut body, &ctx_tag(5), os.as_slice());
        }

        let mut out = Writer::new();
        write_tlv(&mut out, TAG_SEQUENCE, body.as_slice());
        out.into_vec()
    }

    /// Decode a `TSRequest` from DER.
    pub fn decode(buf: &[u8]) -> Result<TsRequest> {
        let mut r = Reader::new(buf);
        let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let body = r.read_bytes(seq_len)?;
        let mut r = Reader::new(body);

        let mut req = TsRequest::default();
        while !r.is_empty() {
            let tag = r.peek_remaining()[0];
            match tag {
                0xA0 => {
                    expect_tag(&mut r, &ctx_tag(0))?;
                    req.version = read_integer(&mut r)?;
                }
                0xA1 => {
                    let len = expect_tag(&mut r, &ctx_tag(1))?;
                    let inner = r.read_bytes(len)?;
                    req.nego_tokens = decode_nego_data(inner)?;
                }
                0xA2 => {
                    expect_tag(&mut r, &ctx_tag(2))?;
                    req.auth_info = Some(read_octet_string(&mut r)?.to_vec());
                }
                0xA3 => {
                    expect_tag(&mut r, &ctx_tag(3))?;
                    req.pub_key_auth = Some(read_octet_string(&mut r)?.to_vec());
                }
                0xA4 => {
                    expect_tag(&mut r, &ctx_tag(4))?;
                    req.error_code = Some(read_integer(&mut r)?);
                }
                0xA5 => {
                    expect_tag(&mut r, &ctx_tag(5))?;
                    req.client_nonce = Some(read_octet_string(&mut r)?.to_vec());
                }
                other => {
                    return Err(Error::InvalidValue {
                        field: "TSRequest field tag",
                        value: format!("0x{other:02X}"),
                    });
                }
            }
        }
        Ok(req)
    }
}

/// Decode `NegoData ::= SEQUENCE OF SEQUENCE { negoToken [0] OCTET STRING }`.
fn decode_nego_data(buf: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut r = Reader::new(buf);
    let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
    let items = r.read_bytes(seq_len)?;
    let mut r = Reader::new(items);
    let mut tokens = Vec::new();
    while !r.is_empty() {
        let item_len = expect_tag(&mut r, TAG_SEQUENCE)?;
        let item = r.read_bytes(item_len)?;
        let mut ir = Reader::new(item);
        expect_tag(&mut ir, &ctx_tag(0))?;
        tokens.push(read_octet_string(&mut ir)?.to_vec());
    }
    Ok(tokens)
}

// ---------------------------------------------------------------------------
// TSCredentials / TSPasswordCreds (MS-CSSP 2.2.1.2)
// ---------------------------------------------------------------------------

/// Encode `TSPasswordCreds { domainName, userName, password }` (UTF-16LE
/// strings) then wrap it in `TSCredentials { credType = 1, credentials }`.
pub fn encode_ts_credentials(domain: &str, user: &str, password: &str) -> Vec<u8> {
    let utf16le = |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect() };

    // TSPasswordCreds.
    let mut pwd_body = Writer::new();
    for (i, field) in [domain, user, password].iter().enumerate() {
        let mut os = Writer::new();
        write_octet_string(&mut os, &utf16le(field));
        write_tlv(&mut pwd_body, &ctx_tag(i as u8), os.as_slice());
    }
    let mut pwd_creds = Writer::new();
    write_tlv(&mut pwd_creds, TAG_SEQUENCE, pwd_body.as_slice());

    // TSCredentials.
    let mut body = Writer::new();
    let mut ct = Writer::new();
    write_integer(&mut ct, 1); // credType 1 = password credentials
    write_tlv(&mut body, &ctx_tag(0), ct.as_slice());
    let mut os = Writer::new();
    write_octet_string(&mut os, &pwd_creds.into_vec());
    write_tlv(&mut body, &ctx_tag(1), os.as_slice());

    let mut out = Writer::new();
    write_tlv(&mut out, TAG_SEQUENCE, body.as_slice());
    out.into_vec()
}

// ---------------------------------------------------------------------------
// CredSSP client state machine
// ---------------------------------------------------------------------------

/// Drives the CredSSP client exchange over an already-established TLS channel.
///
/// The nondeterministic inputs (NTLM challenge/timestamp/session key and the
/// CredSSP nonce) are injected so the exchange is testable; production callers
/// pass real random bytes (e.g. from the OS).
pub struct CredSspClient {
    ntlm: NtlmClient,
    context: Option<NtlmContext>,
    server_version: u32,
    public_key: Vec<u8>,
    nonce: [u8; 32],
    domain: String,
    user: String,
    password: String,
}

impl CredSspClient {
    /// Create a client. `public_key` is the server's TLS `SubjectPublicKeyInfo`
    /// (used for the channel binding).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        domain: &str,
        user: &str,
        password: &str,
        workstation: &str,
        public_key: Vec<u8>,
        nonce: [u8; 32],
        client_challenge: [u8; 8],
        timestamp: [u8; 8],
        exported_session_key: [u8; 16],
    ) -> Self {
        CredSspClient {
            ntlm: NtlmClient::new(
                domain,
                user,
                password,
                workstation,
                client_challenge,
                timestamp,
                exported_session_key,
            ),
            context: None,
            server_version: 0,
            public_key,
            nonce,
            domain: domain.to_string(),
            user: user.to_string(),
            password: password.to_string(),
        }
    }

    /// Leg 1: the `TSRequest` carrying the NTLM NEGOTIATE token.
    pub fn negotiate_request(&mut self) -> Vec<u8> {
        let negotiate = self.ntlm.negotiate();
        TsRequest {
            version: CLIENT_VERSION,
            nego_tokens: vec![negotiate],
            ..Default::default()
        }
        .to_vec()
    }

    /// Leg 3: consume the server's CHALLENGE `TSRequest` and return the
    /// `TSRequest` carrying the NTLM AUTHENTICATE plus the sealed public-key
    /// token.
    pub fn challenge_response(&mut self, server_msg: &[u8]) -> Result<Vec<u8>> {
        let request = TsRequest::decode(server_msg)?;
        check_error(&request)?;
        self.server_version = request.version;
        let challenge = request.nego_tokens.first().ok_or(Error::InvalidValue {
            field: "CredSSP challenge",
            value: "no negoToken".to_string(),
        })?;

        let (authenticate, mut context) = self.ntlm.authenticate(challenge)?;

        // Bind the TLS channel: seal the (hashed) public key.
        let bound = self.client_public_key_binding();
        let pub_key_auth = context.encrypt_message(&bound);
        self.context = Some(context);

        let effective = CLIENT_VERSION.min(self.server_version);
        let client_nonce = if effective >= 5 {
            Some(self.nonce.to_vec())
        } else {
            None
        };

        Ok(TsRequest {
            version: CLIENT_VERSION,
            nego_tokens: vec![authenticate],
            pub_key_auth: Some(pub_key_auth),
            client_nonce,
            ..Default::default()
        }
        .to_vec())
    }

    /// Leg 5: verify the server's public-key confirmation and return the
    /// `TSRequest` carrying the sealed credentials (`authInfo`).
    pub fn finish(&mut self, server_msg: &[u8]) -> Result<Vec<u8>> {
        let request = TsRequest::decode(server_msg)?;
        check_error(&request)?;
        let pub_key_auth = request.pub_key_auth.ok_or(Error::InvalidValue {
            field: "CredSSP server response",
            value: "missing pubKeyAuth".to_string(),
        })?;

        // Compute the expected binding and the credentials before taking the
        // mutable borrow of the context.
        let expected = self.server_public_key_binding();
        let credentials = encode_ts_credentials(&self.domain, &self.user, &self.password);

        let context = self.context.as_mut().ok_or(Error::InvalidValue {
            field: "CredSSP state",
            value: "no security context".to_string(),
        })?;
        let received = context.decrypt_message(&pub_key_auth)?;
        if received != expected {
            return Err(Error::InvalidValue {
                field: "CredSSP public key confirmation",
                value: "mismatch".to_string(),
            });
        }

        // Delegate the credentials, sealed with the NTLM context.
        let auth_info = context.encrypt_message(&credentials);
        Ok(TsRequest {
            version: CLIENT_VERSION,
            auth_info: Some(auth_info),
            ..Default::default()
        }
        .to_vec())
    }

    /// The negotiated CredSSP version (min of client and server).
    fn effective_version(&self) -> u32 {
        CLIENT_VERSION.min(self.server_version)
    }

    /// The client's public-key binding value (before sealing). Called after
    /// `server_version` is known.
    fn client_public_key_binding(&self) -> Vec<u8> {
        if self.effective_version() >= 5 {
            hash_binding(CLIENT_TO_SERVER_MAGIC, &self.nonce, &self.public_key)
        } else {
            self.public_key.clone()
        }
    }

    /// The server's expected public-key binding value (before it was sealed).
    fn server_public_key_binding(&self) -> Vec<u8> {
        if self.effective_version() >= 5 {
            hash_binding(SERVER_TO_CLIENT_MAGIC, &self.nonce, &self.public_key)
        } else {
            // Legacy: the server increments the first byte of the public key.
            let mut pk = self.public_key.clone();
            if let Some(first) = pk.first_mut() {
                *first = first.wrapping_add(1);
            }
            pk
        }
    }
}

/// `SHA256(magic || nonce || public_key)` — the version 5+ channel binding.
fn hash_binding(magic: &[u8], nonce: &[u8; 32], public_key: &[u8]) -> Vec<u8> {
    let mut input = Vec::with_capacity(magic.len() + 32 + public_key.len());
    input.extend_from_slice(magic);
    input.extend_from_slice(nonce);
    input.extend_from_slice(public_key);
    sha256(&input).to_vec()
}

/// Turn a server-reported `errorCode` into an error.
fn check_error(request: &TsRequest) -> Result<()> {
    match request.error_code {
        Some(0) | None => Ok(()),
        Some(code) => Err(Error::InvalidValue {
            field: "CredSSP server errorCode",
            value: format!("0x{code:08X}"),
        }),
    }
}

/// Read the CredSSP `version` from a `TSRequest` without decoding the rest.
///
/// A tiny helper used to sanity-check that we are looking at a well-formed
/// request when reading is best-effort.
pub fn peek_version(buf: &[u8]) -> Result<u32> {
    let mut r = Reader::new(buf);
    let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
    let body = r.read_bytes(seq_len)?;
    let mut r = Reader::new(body);
    expect_tag(&mut r, &ctx_tag(0))?;
    read_integer(&mut r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_request_roundtrip_all_fields() {
        let req = TsRequest {
            version: 6,
            nego_tokens: vec![vec![1, 2, 3], vec![4, 5]],
            auth_info: Some(vec![0xAA; 40]),
            pub_key_auth: Some(vec![0xBB; 24]),
            error_code: Some(0),
            client_nonce: Some(vec![0xCC; 32]),
        };
        let encoded = req.to_vec();
        let decoded = TsRequest::decode(&encoded).unwrap();
        assert_eq!(decoded, req);
    }

    #[test]
    fn ts_request_minimal_negotiate() {
        let req = TsRequest {
            version: 6,
            nego_tokens: vec![b"NTLMSSP\0negotiate".to_vec()],
            ..Default::default()
        };
        let encoded = req.to_vec();
        assert_eq!(peek_version(&encoded).unwrap(), 6);
        let decoded = TsRequest::decode(&encoded).unwrap();
        assert_eq!(decoded.nego_tokens.len(), 1);
        assert_eq!(decoded.nego_tokens[0], b"NTLMSSP\0negotiate");
        assert!(decoded.auth_info.is_none());
    }

    #[test]
    fn ts_credentials_encodes_password_creds() {
        let creds = encode_ts_credentials("CORP", "alice", "secret");
        // Outer TSCredentials is a SEQUENCE.
        assert_eq!(creds[0], TAG_SEQUENCE[0]);
        // credType = 1 appears as [0] INTEGER 1: A0 03 02 01 01.
        assert!(creds
            .windows(5)
            .any(|w| w == [0xA0, 0x03, 0x02, 0x01, 0x01]));
    }

    #[test]
    fn server_error_code_is_surfaced() {
        let req = TsRequest {
            version: 6,
            error_code: Some(0xC0000022),
            ..Default::default()
        };
        assert!(check_error(&req).is_err());
    }

    #[test]
    fn full_credssp_exchange_against_mock_server() {
        // Build a mock server that mirrors the NTLM keys, so it can verify the
        // client's sealed tokens and produce its own public-key confirmation.
        let public_key = vec![0x30, 0x82, 0x01, 0x0A, 0xDE, 0xAD, 0xBE, 0xEF]; // stand-in SPKI
        let nonce = [0x77u8; 32];
        let esk = [0x55u8; 16];
        let client_challenge = [0xAAu8; 8];

        let mut client = CredSspClient::new(
            "Domain",
            "User",
            "Password",
            "WKS",
            public_key.clone(),
            nonce,
            client_challenge,
            [0u8; 8],
            esk,
        );

        // Leg 1.
        let leg1 = client.negotiate_request();
        let req1 = TsRequest::decode(&leg1).unwrap();
        assert_eq!(req1.nego_tokens.len(), 1);

        // Server builds a CHALLENGE (reusing the test helper shape).
        let challenge = mock_challenge();
        let leg2 = TsRequest {
            version: 6,
            nego_tokens: vec![challenge.clone()],
            ..Default::default()
        }
        .to_vec();

        // Leg 3: client authenticates and seals the public-key hash.
        let leg3 = client.challenge_response(&leg2).unwrap();
        let req3 = TsRequest::decode(&leg3).unwrap();
        assert!(req3.pub_key_auth.is_some());
        assert!(req3.client_nonce.is_some());

        // The mock server recreates the NTLM context from the same keys and
        // verifies the client's pubKeyAuth, then produces its own.
        let mut server_ctx = mirror_context(&esk);
        let received = server_ctx
            .decrypt_message(req3.pub_key_auth.as_ref().unwrap())
            .unwrap();
        let expected_client = hash_binding(CLIENT_TO_SERVER_MAGIC, &nonce, &public_key);
        assert_eq!(received, expected_client);

        let server_binding = hash_binding(SERVER_TO_CLIENT_MAGIC, &nonce, &public_key);
        let server_pub_key_auth = server_ctx.encrypt_message(&server_binding);
        let leg4 = TsRequest {
            version: 6,
            pub_key_auth: Some(server_pub_key_auth),
            ..Default::default()
        }
        .to_vec();

        // Leg 5: client verifies and delegates credentials.
        let leg5 = client.finish(&leg4).unwrap();
        let req5 = TsRequest::decode(&leg5).unwrap();
        let auth_info = req5.auth_info.unwrap();
        let credentials = server_ctx.decrypt_message(&auth_info).unwrap();
        // The delegated credentials decode to our TSCredentials.
        assert_eq!(
            credentials,
            encode_ts_credentials("Domain", "User", "Password")
        );
    }

    /// A minimal NTLM CHALLENGE_MESSAGE the mock server would send.
    fn mock_challenge() -> Vec<u8> {
        let signature = *b"NTLMSSP\0";
        let server_challenge = [0x01u8, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
        // Minimal target info: just EOL.
        let target_info = [0u8, 0, 0, 0];
        let version = [0x06, 0x01, 0xB1, 0x1D, 0x00, 0x00, 0x00, 0x0F];
        let ti_offset = 48u32 + 8;
        let mut m = Vec::new();
        m.extend_from_slice(&signature);
        m.extend_from_slice(&2u32.to_le_bytes());
        m.extend_from_slice(&[0u8; 8]); // TargetNameFields
        m.extend_from_slice(&0x6008_8215u32.to_le_bytes()); // some flags
        m.extend_from_slice(&server_challenge);
        m.extend_from_slice(&[0u8; 8]); // Reserved
        m.extend_from_slice(&(target_info.len() as u16).to_le_bytes());
        m.extend_from_slice(&(target_info.len() as u16).to_le_bytes());
        m.extend_from_slice(&ti_offset.to_le_bytes());
        m.extend_from_slice(&version);
        m.extend_from_slice(&target_info);
        m
    }

    /// Build a server-side NTLM context whose directions mirror the client's:
    /// its server-direction keys equal the client's client-direction keys.
    fn mirror_context(esk: &[u8; 16]) -> NtlmContext {
        NtlmContext::mirror_for_test(esk)
    }
}
