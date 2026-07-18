//! NTLM authentication (MS-NLMP), std-only.
//!
//! This implements the NTLMv2 exchange used by RDP's Network Level
//! Authentication (NLA / CredSSP), both roles: the NEGOTIATE → CHALLENGE →
//! AUTHENTICATE message flow, the NTLMv2 response and key schedule, and the
//! NTLM "extended session security" sealing ([`NtlmContext`]) that CredSSP
//! uses to protect the public-key and credential tokens.
//!
//! [`NtlmClient`] drives the initiator side (used by
//! [`crate::credssp::CredSspClient`]). [`NtlmServer`] drives the acceptor
//! side (used by [`crate::credssp::CredSspServer`]): it verifies a client's
//! AUTHENTICATE message against a caller-supplied password-hash lookup
//! rather than owning any credential storage itself — see [`NtlmServer`]'s
//! docs for the callback's exact contract.
//!
//! Only NTLMv2 is implemented (no LM, no NTLMv1, no Kerberos). The crypto is
//! the hand-rolled MD4/MD5/HMAC-MD5/RC4 from [`crate::crypto`]; NTLM is
//! obsolete and weak, and this exists only to speak the wire protocol.
//!
//! ## Security warning
//!
//! NTLM is not secure against modern attacks. Prefer Kerberos where available.
//! This code is for interoperability, not protection.

use crate::crypto::hmac::hmac_md5;
use crate::crypto::md4::md4;
use crate::crypto::md5::Md5;
use crate::crypto::rc4::Rc4;
use crate::cursor::Reader;
use crate::error::{Error, Result};

/// The `"NTLMSSP\0"` message signature.
const SIGNATURE: [u8; 8] = *b"NTLMSSP\0";

// NegotiateFlags (MS-NLMP 2.2.2.5) used by this client.
const NTLMSSP_NEGOTIATE_UNICODE: u32 = 0x0000_0001;
const NTLMSSP_REQUEST_TARGET: u32 = 0x0000_0004;
const NTLMSSP_NEGOTIATE_SIGN: u32 = 0x0000_0010;
const NTLMSSP_NEGOTIATE_SEAL: u32 = 0x0000_0020;
const NTLMSSP_NEGOTIATE_NTLM: u32 = 0x0000_0200;
const NTLMSSP_NEGOTIATE_ALWAYS_SIGN: u32 = 0x0000_8000;
const NTLMSSP_NEGOTIATE_EXTENDED_SESSIONSECURITY: u32 = 0x0008_0000;
const NTLMSSP_TARGET_TYPE_SERVER: u32 = 0x0002_0000;
const NTLMSSP_NEGOTIATE_TARGET_INFO: u32 = 0x0080_0000;
const NTLMSSP_NEGOTIATE_VERSION: u32 = 0x0200_0000;
const NTLMSSP_NEGOTIATE_128: u32 = 0x2000_0000;
const NTLMSSP_NEGOTIATE_KEY_EXCH: u32 = 0x4000_0000;
const NTLMSSP_NEGOTIATE_56: u32 = 0x8000_0000;

/// The client NegotiateFlags: Unicode NTLMv2 with extended session security,
/// sealing, signing, and key exchange.
const CLIENT_FLAGS: u32 = NTLMSSP_NEGOTIATE_UNICODE
    | NTLMSSP_REQUEST_TARGET
    | NTLMSSP_NEGOTIATE_SIGN
    | NTLMSSP_NEGOTIATE_SEAL
    | NTLMSSP_NEGOTIATE_NTLM
    | NTLMSSP_NEGOTIATE_ALWAYS_SIGN
    | NTLMSSP_NEGOTIATE_EXTENDED_SESSIONSECURITY
    | NTLMSSP_NEGOTIATE_VERSION
    | NTLMSSP_NEGOTIATE_128
    | NTLMSSP_NEGOTIATE_KEY_EXCH
    | NTLMSSP_NEGOTIATE_56;

/// The server NegotiateFlags in [`NtlmServer`]'s CHALLENGE_MESSAGE: Unicode
/// NTLMv2 with extended session security, a `TargetInfo` block, and a
/// server-type target.
const SERVER_FLAGS: u32 = NTLMSSP_NEGOTIATE_UNICODE
    | NTLMSSP_REQUEST_TARGET
    | NTLMSSP_NEGOTIATE_NTLM
    | NTLMSSP_NEGOTIATE_ALWAYS_SIGN
    | NTLMSSP_TARGET_TYPE_SERVER
    | NTLMSSP_NEGOTIATE_EXTENDED_SESSIONSECURITY
    | NTLMSSP_NEGOTIATE_TARGET_INFO
    | NTLMSSP_NEGOTIATE_VERSION
    | NTLMSSP_NEGOTIATE_128
    | NTLMSSP_NEGOTIATE_KEY_EXCH
    | NTLMSSP_NEGOTIATE_56;

// AV_PAIR identifiers (MS-NLMP 2.2.2.1).
const MSV_AV_EOL: u16 = 0x0000;
const MSV_AV_NB_COMPUTER_NAME: u16 = 0x0001;
const MSV_AV_FLAGS: u16 = 0x0006;
const MSV_AV_TIMESTAMP: u16 = 0x0007;

/// A fixed Version field (`TS_UD` style) — Windows 6.1 build 7601.
const VERSION: [u8; 8] = [0x06, 0x01, 0xB1, 0x1D, 0x00, 0x00, 0x00, 0x0F];

