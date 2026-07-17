//! NTLM authentication (MS-NLMP), std-only.
//!
//! This implements the NTLMv2 client exchange used by RDP's Network Level
//! Authentication (NLA / CredSSP): the NEGOTIATE → CHALLENGE → AUTHENTICATE
//! message flow, the NTLMv2 response and key schedule, and the NTLM "extended
//! session security" sealing ([`NtlmContext`]) that CredSSP uses to protect
//! the public-key and credential tokens.
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

// AV_PAIR identifiers (MS-NLMP 2.2.2.1).
const MSV_AV_EOL: u16 = 0x0000;
const MSV_AV_FLAGS: u16 = 0x0006;
const MSV_AV_TIMESTAMP: u16 = 0x0007;

/// A fixed Version field (`TS_UD` style) — Windows 6.1 build 7601.
const VERSION: [u8; 8] = [0x06, 0x01, 0xB1, 0x1D, 0x00, 0x00, 0x00, 0x0F];

/// UTF-16LE encode a string.
fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// `NTOWFv2 = HMAC_MD5(MD4(UTF16LE(password)), UTF16LE(Uppercase(user) + domain))`.
pub fn ntowf_v2(domain: &str, user: &str, password: &str) -> [u8; 16] {
    let nt_hash = md4(&utf16le(password));
    let mut identity = utf16le(&user.to_uppercase());
    identity.extend_from_slice(&utf16le(domain));
    hmac_md5(&nt_hash, &identity)
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
    fn new(exported_session_key: &[u8; 16]) -> Self {
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

    /// Test helper: build a peer context with swapped directions, so it can
    /// exchange sealed tokens with a normal context derived from the same key
    /// (its send keys are the other side's receive keys and vice versa).
    #[cfg(test)]
    pub fn mirror_for_test(exported_session_key: &[u8; 16]) -> Self {
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
        let field = |data: &[u8], offset: &mut u32| -> [u8; 8] {
            let len = data.len() as u16;
            let mut f = [0u8; 8];
            f[..2].copy_from_slice(&len.to_le_bytes());
            f[2..4].copy_from_slice(&len.to_le_bytes());
            f[4..8].copy_from_slice(&offset.to_le_bytes());
            *offset += data.len() as u32;
            f
        };
        let lm_field = field(&lm_response, &mut offset);
        let nt_field = field(nt_response, &mut offset);
        let domain_field = field(&domain, &mut offset);
        let user_field = field(&user, &mut offset);
        let workstation_field = field(&workstation, &mut offset);
        let session_key_field = field(encrypted_session_key, &mut offset);

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
        // A client context and a mirror "server" context (its server keys are
        // the client's client keys) agree on the sealed token.
        let esk = [0x55u8; 16];
        let mut client = NtlmContext::new(&esk);
        // Build a peer whose server-direction keys equal our client-direction
        // keys by swapping the derivation roles.
        let mut server = NtlmContext {
            client_signing_key: client.server_signing_key,
            server_signing_key: client.client_signing_key,
            client_seal: Rc4::new(&derive_key(&esk, SERVER_SEAL_MAGIC)),
            server_seal: Rc4::new(&derive_key(&esk, CLIENT_SEAL_MAGIC)),
            client_seq: 0,
            server_seq: 0,
        };
        let sealed = client.encrypt_message(b"public-key-bytes");
        let recovered = server.decrypt_message(&sealed).unwrap();
        assert_eq!(recovered, b"public-key-bytes");
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
}
