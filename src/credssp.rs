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
//!
//! [`KerberosCredSspClient`] is the Kerberos counterpart: it carries the
//! SPNEGO-wrapped `AP-REQ` in `negoTokens` and seals the public key and
//! credentials with the Kerberos session key (RFC 4121 Wrap tokens,
//! [`crate::krb5::cfx`]) instead of the NTLM context.
//!
//! [`CredSspServer`] drives the NTLM-only server (acceptor) side — the same
//! three legs in reverse, ending with the delegated `(domain, user,
//! password)` recovered from `authInfo`. It verifies the client's password
//! via a caller-supplied hash lookup (see [`crate::ntlm::NtlmServer`]);
//! there is no Kerberos-accepting counterpart, since validating an `AP-REQ`
//! needs a keytab and a much larger surface than this crate implements.

use crate::ber::{
    expect_tag, read_integer, read_octet_string, write_integer, write_octet_string, write_tlv,
    TAG_SEQUENCE,
};
use crate::crypto::sha256::sha256;
use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};
use crate::ntlm::{NtlmClient, NtlmContext, NtlmServer};

/// The CredSSP protocol version this client offers.
pub const CLIENT_VERSION: u32 = 6;
/// The CredSSP protocol version [`CredSspServer`] offers.
pub const SERVER_VERSION: u32 = 6;
/// `STATUS_LOGON_FAILURE` — the conventional NTSTATUS servers report in a
/// CredSSP `TSRequest.errorCode` to reject a client (see
/// [`encode_error_response`]).
pub const STATUS_LOGON_FAILURE: u32 = 0xC000_006D;

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