/// UTF-16LE encode a string.
fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// UTF-16LE decode a byte string.
pub(crate) fn utf16le_decode(bytes: &[u8]) -> Result<String> {
    if bytes.len() % 2 != 0 {
        return Err(Error::InvalidLength {
            field: "UTF-16LE string",
            length: bytes.len(),
        });
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).map_err(|_| Error::InvalidValue {
        field: "UTF-16LE string",
        value: "invalid UTF-16".to_string(),
    })
}

/// The NT hash: `MD4(UTF16LE(password))`. Real deployments should store only
/// this value (never the plaintext password) and look it up by identity for
/// [`NtlmServer`]'s `hash_lookup` callback — this helper exists mainly for
/// tests and simple setups that still start from a plaintext password.
pub fn nt_hash(password: &str) -> [u8; 16] {
    md4(&utf16le(password))
}

/// `NTOWFv2 = HMAC_MD5(MD4(UTF16LE(password)), UTF16LE(Uppercase(user) + domain))`.
pub fn ntowf_v2(domain: &str, user: &str, password: &str) -> [u8; 16] {
    ntowf_v2_from_hash(&nt_hash(password), domain, user)
}

/// `NTOWFv2`, starting from an already-computed NT hash (see [`nt_hash`])
/// instead of a plaintext password — what [`NtlmServer`] uses, since a
/// server's credential store holds the hash, not the password.
pub fn ntowf_v2_from_hash(nt_hash: &[u8; 16], domain: &str, user: &str) -> [u8; 16] {
    let mut identity = utf16le(&user.to_uppercase());
    identity.extend_from_slice(&utf16le(domain));
    hmac_md5(nt_hash, &identity)
}

/// Parse the AV_PAIR list in `target_info` into `(id, value)` pairs, dropping
/// the terminating `MsvAvEOL`.
fn parse_av_pairs(target_info: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut r = Reader::new(target_info);
    let mut pairs = Vec::new();
    while let (Ok(id), Ok(len)) = (r.read_u16_le(), r.read_u16_le()) {
        if id == MSV_AV_EOL {
            break;
        }
        match r.read_bytes(len as usize) {
            Ok(value) => pairs.push((id, value.to_vec())),
            Err(_) => break,
        }
    }
    pairs
}

/// Re-serialize AV_PAIRs and append the `MsvAvEOL` terminator.
fn encode_av_pairs(pairs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (id, value) in pairs {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(value);
    }
    out.extend_from_slice(&MSV_AV_EOL.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// Return a copy of `target_info` with the `MsvAvFlags` "MIC present" bit
/// (0x2) set, inserting an `MsvAvFlags` pair before the EOL if absent.
fn with_mic_flag(target_info: &[u8]) -> Vec<u8> {
    let mut pairs = parse_av_pairs(target_info);
    if let Some((_, value)) = pairs.iter_mut().find(|(id, _)| *id == MSV_AV_FLAGS) {
        if value.len() >= 4 {
            let mut flags = u32::from_le_bytes([value[0], value[1], value[2], value[3]]);
            flags |= 0x0000_0002;
            value[..4].copy_from_slice(&flags.to_le_bytes());
        }
    } else {
        pairs.push((MSV_AV_FLAGS, 0x0000_0002u32.to_le_bytes().to_vec()));
    }
    encode_av_pairs(&pairs)
}

/// Extract the `MsvAvTimestamp` value (8 bytes) from a target-info blob.
fn timestamp_from(target_info: &[u8]) -> Option<[u8; 8]> {
    parse_av_pairs(target_info).into_iter().find_map(|(id, v)| {
        if id == MSV_AV_TIMESTAMP && v.len() == 8 {
            let mut ts = [0u8; 8];
            ts.copy_from_slice(&v);
            Some(ts)
        } else {
            None
        }
    })
}

/// The NTLMv2 response and its derived session base key.
struct Ntlmv2Response {
    /// `NtChallengeResponse = NTProofStr(16) + temp`.
    nt_response: Vec<u8>,
    /// `SessionBaseKey = HMAC_MD5(NTOWFv2, NTProofStr)`.
    session_base_key: [u8; 16],
}

/// Compute the NTLMv2 `NtChallengeResponse` and session base key
/// (MS-NLMP 3.3.2).
fn compute_ntlmv2(
    response_key: &[u8; 16],
    server_challenge: &[u8; 8],
    client_challenge: &[u8; 8],
    timestamp: &[u8; 8],
    target_info: &[u8],
) -> Ntlmv2Response {
    // temp = Responserversion(1) HiResponserversion(1) Z(6) Timestamp(8)
    //        ClientChallenge(8) Z(4) TargetInfo Z(4)
    let mut temp = Vec::new();
    temp.push(0x01);
    temp.push(0x01);
    temp.extend_from_slice(&[0u8; 6]);
    temp.extend_from_slice(timestamp);
    temp.extend_from_slice(client_challenge);
    temp.extend_from_slice(&[0u8; 4]);
    temp.extend_from_slice(target_info);
    temp.extend_from_slice(&[0u8; 4]);

    // NTProofStr = HMAC_MD5(response_key, ServerChallenge + temp)
    let mut proof_input = Vec::with_capacity(8 + temp.len());
    proof_input.extend_from_slice(server_challenge);
    proof_input.extend_from_slice(&temp);
    let nt_proof = hmac_md5(response_key, &proof_input);

    let mut nt_response = Vec::with_capacity(16 + temp.len());
    nt_response.extend_from_slice(&nt_proof);
    nt_response.extend_from_slice(&temp);

    let session_base_key = hmac_md5(response_key, &nt_proof);
    Ntlmv2Response {
        nt_response,
        session_base_key,
    }
}

/// Build an 8-byte MS-NLMP field descriptor (`Len`, `MaxLen`, `Offset`) for
/// `data` at the current `offset`, then advance `offset` past it.
fn field_descriptor(data: &[u8], offset: &mut u32) -> [u8; 8] {
    let len = data.len() as u16;
    let mut f = [0u8; 8];
    f[..2].copy_from_slice(&len.to_le_bytes());
    f[2..4].copy_from_slice(&len.to_le_bytes());
    f[4..8].copy_from_slice(&offset.to_le_bytes());
    *offset += data.len() as u32;
    f
}

/// `MD5(key + magic)` — the NTLM signing/sealing key derivation.
fn derive_key(exported_session_key: &[u8; 16], magic: &[u8]) -> [u8; 16] {
    let mut h = Md5::new();
    h.update(exported_session_key);
    h.update(magic);
    h.finish()
}

const CLIENT_SIGN_MAGIC: &[u8] = b"session key to client-to-server signing key magic constant\0";
const SERVER_SIGN_MAGIC: &[u8] = b"session key to server-to-client signing key magic constant\0";
const CLIENT_SEAL_MAGIC: &[u8] = b"session key to client-to-server sealing key magic constant\0";
const SERVER_SEAL_MAGIC: &[u8] = b"session key to server-to-client sealing key magic constant\0";

/// An established NTLM security context: the signing keys, sealing RC4 states,
/// and per-direction sequence numbers used to protect CredSSP tokens
/// (MS-NLMP 3.4).
pub struct NtlmContext {
    client_signing_key: [u8; 16],
    server_signing_key: [u8; 16],
    client_seal: Rc4,
    server_seal: Rc4,
    client_seq: u32,
    server_seq: u32,
}

impl NtlmContext {
    /// Build a context from the *client's* (initiator's) point of view:
    /// [`NtlmContext::encrypt_message`] sends using the client-to-server
    /// keys and [`NtlmContext::decrypt_message`] expects the server-to-client
    /// keys. Servers must use [`NtlmContext::new_server`] instead.
    pub fn new(exported_session_key: &[u8; 16]) -> Self {
        NtlmContext {
            client_signing_key: derive_key(exported_session_key, CLIENT_SIGN_MAGIC),
            server_signing_key: derive_key(exported_session_key, SERVER_SIGN_MAGIC),
            client_seal: Rc4::new(&derive_key(exported_session_key, CLIENT_SEAL_MAGIC)),
            server_seal: Rc4::new(&derive_key(exported_session_key, SERVER_SEAL_MAGIC)),
            client_seq: 0,
            server_seq: 0,
        }
    }

    /// Seal and sign `message` for sending to the server. Returns the 16-byte
    /// signature followed by the sealed message (MS-NLMP 3.4.1 / 3.4.4.2 with
    /// extended session security and key exchange).
    pub fn encrypt_message(&mut self, message: &[u8]) -> Vec<u8> {
        // SEAL: RC4 the message first, consuming the keystream.
        let sealed = self.client_seal.applied(message);

        // MAC checksum is HMAC over the *plaintext*, then RC4-encrypted with
        // the same handle (key exchange is negotiated).
        let mut mac_input = Vec::with_capacity(4 + message.len());
        mac_input.extend_from_slice(&self.client_seq.to_le_bytes());
        mac_input.extend_from_slice(message);
        let full = hmac_md5(&self.client_signing_key, &mac_input);
        let checksum = self.client_seal.applied(&full[..8]);

        let mut out = Vec::with_capacity(16 + sealed.len());
        out.extend_from_slice(&1u32.to_le_bytes()); // Version
        out.extend_from_slice(&checksum);
        out.extend_from_slice(&self.client_seq.to_le_bytes());
        out.extend_from_slice(&sealed);
        self.client_seq = self.client_seq.wrapping_add(1);
        out
    }

    /// Verify and unseal a server message (16-byte signature + sealed data).
    pub fn decrypt_message(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 16 {
            return Err(Error::InvalidLength {
                field: "NTLM message signature",
                length: data.len(),
            });
        }
        let signature = &data[..16];
        let sealed = &data[16..];

        let plaintext = self.server_seal.applied(sealed);
        let mut mac_input = Vec::with_capacity(4 + plaintext.len());
        mac_input.extend_from_slice(&self.server_seq.to_le_bytes());
        mac_input.extend_from_slice(&plaintext);
        let full = hmac_md5(&self.server_signing_key, &mac_input);
        let checksum = self.server_seal.applied(&full[..8]);

        let mut expected = Vec::with_capacity(16);
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(&checksum);
        expected.extend_from_slice(&self.server_seq.to_le_bytes());
        if expected != signature {
            return Err(Error::InvalidValue {
                field: "NTLM message signature",
                value: "verification failed".to_string(),
            });
        }
        self.server_seq = self.server_seq.wrapping_add(1);
        Ok(plaintext)
    }

    /// Build a context from the *server's* (acceptor's) point of view: the
    /// send/receive directions are swapped relative to [`NtlmContext::new`],
    /// so a server built this way can exchange sealed tokens with a client's
    /// context derived from the same `exported_session_key` (its
    /// [`NtlmContext::encrypt_message`] sends what the client's
    /// `decrypt_message` expects, and vice versa) — the same client/server
    /// role split as [`crate::security::Rc4Session::new`] /
    /// [`crate::security::Rc4Session::new_server`].
    pub fn new_server(exported_session_key: &[u8; 16]) -> Self {
        NtlmContext {
            client_signing_key: derive_key(exported_session_key, SERVER_SIGN_MAGIC),
            server_signing_key: derive_key(exported_session_key, CLIENT_SIGN_MAGIC),
            client_seal: Rc4::new(&derive_key(exported_session_key, SERVER_SEAL_MAGIC)),
            server_seal: Rc4::new(&derive_key(exported_session_key, CLIENT_SEAL_MAGIC)),
            client_seq: 0,
            server_seq: 0,
        }
    }
}

/// A stateful NTLMv2 client that produces the NEGOTIATE and AUTHENTICATE
/// messages and the resulting [`NtlmContext`].
///
/// The nondeterministic inputs (client challenge, timestamp fallback, and the
/// exported session key) are supplied to [`NtlmClient::new`] so the exchange
/// is fully testable; callers that want real entropy should pass random bytes.
pub struct NtlmClient {
    domain: String,
    user: String,
    password: String,
    workstation: String,
    client_challenge: [u8; 8],
    timestamp: [u8; 8],
    exported_session_key: [u8; 16],
    negotiate_message: Vec<u8>,
}

impl NtlmClient {
    /// Create a client for the given credentials and nondeterministic inputs.
    pub fn new(
        domain: &str,
        user: &str,
        password: &str,
        workstation: &str,
        client_challenge: [u8; 8],
        timestamp: [u8; 8],
        exported_session_key: [u8; 16],
    ) -> Self {
        NtlmClient {
            domain: domain.to_string(),
            user: user.to_string(),
            password: password.to_string(),
            workstation: workstation.to_string(),
            client_challenge,
            timestamp,
            exported_session_key,
            negotiate_message: Vec::new(),
        }
    }

    /// Build the NEGOTIATE_MESSAGE (type 1) and remember it for the MIC.
    pub fn negotiate(&mut self) -> Vec<u8> {
        let mut m = Vec::new();
        m.extend_from_slice(&SIGNATURE);
        m.extend_from_slice(&1u32.to_le_bytes()); // MessageType
        m.extend_from_slice(&CLIENT_FLAGS.to_le_bytes());
        // DomainName and Workstation fields empty (len 0, offset past header).
        m.extend_from_slice(&[0u8; 8]); // DomainNameFields
        m.extend_from_slice(&[0u8; 8]); // WorkstationFields
        m.extend_from_slice(&VERSION);
        self.negotiate_message = m.clone();
        m
    }

    /// Consume the server's CHALLENGE_MESSAGE and build the AUTHENTICATE
    /// message, returning it together with the established [`NtlmContext`].
    pub fn authenticate(&mut self, challenge: &[u8]) -> Result<(Vec<u8>, NtlmContext)> {
        let (server_challenge, target_info) = parse_challenge(challenge)?;

        // Timestamp: prefer the server's MsvAvTimestamp, else our fallback.
        let timestamp = timestamp_from(&target_info).unwrap_or(self.timestamp);
        // Target info echoed in the response gets the MIC-present flag.
        let response_target_info = with_mic_flag(&target_info);

        let response_key = ntowf_v2(&self.domain, &self.user, &self.password);
        let v2 = compute_ntlmv2(
            &response_key,
            &server_challenge,
            &self.client_challenge,
            &timestamp,
            &response_target_info,
        );

        // NTLMv2 key-exchange key is the session base key. With KEY_EXCH the
        // real session key is random and shipped RC4-wrapped.
        let mut rc4 = Rc4::new(&v2.session_base_key);
        let encrypted_random_session_key = rc4.applied(&self.exported_session_key);

        let auth =
            self.build_authenticate(&v2.nt_response, &encrypted_random_session_key, challenge);
        let context = NtlmContext::new(&self.exported_session_key);
        Ok((auth, context))
    }

    /// Assemble the AUTHENTICATE_MESSAGE (type 3), including the MIC.
    fn build_authenticate(
        &self,
        nt_response: &[u8],
        encrypted_session_key: &[u8],
        challenge: &[u8],
    ) -> Vec<u8> {
        let domain = utf16le(&self.domain);
        let user = utf16le(&self.user);
        let workstation = utf16le(&self.workstation);
        // LmChallengeResponse is Z(24) for NTLMv2 with target info.
        let lm_response = [0u8; 24];

        // Fixed header is 88 bytes (through the 16-byte MIC); payload follows.
        const HEADER_LEN: usize = 88;
        let mut offset = HEADER_LEN as u32;
        let lm_field = field_descriptor(&lm_response, &mut offset);
        let nt_field = field_descriptor(nt_response, &mut offset);
        let domain_field = field_descriptor(&domain, &mut offset);
        let user_field = field_descriptor(&user, &mut offset);
        let workstation_field = field_descriptor(&workstation, &mut offset);
        let session_key_field = field_descriptor(encrypted_session_key, &mut offset);

        let mut m = Vec::new();
        m.extend_from_slice(&SIGNATURE);
        m.extend_from_slice(&3u32.to_le_bytes()); // MessageType
        m.extend_from_slice(&lm_field);
        m.extend_from_slice(&nt_field);
        m.extend_from_slice(&domain_field);
        m.extend_from_slice(&user_field);
        m.extend_from_slice(&workstation_field);
        m.extend_from_slice(&session_key_field);
        m.extend_from_slice(&CLIENT_FLAGS.to_le_bytes());
        m.extend_from_slice(&VERSION);
        let mic_offset = m.len();
        m.extend_from_slice(&[0u8; 16]); // MIC placeholder
        debug_assert_eq!(m.len(), HEADER_LEN);
        // Payload, in field order.
        m.extend_from_slice(&lm_response);
        m.extend_from_slice(nt_response);
        m.extend_from_slice(&domain);
        m.extend_from_slice(&user);
        m.extend_from_slice(&workstation);
        m.extend_from_slice(encrypted_session_key);

        // MIC = HMAC_MD5(ExportedSessionKey, Negotiate + Challenge + Authenticate)
        // with the MIC field zeroed during computation.
        let mut mic_input = Vec::new();
        mic_input.extend_from_slice(&self.negotiate_message);
        mic_input.extend_from_slice(challenge);
        mic_input.extend_from_slice(&m);
        let mic = hmac_md5(&self.exported_session_key, &mic_input);
        m[mic_offset..mic_offset + 16].copy_from_slice(&mic);
        m
    }
}

/// Parse a CHALLENGE_MESSAGE, returning `(server_challenge, target_info)`.
fn parse_challenge(challenge: &[u8]) -> Result<([u8; 8], Vec<u8>)> {
    if challenge.len() < 48 || challenge[..8] != SIGNATURE {
        return Err(Error::InvalidValue {
            field: "NTLM CHALLENGE signature",
            value: "missing or short".to_string(),
        });
    }
    let msg_type = u32::from_le_bytes([challenge[8], challenge[9], challenge[10], challenge[11]]);
    if msg_type != 2 {
        return Err(Error::InvalidValue {
            field: "NTLM message type",
            value: msg_type.to_string(),
        });
    }
    let mut server_challenge = [0u8; 8];
    server_challenge.copy_from_slice(&challenge[24..32]);

    // TargetInfoFields at offset 40: len(2), maxlen(2), offset(4).
    let ti_len = u16::from_le_bytes([challenge[40], challenge[41]]) as usize;
    let ti_offset =
        u32::from_le_bytes([challenge[44], challenge[45], challenge[46], challenge[47]]) as usize;
    let target_info = if ti_len == 0 {
        Vec::new()
    } else {
        challenge
            .get(ti_offset..ti_offset + ti_len)
            .ok_or(Error::InvalidLength {
                field: "NTLM target info",
                length: ti_len,
            })?
            .to_vec()
    };
    Ok((server_challenge, target_info))
}

/// Generic authentication-failure error: returned for an unknown identity, a
/// wrong password, and a MIC mismatch alike, so a caller can never
/// distinguish these from the error alone.
fn auth_failure() -> Error {
    Error::InvalidValue {
        field: "NTLM AUTHENTICATE",
        value: "authentication failed".to_string(),
    }
}

/// Sanity-check a NEGOTIATE_MESSAGE's signature and message type.
fn parse_negotiate(negotiate: &[u8]) -> Result<()> {
    if negotiate.len() < 12 || negotiate[..8] != SIGNATURE {
        return Err(Error::InvalidValue {
            field: "NTLM NEGOTIATE signature",
            value: "missing or short".to_string(),
        });
    }
    let msg_type = u32::from_le_bytes([negotiate[8], negotiate[9], negotiate[10], negotiate[11]]);
    if msg_type != 1 {
        return Err(Error::InvalidValue {
            field: "NTLM message type",
            value: msg_type.to_string(),
        });
    }
    Ok(())
}

/// The parts of an AUTHENTICATE_MESSAGE this crate's server needs.
struct ParsedAuthenticate {
    domain: String,
    user: String,
    nt_challenge_response: Vec<u8>,
    encrypted_session_key: Vec<u8>,
    mic: [u8; 16],
}

/// Parse an AUTHENTICATE_MESSAGE (MS-NLMP 2.2.1.3).
fn parse_authenticate(buf: &[u8]) -> Result<ParsedAuthenticate> {
    if buf.len() < 88 || buf[..8] != SIGNATURE {
        return Err(Error::InvalidValue {
            field: "NTLM AUTHENTICATE signature",
            value: "missing or short".to_string(),
        });
    }
    let msg_type = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    if msg_type != 3 {
        return Err(Error::InvalidValue {
            field: "NTLM message type",
            value: msg_type.to_string(),
        });
    }
    // Each *Fields descriptor is Len(2) MaxLen(2) Offset(4).
    let field = |header_offset: usize| -> (usize, usize) {
        let len = u16::from_le_bytes([buf[header_offset], buf[header_offset + 1]]) as usize;
        let offset = u32::from_le_bytes([
            buf[header_offset + 4],
            buf[header_offset + 5],
            buf[header_offset + 6],
            buf[header_offset + 7],
        ]) as usize;
        (offset, len)
    };
    let get = |offset: usize, len: usize, name: &'static str| -> Result<&[u8]> {
        buf.get(offset..offset + len).ok_or(Error::InvalidLength {
            field: name,
            length: len,
        })
    };

    let (nt_off, nt_len) = field(20);
    let (dom_off, dom_len) = field(28);
    let (usr_off, usr_len) = field(36);
    let (key_off, key_len) = field(52);

    let nt_challenge_response = get(nt_off, nt_len, "NtChallengeResponse")?.to_vec();
    let domain = utf16le_decode(get(dom_off, dom_len, "AUTHENTICATE DomainName")?)?;
    let user = utf16le_decode(get(usr_off, usr_len, "AUTHENTICATE UserName")?)?;
    let encrypted_session_key = get(key_off, key_len, "EncryptedRandomSessionKey")?.to_vec();
    let mut mic = [0u8; 16];
    mic.copy_from_slice(&buf[72..88]);

    Ok(ParsedAuthenticate {
        domain,
        user,
        nt_challenge_response,
        encrypted_session_key,
        mic,
    })
}

/// The client-supplied parts of `NtChallengeResponse`'s `temp` blob
/// (MS-NLMP 2.2.2.7 `NTLMv2_CLIENT_CHALLENGE`) needed to recompute
/// `NTProofStr`.
struct ClientChallengeInfo {
    timestamp: [u8; 8],
    client_challenge: [u8; 8],
    target_info: Vec<u8>,
}

/// Split `NtChallengeResponse` into `(NTProofStr, ClientChallengeInfo)`.
fn parse_nt_challenge_response(
    nt_challenge_response: &[u8],
) -> Result<([u8; 16], ClientChallengeInfo)> {
    // NTProofStr(16) + temp, where temp is at least RespVer(1) HiRespVer(1)
    // Z(6) Timestamp(8) ClientChallenge(8) Z(4) TargetInfo(>=0) Z(4).
    const MIN_LEN: usize = 16 + 28 + 4;
    if nt_challenge_response.len() < MIN_LEN {
        return Err(Error::InvalidLength {
            field: "NtChallengeResponse",
            length: nt_challenge_response.len(),
        });
    }
    let mut nt_proof = [0u8; 16];
    nt_proof.copy_from_slice(&nt_challenge_response[..16]);
    let temp = &nt_challenge_response[16..];

    let mut timestamp = [0u8; 8];
    timestamp.copy_from_slice(&temp[8..16]);
    let mut client_challenge = [0u8; 8];
    client_challenge.copy_from_slice(&temp[16..24]);
    let target_info = temp[28..temp.len() - 4].to_vec();

    Ok((
        nt_proof,
        ClientChallengeInfo {
            timestamp,
            client_challenge,
            target_info,
        },
    ))
}

/// A stateful NTLMv2 server (acceptor) that builds the CHALLENGE_MESSAGE and
/// verifies the client's AUTHENTICATE_MESSAGE against a caller-supplied
/// password-hash lookup, producing the resulting [`NtlmContext`].
///
/// `hash_lookup(domain, user)` should return the account's NT hash (see
/// [`nt_hash`]) if the identity is known, or `None` otherwise. This crate
/// never stores or looks up credentials itself — real deployments should
/// keep only the NT hash (never the plaintext password) in whatever
/// directory or credential store they already have, and look it up here.
///
/// [`NtlmServer::authenticate`] returns the same generic error for an
/// unknown identity, a wrong password, and a MIC mismatch, so a caller can
/// never distinguish these from the error alone (and should not try to —
/// report them identically to the peer too, to avoid a username oracle).
pub struct NtlmServer<F: Fn(&str, &str) -> Option<[u8; 16]>> {
    hash_lookup: F,
    target_name: String,
    server_challenge: [u8; 8],
    timestamp: [u8; 8],
    negotiate_message: Vec<u8>,
    challenge_message: Vec<u8>,
}

impl<F: Fn(&str, &str) -> Option<[u8; 16]>> NtlmServer<F> {
    /// Create a server. `target_name` is the NetBIOS server name advertised
    /// in the CHALLENGE_MESSAGE's `TargetName` and `MsvAvNbComputerName`. The
    /// nondeterministic inputs (`server_challenge`, `timestamp`) are supplied
    /// by the caller so the exchange is testable; production callers should
    /// pass random bytes and the current time (as a Windows FILETIME).
    pub fn new(
        target_name: &str,
        server_challenge: [u8; 8],
        timestamp: [u8; 8],
        hash_lookup: F,
    ) -> Self {
        NtlmServer {
            hash_lookup,
            target_name: target_name.to_string(),
            server_challenge,
            timestamp,
            negotiate_message: Vec::new(),
            challenge_message: Vec::new(),
        }
    }

    /// Consume the client's NEGOTIATE_MESSAGE and build the
    /// CHALLENGE_MESSAGE, remembering both for the AUTHENTICATE's MIC check.
    pub fn challenge(&mut self, negotiate: &[u8]) -> Result<Vec<u8>> {
        parse_negotiate(negotiate)?;
        self.negotiate_message = negotiate.to_vec();

        let target_name = utf16le(&self.target_name);
        let target_info = encode_av_pairs(&[
            (MSV_AV_NB_COMPUTER_NAME, target_name.clone()),
            (MSV_AV_TIMESTAMP, self.timestamp.to_vec()),
        ]);

        // Header: Signature(8) Type(4) TargetNameFields(8) NegotiateFlags(4)
        // ServerChallenge(8) Reserved(8) TargetInfoFields(8) Version(8) = 56.
        const HEADER_LEN: u32 = 56;
        let mut offset = HEADER_LEN;
        let target_name_field = field_descriptor(&target_name, &mut offset);
        let target_info_field = field_descriptor(&target_info, &mut offset);

        let mut m = Vec::new();
        m.extend_from_slice(&SIGNATURE);
        m.extend_from_slice(&2u32.to_le_bytes()); // MessageType
        m.extend_from_slice(&target_name_field);
        m.extend_from_slice(&SERVER_FLAGS.to_le_bytes());
        m.extend_from_slice(&self.server_challenge);
        m.extend_from_slice(&[0u8; 8]); // Reserved
        m.extend_from_slice(&target_info_field);
        m.extend_from_slice(&VERSION);
        debug_assert_eq!(m.len(), HEADER_LEN as usize);
        m.extend_from_slice(&target_name);
        m.extend_from_slice(&target_info);

        self.challenge_message = m.clone();
        Ok(m)
    }

    /// Consume the client's AUTHENTICATE_MESSAGE: verify `NTProofStr`
    /// against the hash lookup and the MIC against the full message
    /// transcript, then return `(domain, user, NtlmContext)`.
    pub fn authenticate(&mut self, authenticate: &[u8]) -> Result<(String, String, NtlmContext)> {
        let parsed = parse_authenticate(authenticate)?;
        let (nt_proof, info) = parse_nt_challenge_response(&parsed.nt_challenge_response)?;

        let hash = (self.hash_lookup)(&parsed.domain, &parsed.user).ok_or_else(auth_failure)?;
        let response_key = ntowf_v2_from_hash(&hash, &parsed.domain, &parsed.user);
        let v2 = compute_ntlmv2(
            &response_key,
            &self.server_challenge,
            &info.client_challenge,
            &info.timestamp,
            &info.target_info,
        );
        if v2.nt_response[..16] != nt_proof {
            return Err(auth_failure());
        }

        let mut rc4 = Rc4::new(&v2.session_base_key);
        let exported_session_key_vec = rc4.applied(&parsed.encrypted_session_key);
        let exported_session_key: [u8; 16] = exported_session_key_vec
            .as_slice()
            .try_into()
            .map_err(|_| auth_failure())?;

        // MIC = HMAC_MD5(ExportedSessionKey, Negotiate + Challenge +
        // Authenticate-with-MIC-zeroed).
        let mut mic_input = Vec::new();
        mic_input.extend_from_slice(&self.negotiate_message);
        mic_input.extend_from_slice(&self.challenge_message);
        let mut zeroed = authenticate.to_vec();
        zeroed[72..88].copy_from_slice(&[0u8; 16]);
        mic_input.extend_from_slice(&zeroed);
        let expected_mic = hmac_md5(&exported_session_key, &mic_input);
        if expected_mic != parsed.mic {
            return Err(auth_failure());
        }

        let context = NtlmContext::new_server(&exported_session_key);
        Ok((parsed.domain, parsed.user, context))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    // MS-NLMP 4.2.4 test parameters.
    const SERVER_CHALLENGE: [u8; 8] = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
    const CLIENT_CHALLENGE: [u8; 8] = [0xaa; 8];

    fn sample_target_info() -> Vec<u8> {
        // MsvAvNbDomainName "Domain", MsvAvNbComputerName "Server", EOL.
        let mut ti = Vec::new();
        let domain = utf16le("Domain");
        let server = utf16le("Server");
        ti.extend_from_slice(&2u16.to_le_bytes());
        ti.extend_from_slice(&(domain.len() as u16).to_le_bytes());
        ti.extend_from_slice(&domain);
        ti.extend_from_slice(&1u16.to_le_bytes());
        ti.extend_from_slice(&(server.len() as u16).to_le_bytes());
        ti.extend_from_slice(&server);
        ti.extend_from_slice(&[0, 0, 0, 0]);
        ti
    }

    #[test]
    fn ntowf_v2_matches_spec() {
        // MS-NLMP 4.2.4.1.1.
        let key = ntowf_v2("Domain", "User", "Password");
        assert_eq!(hex(&key), "0c868a403bfd7a93a3001ef22ef02e3f");
    }

    #[test]
    fn ntlmv2_proof_and_base_key_match_spec() {
        // MS-NLMP 4.2.4.2.2: with timestamp 0 the NTProofStr and SessionBaseKey
        // are the documented values.
        let key = ntowf_v2("Domain", "User", "Password");
        let v2 = compute_ntlmv2(
            &key,
            &SERVER_CHALLENGE,
            &CLIENT_CHALLENGE,
            &[0u8; 8],
            &sample_target_info(),
        );
        // NTProofStr is the first 16 bytes of the NT response.
        assert_eq!(
            hex(&v2.nt_response[..16]),
            "68cd0ab851e51c96aabc927bebef6a1c"
        );
        assert_eq!(
            hex(&v2.session_base_key),
            "8de40ccadbc14a82f15cb0ad0de95ca3"
        );
    }

    #[test]
    fn context_encrypt_decrypt_roundtrip() {
        // A client context and a real server context derived from the same
        // exported session key agree on the sealed token, in both directions.
        let esk = [0x55u8; 16];
        let mut client = NtlmContext::new(&esk);
        let mut server = NtlmContext::new_server(&esk);

        let sealed = client.encrypt_message(b"public-key-bytes");
        let recovered = server.decrypt_message(&sealed).unwrap();
        assert_eq!(recovered, b"public-key-bytes");

        let sealed = server.encrypt_message(b"server-confirmation");
        let recovered = client.decrypt_message(&sealed).unwrap();
        assert_eq!(recovered, b"server-confirmation");
    }

    #[test]
    fn new_vs_new_server_actually_differ() {
        // Guards against the exact bug PR #21 caught in Rc4Session: two
        // client-role contexts must not be able to talk to each other.
        let esk = [0x42u8; 16];
        let mut a = NtlmContext::new(&esk);
        let mut b = NtlmContext::new(&esk);
        let sealed = a.encrypt_message(b"hello");
        assert!(b.decrypt_message(&sealed).is_err());
    }

    #[test]
    fn full_client_exchange_shapes() {
        // Build a CHALLENGE the way a server would and run the client through
        // negotiate → authenticate, checking the message framing.
        let mut challenge = Vec::new();
        let ti = sample_target_info();
        challenge.extend_from_slice(&SIGNATURE);
        challenge.extend_from_slice(&2u32.to_le_bytes());
        challenge.extend_from_slice(&[0u8; 8]); // TargetNameFields
        challenge.extend_from_slice(&CLIENT_FLAGS.to_le_bytes());
        challenge.extend_from_slice(&SERVER_CHALLENGE);
        challenge.extend_from_slice(&[0u8; 8]); // Reserved
        let ti_offset = 48u32 + 8; // header + version
        challenge.extend_from_slice(&(ti.len() as u16).to_le_bytes());
        challenge.extend_from_slice(&(ti.len() as u16).to_le_bytes());
        challenge.extend_from_slice(&ti_offset.to_le_bytes());
        challenge.extend_from_slice(&VERSION);
        challenge.extend_from_slice(&ti);

        let mut client = NtlmClient::new(
            "Domain",
            "User",
            "Password",
            "COMPUTER",
            CLIENT_CHALLENGE,
            [0u8; 8],
            [0x55u8; 16],
        );
        let neg = client.negotiate();
        assert_eq!(&neg[..8], &SIGNATURE);
        assert_eq!(u32::from_le_bytes([neg[8], neg[9], neg[10], neg[11]]), 1);

        let (auth, _ctx) = client.authenticate(&challenge).unwrap();
        assert_eq!(&auth[..8], &SIGNATURE);
        assert_eq!(
            u32::from_le_bytes([auth[8], auth[9], auth[10], auth[11]]),
            3
        );
        // NtChallengeResponse is present and longer than the 16-byte proof.
        let nt_len = u16::from_le_bytes([auth[20], auth[21]]) as usize;
        assert!(nt_len > 16);
        // The MIC field (offset 72) is non-zero.
        assert_ne!(&auth[72..88], &[0u8; 16]);
    }

    fn credentials_store(domain: &str, user: &str) -> Option<[u8; 16]> {
        if domain == "CORP" && user == "alice" {
            Some(nt_hash("secret"))
        } else {
            None
        }
    }

    #[test]
    fn full_client_server_exchange_authenticates_and_derives_matching_context() {
        let mut client = NtlmClient::new(
            "CORP",
            "alice",
            "secret",
            "WKS",
            CLIENT_CHALLENGE,
            [0u8; 8],
            [0x99u8; 16],
        );
        let mut server = NtlmServer::new("SRV", SERVER_CHALLENGE, [0u8; 8], credentials_store);

        let negotiate = client.negotiate();
        let challenge = server.challenge(&negotiate).unwrap();
        let (authenticate, mut client_ctx) = client.authenticate(&challenge).unwrap();
        let (domain, user, mut server_ctx) = server.authenticate(&authenticate).unwrap();

        assert_eq!(domain, "CORP");
        assert_eq!(user, "alice");

        // The two independently-derived contexts can talk in both directions
        // — the same real (not mocked) two-role exchange pattern that caught
        // the Rc4Session client/server role-swap bug in PR #21.
        let sealed = client_ctx.encrypt_message(b"public-key-bytes");
        assert_eq!(
            server_ctx.decrypt_message(&sealed).unwrap(),
            b"public-key-bytes"
        );
        let sealed = server_ctx.encrypt_message(b"server-confirmation");
        assert_eq!(
            client_ctx.decrypt_message(&sealed).unwrap(),
            b"server-confirmation"
        );
    }

    #[test]
    fn server_rejects_wrong_password() {
        let mut client = NtlmClient::new(
            "CORP",
            "alice",
            "wrong-password",
            "WKS",
            CLIENT_CHALLENGE,
            [0u8; 8],
            [0x99u8; 16],
        );
        let mut server = NtlmServer::new("SRV", SERVER_CHALLENGE, [0u8; 8], credentials_store);

        let negotiate = client.negotiate();
        let challenge = server.challenge(&negotiate).unwrap();
        let (authenticate, _ctx) = client.authenticate(&challenge).unwrap();
        assert!(server.authenticate(&authenticate).is_err());
    }

    #[test]
    fn server_rejects_unknown_user() {
        let mut client = NtlmClient::new(
            "CORP",
            "bob",
            "secret",
            "WKS",
            CLIENT_CHALLENGE,
            [0u8; 8],
            [0x99u8; 16],
        );
        let mut server = NtlmServer::new("SRV", SERVER_CHALLENGE, [0u8; 8], credentials_store);

        let negotiate = client.negotiate();
        let challenge = server.challenge(&negotiate).unwrap();
        let (authenticate, _ctx) = client.authenticate(&challenge).unwrap();
        assert!(server.authenticate(&authenticate).is_err());
    }

    #[test]
    fn server_rejects_tampered_mic() {
        let mut client = NtlmClient::new(
            "CORP",
            "alice",
            "secret",
            "WKS",
            CLIENT_CHALLENGE,
            [0u8; 8],
            [0x99u8; 16],
        );
        let mut server = NtlmServer::new("SRV", SERVER_CHALLENGE, [0u8; 8], credentials_store);

        let negotiate = client.negotiate();
        let challenge = server.challenge(&negotiate).unwrap();
        let (mut authenticate, _ctx) = client.authenticate(&challenge).unwrap();
        // Flip a bit in the MIC field (offset 72..88).
        authenticate[72] ^= 0xFF;
        assert!(server.authenticate(&authenticate).is_err());
    }

    #[test]
    fn challenge_rejects_non_negotiate_message() {
        let mut server = NtlmServer::new("SRV", SERVER_CHALLENGE, [0u8; 8], credentials_store);
        assert!(server.challenge(&[0u8; 4]).is_err());
    }

    #[test]
    fn nt_hash_matches_manual_md4() {
        assert_eq!(nt_hash("Password"), md4(&utf16le("Password")));
    }
}