/// Decode a `TSCredentials { credType = 1, credentials }` wrapping
/// `TSPasswordCreds { domainName, userName, password }` back into
/// `(domain, user, password)` — the counterpart to [`encode_ts_credentials`],
/// used by [`CredSspServer::finish`] to recover the delegated credentials.
pub fn decode_ts_credentials(buf: &[u8]) -> Result<(String, String, String)> {
    let mut r = Reader::new(buf);
    let seq_len = expect_tag(&mut r, TAG_SEQUENCE)?;
    let body = r.read_bytes(seq_len)?;
    let mut r = Reader::new(body);

    expect_tag(&mut r, &ctx_tag(0))?;
    let _cred_type = read_integer(&mut r)?;
    expect_tag(&mut r, &ctx_tag(1))?;
    let pwd_creds = read_octet_string(&mut r)?;

    let mut pr = Reader::new(pwd_creds);
    let pwd_seq_len = expect_tag(&mut pr, TAG_SEQUENCE)?;
    let pwd_body = pr.read_bytes(pwd_seq_len)?;
    let mut pr = Reader::new(pwd_body);

    let mut fields = Vec::with_capacity(3);
    for i in 0..3u8 {
        expect_tag(&mut pr, &ctx_tag(i))?;
        fields.push(crate::ntlm::utf16le_decode(read_octet_string(&mut pr)?)?);
    }
    Ok((fields[0].clone(), fields[1].clone(), fields[2].clone()))
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

/// Encode a `TSRequest` carrying only an `errorCode` — send this after
/// [`CredSspServer::verify_authenticate`] returns an error, so the client
/// sees a clean rejection instead of a dropped connection. Pass
/// [`STATUS_LOGON_FAILURE`] unless a more specific NTSTATUS applies.
pub fn encode_error_response(code: u32) -> Vec<u8> {
    TsRequest {
        version: SERVER_VERSION,
        error_code: Some(code),
        ..Default::default()
    }
    .to_vec()
}

// ---------------------------------------------------------------------------
// CredSSP server state machine
// ---------------------------------------------------------------------------

/// Drives the CredSSP server (acceptor) exchange over an already-established
/// TLS channel, verifying the client's NTLM authentication and recovering
/// its delegated credentials.
///
/// Mirrors [`CredSspClient`]'s three legs from the server's side:
///
/// 1. [`CredSspServer::challenge_response`] ← client NEGOTIATE, → NTLM
///    CHALLENGE.
/// 2. [`CredSspServer::verify_authenticate`] ← client AUTHENTICATE +
///    `pubKeyAuth`, → the server's own sealed public-key confirmation. This
///    is where the `hash_lookup` callback passed to [`CredSspServer::new`]
///    is consulted; on failure, reply with [`encode_error_response`] instead
///    of this method's `Ok` value.
/// 3. [`CredSspServer::finish`] ← the sealed `authInfo`, returning the
///    delegated `(domain, user, password)` to log the client on with.
///
/// Only NTLM is supported server-side — there is no Kerberos-accepting
/// counterpart here, since validating a Kerberos `AP-REQ` needs a keytab (or
/// equivalent long-term key access) and a much larger surface than this
/// crate implements.
pub struct CredSspServer<F: Fn(&str, &str) -> Option<[u8; 16]>> {
    ntlm: NtlmServer<F>,
    context: Option<NtlmContext>,
    client_version: u32,
    public_key: Vec<u8>,
    nonce: Option<[u8; 32]>,
}

impl<F: Fn(&str, &str) -> Option<[u8; 16]>> CredSspServer<F> {
    /// Create a server. `public_key` is this server's own TLS
    /// `SubjectPublicKeyInfo` (the same bytes a peer's `connect_tls` extracts
    /// from the certificate it receives — used for the channel binding).
    /// `target_name`/`server_challenge`/`timestamp` feed [`NtlmServer::new`];
    /// `hash_lookup` is the password-hash callback (see [`NtlmServer`]).
    pub fn new(
        target_name: &str,
        server_challenge: [u8; 8],
        timestamp: [u8; 8],
        public_key: Vec<u8>,
        hash_lookup: F,
    ) -> Self {
        CredSspServer {
            ntlm: NtlmServer::new(target_name, server_challenge, timestamp, hash_lookup),
            context: None,
            client_version: 0,
            public_key,
            nonce: None,
        }
    }

    /// Leg 2: consume the client's `TSRequest` carrying the NTLM NEGOTIATE
    /// token and return the `TSRequest` carrying the CHALLENGE.
    pub fn challenge_response(&mut self, client_msg: &[u8]) -> Result<Vec<u8>> {
        let request = TsRequest::decode(client_msg)?;
        self.client_version = request.version;
        let negotiate = request.nego_tokens.first().ok_or(Error::InvalidValue {
            field: "CredSSP negotiate",
            value: "no negoToken".to_string(),
        })?;
        let challenge = self.ntlm.challenge(negotiate)?;
        Ok(TsRequest {
            version: self.effective_version(),
            nego_tokens: vec![challenge],
            ..Default::default()
        }
        .to_vec())
    }

    /// Leg 4: consume the client's `TSRequest` carrying the NTLM
    /// AUTHENTICATE token and its sealed public-key binding. Verifies the
    /// password (via the `hash_lookup` callback) and the channel binding,
    /// then returns the `TSRequest` carrying this server's own sealed
    /// public-key confirmation.
    pub fn verify_authenticate(&mut self, client_msg: &[u8]) -> Result<Vec<u8>> {
        let request = TsRequest::decode(client_msg)?;
        let authenticate = request.nego_tokens.first().ok_or(Error::InvalidValue {
            field: "CredSSP authenticate",
            value: "no negoToken".to_string(),
        })?;
        let pub_key_auth = request.pub_key_auth.ok_or(Error::InvalidValue {
            field: "CredSSP client response",
            value: "missing pubKeyAuth".to_string(),
        })?;

        let (_domain, _user, mut context) = self.ntlm.authenticate(authenticate)?;

        let expected = self.client_public_key_binding(request.client_nonce.as_deref());
        let received = context.decrypt_message(&pub_key_auth)?;
        if received != expected {
            return Err(Error::InvalidValue {
                field: "CredSSP public key confirmation",
                value: "mismatch".to_string(),
            });
        }
        self.nonce = request.client_nonce.map(|n| {
            let mut a = [0u8; 32];
            let len = n.len().min(32);
            a[..len].copy_from_slice(&n[..len]);
            a
        });

        let outgoing = self.server_public_key_binding();
        let sealed = context.encrypt_message(&outgoing);
        self.context = Some(context);

        Ok(TsRequest {
            version: self.effective_version(),
            pub_key_auth: Some(sealed),
            ..Default::default()
        }
        .to_vec())
    }

    /// Leg 6: consume the client's `TSRequest` carrying the sealed
    /// `authInfo` and return the delegated `(domain, user, password)`.
    pub fn finish(&mut self, client_msg: &[u8]) -> Result<(String, String, String)> {
        let request = TsRequest::decode(client_msg)?;
        let auth_info = request.auth_info.ok_or(Error::InvalidValue {
            field: "CredSSP client response",
            value: "missing authInfo".to_string(),
        })?;
        let context = self.context.as_mut().ok_or(Error::InvalidValue {
            field: "CredSSP state",
            value: "no security context".to_string(),
        })?;
        let credentials = context.decrypt_message(&auth_info)?;
        decode_ts_credentials(&credentials)
    }

    /// The negotiated CredSSP version (min of client and server).
    fn effective_version(&self) -> u32 {
        SERVER_VERSION.min(self.client_version)
    }

    /// The client's expected public-key binding value (before it was
    /// sealed).
    fn client_public_key_binding(&self, client_nonce: Option<&[u8]>) -> Vec<u8> {
        if self.effective_version() >= 5 {
            let mut nonce = [0u8; 32];
            if let Some(n) = client_nonce {
                let len = n.len().min(32);
                nonce[..len].copy_from_slice(&n[..len]);
            }
            hash_binding(CLIENT_TO_SERVER_MAGIC, &nonce, &self.public_key)
        } else {
            self.public_key.clone()
        }
    }

    /// This server's own public-key binding value (before sealing).
    fn server_public_key_binding(&self) -> Vec<u8> {
        if self.effective_version() >= 5 {
            let nonce = self.nonce.unwrap_or([0u8; 32]);
            hash_binding(SERVER_TO_CLIENT_MAGIC, &nonce, &self.public_key)
        } else {
            // Legacy: increment the first byte of the public key.
            let mut pk = self.public_key.clone();
            if let Some(first) = pk.first_mut() {
                *first = first.wrapping_add(1);
            }
            pk
        }
    }
}

// ---------------------------------------------------------------------------
// CredSSP over Kerberos
// ---------------------------------------------------------------------------

/// Drives CredSSP using a Kerberos ticket instead of NTLM.
///
/// The Kerberos GSS context is set up by the client's `AP-REQ` (carried in the
/// first message's SPNEGO token) and the server's `AP-REP`; the CredSSP public
/// key and credentials are then sealed with the shared session key using the
/// RFC 4121 Wrap tokens ([`crate::krb5::cfx`]). This is the same three-leg
/// shape as the NTLM path, one round shorter.
///
/// Obtaining the ticket and session key from a KDC is out of scope here — pass
/// an already-built `AP-REQ` and the matching AES session key (e.g. from a
/// credential cache). The nonce and per-message confounders are injected so
/// the exchange is testable; real callers pass OS randomness.
pub struct KerberosCredSspClient {
    session_key: crate::krb5::aes::AesKey,
    ap_req: Vec<u8>,
    public_key: Vec<u8>,
    nonce: [u8; 32],
    domain: String,
    user: String,
    password: String,
    seq: u64,
}

impl KerberosCredSspClient {
    /// Create a client from a Kerberos AP-REQ, its session key, and the
    /// server's TLS `SubjectPublicKeyInfo`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_key: crate::krb5::aes::AesKey,
        ap_req: Vec<u8>,
        public_key: Vec<u8>,
        nonce: [u8; 32],
        domain: &str,
        user: &str,
        password: &str,
    ) -> Self {
        KerberosCredSspClient {
            session_key,
            ap_req,
            public_key,
            nonce,
            domain: domain.to_string(),
            user: user.to_string(),
            password: password.to_string(),
            seq: 0,
        }
    }

    /// Leg 1: the `TSRequest` carrying the SPNEGO/AP-REQ token and the sealed
    /// public-key binding. `confounder` is 16 bytes of randomness.
    pub fn initial_request(&mut self, confounder: &[u8]) -> Vec<u8> {
        let spnego = crate::krb5::gss::spnego_init_kerberos(&self.ap_req);
        let binding = hash_binding(CLIENT_TO_SERVER_MAGIC, &self.nonce, &self.public_key);
        let pub_key_auth = crate::krb5::cfx::wrap(
            &self.session_key,
            crate::krb5::cfx::KG_USAGE_INITIATOR_SEAL,
            self.seq,
            false,
            true,
            &binding,
            confounder,
        );
        self.seq += 1;
        TsRequest {
            version: CLIENT_VERSION,
            nego_tokens: vec![spnego],
            pub_key_auth: Some(pub_key_auth),
            client_nonce: Some(self.nonce.to_vec()),
            ..Default::default()
        }
        .to_vec()
    }

    /// Leg 3: verify the server's `AP-REP` and sealed public-key confirmation,
    /// then return the `TSRequest` carrying the sealed credentials.
    pub fn finish(&mut self, server_msg: &[u8], confounder: &[u8]) -> Result<Vec<u8>> {
        let request = TsRequest::decode(server_msg)?;
        check_error(&request)?;

        // The server's SPNEGO reply should not be a rejection.
        if let Some(token) = request.nego_tokens.first() {
            let resp = crate::krb5::gss::NegTokenResp::decode(token)?;
            if resp.neg_state == Some(crate::krb5::gss::NEG_STATE_REJECT) {
                return Err(Error::InvalidValue {
                    field: "SPNEGO negState",
                    value: "reject".to_string(),
                });
            }
        }

        // Verify the server's sealed public-key binding.
        let pk = request.pub_key_auth.ok_or(Error::InvalidValue {
            field: "CredSSP server response",
            value: "missing pubKeyAuth".to_string(),
        })?;
        let received = crate::krb5::cfx::unwrap(
            &self.session_key,
            crate::krb5::cfx::KG_USAGE_ACCEPTOR_SEAL,
            &pk,
        )?;
        let expected = hash_binding(SERVER_TO_CLIENT_MAGIC, &self.nonce, &self.public_key);
        if received != expected {
            return Err(Error::InvalidValue {
                field: "CredSSP public key confirmation",
                value: "mismatch".to_string(),
            });
        }

        // Seal and delegate the credentials.
        let credentials = encode_ts_credentials(&self.domain, &self.user, &self.password);
        let auth_info = crate::krb5::cfx::wrap(
            &self.session_key,
            crate::krb5::cfx::KG_USAGE_INITIATOR_SEAL,
            self.seq,
            false,
            true,
            &credentials,
            confounder,
        );
        self.seq += 1;
        Ok(TsRequest {
            version: CLIENT_VERSION,
            auth_info: Some(auth_info),
            ..Default::default()
        }
        .to_vec())
    }
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
        NtlmContext::new_server(esk)
    }

    #[test]
    fn full_kerberos_credssp_exchange_against_mock_server() {
        // End-to-end CredSSP over Kerberos: the client and a mock acceptor
        // share the AES session key (as they would after AP-REQ/AP-REP), so
        // the acceptor can open the client's Wrap tokens and seal its own.
        use crate::krb5::aes::{AesKey, ETYPE_AES256_CTS_HMAC_SHA1_96};
        use crate::krb5::cfx;
        use crate::krb5::gss::{self, NegTokenResp, NEG_STATE_ACCEPT_COMPLETED};
        use crate::krb5::messages::{ApReq, Ticket, AP_OPT_MUTUAL_REQUIRED};
        use crate::krb5::{EncryptedData, PrincipalName};

        let public_key = vec![0x30, 0x82, 0x01, 0x0A, 0xCA, 0xFE, 0xBA, 0xBE];
        let nonce = [0x66u8; 32];

        // The GSS session key (in reality the AP-REQ authenticator subkey).
        let session_key =
            AesKey::from_password(ETYPE_AES256_CTS_HMAC_SHA1_96, "s3cr3t", b"EXAMPLE.COMalice")
                .unwrap();
        // A plausible AP-REQ (the mock never decrypts it).
        let princ = |t, parts: &[&str]| PrincipalName {
            name_type: t,
            name_string: parts.iter().map(|s| s.to_string()).collect(),
        };
        let ticket = Ticket {
            realm: "EXAMPLE.COM".to_string(),
            sname: princ(2, &["TERMSRV", "host.example.com"]),
            enc_part: EncryptedData {
                etype: 18,
                kvno: Some(2),
                cipher: vec![0xAB; 32],
            },
        };
        let authenticator = EncryptedData {
            etype: 18,
            kvno: None,
            cipher: vec![0xCD; 48],
        };
        let ap_req = ApReq {
            ap_options: AP_OPT_MUTUAL_REQUIRED,
            ticket,
            authenticator,
        }
        .encode();

        let mut client = KerberosCredSspClient::new(
            AesKey::from_key(ETYPE_AES256_CTS_HMAC_SHA1_96, session_key.key().to_vec()).unwrap(),
            ap_req.clone(),
            public_key.clone(),
            nonce,
            "EXAMPLE.COM",
            "alice",
            "s3cr3t",
        );

        // Leg 1: SPNEGO/AP-REQ + sealed client binding.
        let leg1 = client.initial_request(&[0x01u8; 16]);
        let req1 = TsRequest::decode(&leg1).unwrap();
        assert_eq!(req1.nego_tokens.len(), 1);
        assert!(req1.client_nonce.is_some());

        // The mock acceptor verifies the SPNEGO carries the Kerberos AP-REQ.
        let init = gss::NegTokenInit::decode(&req1.nego_tokens[0]).unwrap();
        assert_eq!(init.mech_types[0], gss::KRB5_OID);
        let (mech_oid, inner) =
            gss::unwrap_initial_context_token(init.mech_token.as_ref().unwrap()).unwrap();
        assert_eq!(mech_oid, gss::KRB5_OID);
        assert_eq!(&inner[..2], &[0x01, 0x00]);
        assert_eq!(&inner[2..], &ap_req[..]);

        // The acceptor opens the client's sealed public-key binding.
        let client_binding = cfx::unwrap(
            &session_key,
            cfx::KG_USAGE_INITIATOR_SEAL,
            req1.pub_key_auth.as_ref().unwrap(),
        )
        .unwrap();
        assert_eq!(
            client_binding,
            hash_binding(CLIENT_TO_SERVER_MAGIC, &nonce, &public_key)
        );

        // Leg 2: the acceptor replies with an AP-REP token and its own sealed
        // binding, keyed with the acceptor-seal usage.
        let server_binding = hash_binding(SERVER_TO_CLIENT_MAGIC, &nonce, &public_key);
        let server_pub_key_auth = cfx::wrap(
            &session_key,
            cfx::KG_USAGE_ACCEPTOR_SEAL,
            0,
            true,
            true,
            &server_binding,
            &[0x02u8; 16],
        );
        let ap_rep_token = NegTokenResp {
            neg_state: Some(NEG_STATE_ACCEPT_COMPLETED),
            supported_mech: Some(gss::KRB5_OID.to_vec()),
            response_token: Some(vec![0x02, 0x00, 0x11, 0x22]), // stand-in AP-REP
            mech_list_mic: None,
        }
        .to_vec();
        let leg2 = TsRequest {
            version: 6,
            nego_tokens: vec![ap_rep_token],
            pub_key_auth: Some(server_pub_key_auth),
            ..Default::default()
        }
        .to_vec();

        // Leg 3: the client verifies and delegates its credentials.
        let leg3 = client.finish(&leg2, &[0x03u8; 16]).unwrap();
        let req3 = TsRequest::decode(&leg3).unwrap();
        let auth_info = req3.auth_info.unwrap();
        let credentials =
            cfx::unwrap(&session_key, cfx::KG_USAGE_INITIATOR_SEAL, &auth_info).unwrap();
        assert_eq!(
            credentials,
            encode_ts_credentials("EXAMPLE.COM", "alice", "s3cr3t")
        );
    }

    fn credentials_store(domain: &str, user: &str) -> Option<[u8; 16]> {
        if domain == "CORP" && user == "alice" {
            Some(crate::ntlm::nt_hash("secret"))
        } else {
            None
        }
    }

    #[test]
    fn ts_credentials_roundtrip() {
        let encoded = encode_ts_credentials("CORP", "alice", "secret");
        let (domain, user, password) = decode_ts_credentials(&encoded).unwrap();
        assert_eq!(
            (domain.as_str(), user.as_str(), password.as_str()),
            ("CORP", "alice", "secret")
        );
    }

    #[test]
    fn full_credssp_exchange_against_real_server() {
        // The real (not mocked) CredSSP server: two independently-derived
        // NTLM contexts genuinely authenticate and delegate credentials.
        let public_key = vec![0x30, 0x82, 0x01, 0x0A, 0xDE, 0xAD, 0xBE, 0xEF];
        let nonce = [0x77u8; 32];
        let esk = [0x55u8; 16];
        let client_challenge = [0xAAu8; 8];

        let mut client = CredSspClient::new(
            "CORP",
            "alice",
            "secret",
            "WKS",
            public_key.clone(),
            nonce,
            client_challenge,
            [0u8; 8],
            esk,
        );
        let mut server = CredSspServer::new(
            "SRV",
            [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef],
            [0u8; 8],
            public_key,
            credentials_store,
        );

        let leg1 = client.negotiate_request();
        let leg2 = server.challenge_response(&leg1).unwrap();
        let leg3 = client.challenge_response(&leg2).unwrap();
        let leg4 = server.verify_authenticate(&leg3).unwrap();
        let leg5 = client.finish(&leg4).unwrap();
        let (domain, user, password) = server.finish(&leg5).unwrap();

        assert_eq!(domain, "CORP");
        assert_eq!(user, "alice");
        assert_eq!(password, "secret");
    }

    #[test]
    fn full_credssp_exchange_legacy_version_binding() {
        // Force a pre-5 negotiated version so both sides fall back to the
        // legacy "public key (+1)" binding instead of the nonce hash.
        let public_key = vec![0x30, 0x82, 0x01, 0x0A, 0xDE, 0xAD, 0xBE, 0xEF];
        let nonce = [0x77u8; 32];
        let esk = [0x55u8; 16];

        let mut client = CredSspClient::new(
            "CORP",
            "alice",
            "secret",
            "WKS",
            public_key.clone(),
            nonce,
            [0xAAu8; 8],
            [0u8; 8],
            esk,
        );
        let mut server = CredSspServer::new(
            "SRV",
            [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef],
            [0u8; 8],
            public_key,
            credentials_store,
        );

        let mut leg1 = TsRequest::decode(&client.negotiate_request()).unwrap();
        leg1.version = 4;
        let leg2 = server.challenge_response(&leg1.to_vec()).unwrap();
        assert_eq!(TsRequest::decode(&leg2).unwrap().version, 4);

        let leg3 = client.challenge_response(&leg2).unwrap();
        assert!(TsRequest::decode(&leg3).unwrap().client_nonce.is_none());

        let leg4 = server.verify_authenticate(&leg3).unwrap();
        let leg5 = client.finish(&leg4).unwrap();
        let (domain, user, password) = server.finish(&leg5).unwrap();
        assert_eq!(domain, "CORP");
        assert_eq!(user, "alice");
        assert_eq!(password, "secret");
    }

    #[test]
    fn real_server_rejects_wrong_password_and_client_sees_error() {
        let public_key = vec![0xAA; 8];
        let nonce = [0x11u8; 32];
        let esk = [0x22u8; 16];

        let mut client = CredSspClient::new(
            "CORP",
            "alice",
            "wrong-password",
            "WKS",
            public_key.clone(),
            nonce,
            [0x33u8; 8],
            [0u8; 8],
            esk,
        );
        let mut server =
            CredSspServer::new("SRV", [0x44u8; 8], [0u8; 8], public_key, credentials_store);

        let leg1 = client.negotiate_request();
        let leg2 = server.challenge_response(&leg1).unwrap();
        let leg3 = client.challenge_response(&leg2).unwrap();
        assert!(server.verify_authenticate(&leg3).is_err());

        // The server tells the client via an errorCode TSRequest instead.
        let error_response = encode_error_response(STATUS_LOGON_FAILURE);
        assert!(client.finish(&error_response).is_err());
    }
}
