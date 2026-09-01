//! Noise IK handshake, mirroring Go `control/controlbase/handshake.go`
//! byte-for-byte (see PROTOCOL.md).
//!
//! Sans-I/O: [`client_initiation`] produces the 101-byte initiation message
//! and a [`ClientHandshake`] that is finished with the server's 48-byte
//! response payload. I/O framing lives in the caller.

use blake2::{Blake2s256, Digest};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::SimpleHkdf;
use ts_key::MachinePrivate;

/// `Noise_IK_25519_ChaChaPoly_BLAKE2s`, fixed by the Noise spec.
const PROTOCOL_NAME: &[u8] = b"Noise_IK_25519_ChaChaPoly_BLAKE2s";
const PROLOGUE_PREFIX: &[u8] = b"Tailscale Control Protocol v";

pub const MSG_TYPE_INITIATION: u8 = 1;
pub const MSG_TYPE_RESPONSE: u8 = 2;
pub const MSG_TYPE_ERROR: u8 = 3;
pub const MSG_TYPE_RECORD: u8 = 4;

/// Total size of the initiation message on the wire.
pub const INITIATION_LEN: usize = 101;
/// Payload size of the response message (after its 3-byte header).
pub const RESPONSE_PAYLOAD_LEN: usize = 48;

const TAG_LEN: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error("server handshake payload failed to authenticate")]
    BadServerTag,
}

/// Keys and identity of an established session.
pub struct SessionKeys {
    /// Client-to-server AEAD key (Noise `c1`).
    pub tx: [u8; 32],
    /// Server-to-client AEAD key (Noise `c2`).
    pub rx: [u8; 32],
    /// The Noise handshake hash, for channel binding.
    pub handshake_hash: [u8; 32],
}

/// Builds the initiation message for the given identity/server/version and
/// returns the in-flight handshake state.
pub fn client_initiation(
    machine_key: &MachinePrivate,
    control_key: &[u8; 32],
    protocol_version: u16,
) -> ([u8; INITIATION_LEN], ClientHandshake) {
    let mut s = SymmetricState::new();

    // prologue: "Tailscale Control Protocol v<decimal version>"
    let mut prologue = Vec::with_capacity(PROLOGUE_PREFIX.len() + 5);
    prologue.extend_from_slice(PROLOGUE_PREFIX);
    prologue.extend_from_slice(protocol_version.to_string().as_bytes());
    s.mix_hash(&prologue);

    // <- s (pre-message: server's static key)
    s.mix_hash(control_key);

    // -> e, es, s, ss
    let mut msg = [0u8; INITIATION_LEN];
    msg[0..2].copy_from_slice(&protocol_version.to_be_bytes());
    msg[2] = MSG_TYPE_INITIATION;
    msg[3..5].copy_from_slice(&((INITIATION_LEN - 5) as u16).to_be_bytes());

    let ephemeral = MachinePrivate::generate();
    let ephemeral_pub = ephemeral.public().0;
    msg[5..37].copy_from_slice(&ephemeral_pub);
    s.mix_hash(&ephemeral_pub);

    let k = s.mix_dh(ephemeral.shared_secret(control_key)); // es
    let machine_pub_ct = s.encrypt_and_hash(&k, &machine_key.public().0);
    debug_assert_eq!(machine_pub_ct.len(), 48);
    msg[37..85].copy_from_slice(&machine_pub_ct);

    let k = s.mix_dh(machine_key.shared_secret(control_key)); // ss
    let tag = s.encrypt_and_hash(&k, &[]);
    debug_assert_eq!(tag.len(), TAG_LEN);
    msg[85..101].copy_from_slice(&tag);

    let hs = ClientHandshake {
        s,
        machine_key: machine_key.clone(),
        ephemeral,
    };
    (msg, hs)
}

/// The client's in-flight handshake state between sending the initiation
/// and receiving the server's response.
pub struct ClientHandshake {
    s: SymmetricState,
    machine_key: MachinePrivate,
    ephemeral: MachinePrivate,
}

impl ClientHandshake {
    /// Processes the server's 48-byte response payload
    /// (`<- e, ee, se`) and derives the transport keys.
    pub fn finish(
        mut self,
        response_payload: &[u8; RESPONSE_PAYLOAD_LEN],
    ) -> Result<SessionKeys, HandshakeError> {
        let mut server_ephemeral = [0u8; 32];
        server_ephemeral.copy_from_slice(&response_payload[..32]);
        let tag = &response_payload[32..48];

        self.s.mix_hash(&server_ephemeral);
        let _ = self
            .s
            .mix_dh(self.ephemeral.shared_secret(&server_ephemeral)); // ee
        let k = self
            .s
            .mix_dh(self.machine_key.shared_secret(&server_ephemeral)); // se
        self.s
            .decrypt_and_hash(&k, tag)
            .map_err(|_| HandshakeError::BadServerTag)?;

        let (tx, rx) = self.s.split();
        Ok(SessionKeys {
            tx,
            rx,
            handshake_hash: self.s.h,
        })
    }
}

/// Noise symmetric state: `h` (handshake hash) and `ck` (chaining key).
struct SymmetricState {
    h: [u8; 32],
    ck: [u8; 32],
}

impl SymmetricState {
    fn new() -> Self {
        // The protocol name is longer than 32 bytes, so h = HASH(name).
        let h: [u8; 32] = Blake2s256::digest(PROTOCOL_NAME).into();
        Self { h, ck: h }
    }

    fn mix_hash(&mut self, data: &[u8]) {
        let mut hasher = Blake2s256::new();
        hasher.update(self.h);
        hasher.update(data);
        self.h = hasher.finalize().into();
    }

    /// `MixKey(DH(...))`: HKDF-BLAKE2s(salt=ck, ikm=shared) → new ck and a
    /// single-use message key.
    fn mix_dh(&mut self, shared: [u8; 32]) -> [u8; 32] {
        let hk = SimpleHkdf::<Blake2s256>::new(Some(&self.ck), &shared);
        let mut okm = [0u8; 64];
        hk.expand(&[], &mut okm)
            .expect("64 bytes is a valid HKDF length");
        self.ck.copy_from_slice(&okm[..32]);
        let mut k = [0u8; 32];
        k.copy_from_slice(&okm[32..]);
        k
    }

    /// Seals `plaintext` with a single-use key (all-zero nonce, AAD = h) and
    /// mixes the ciphertext into `h`.
    fn encrypt_and_hash(&mut self, k: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
        let cipher = ChaCha20Poly1305::new(<&Key>::from(&k[..]));
        let ct = cipher
            .encrypt(
                &Nonce::default(),
                Payload {
                    msg: plaintext,
                    aad: &self.h,
                },
            )
            .expect("encryption is infallible for in-memory buffers");
        self.mix_hash(&ct);
        ct
    }

    /// Opens `ciphertext` (all-zero nonce, AAD = h); on success mixes the
    /// ciphertext into `h`. Only used for the empty-payload tag on the
    /// client side.
    fn decrypt_and_hash(&mut self, k: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, ()> {
        let cipher = ChaCha20Poly1305::new(<&Key>::from(&k[..]));
        let pt = cipher
            .decrypt(
                &Nonce::default(),
                Payload {
                    msg: ciphertext,
                    aad: &self.h,
                },
            )
            .map_err(|_| ())?;
        self.mix_hash(ciphertext);
        Ok(pt)
    }

    /// Derives the two transport keys: HKDF-BLAKE2s(salt=ck, ikm=empty).
    fn split(&self) -> ([u8; 32], [u8; 32]) {
        let hk = SimpleHkdf::<Blake2s256>::new(Some(&self.ck), &[]);
        let mut okm = [0u8; 64];
        hk.expand(&[], &mut okm)
            .expect("64 bytes is a valid HKDF length");
        let mut k1 = [0u8; 32];
        let mut k2 = [0u8; 32];
        k1.copy_from_slice(&okm[..32]);
        k2.copy_from_slice(&okm[32..]);
        (k1, k2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initiation_layout() {
        let machine = MachinePrivate::generate();
        let control = MachinePrivate::generate();
        let (msg, _) = client_initiation(&machine, &control.public().0, 123);

        assert_eq!(u16::from_be_bytes([msg[0], msg[1]]), 123);
        assert_eq!(msg[2], MSG_TYPE_INITIATION);
        assert_eq!(u16::from_be_bytes([msg[3], msg[4]]), 96);
        // Cleartext ephemeral is a valid-looking x25519 point (non-zero).
        assert_ne!(&msg[5..37], &[0u8; 32]);
    }

    /// The full IK exchange against a minimal in-test Noise responder built
    /// from the same primitives, proving internal consistency (Go interop is
    /// covered by the `interop/noise-server-go` harness).
    #[test]
    fn full_handshake_against_reference_responder() {
        let machine = MachinePrivate::generate();
        let control = MachinePrivate::generate();
        let control_pub = control.public().0;

        let (init, client) = client_initiation(&machine, &control_pub, 123);

        // ---- reference responder (mirrors controlbase.Server) ----
        let mut s = SymmetricState::new();
        s.mix_hash(b"Tailscale Control Protocol v123");
        s.mix_hash(&control_pub);

        let mut client_eph = [0u8; 32];
        client_eph.copy_from_slice(&init[5..37]);
        s.mix_hash(&client_eph);
        let k = s.mix_dh(control.shared_secret(&client_eph)); // es
        let machine_pub_pt = s.decrypt_and_hash(&k, &init[37..85]).expect("machine key");
        let mut machine_pub = [0u8; 32];
        machine_pub.copy_from_slice(&machine_pub_pt);
        assert_eq!(
            machine_pub,
            machine.public().0,
            "server sees client identity"
        );
        let k = s.mix_dh(control.shared_secret(&machine_pub)); // ss
        s.decrypt_and_hash(&k, &init[85..101])
            .expect("initiation tag");

        let server_eph = MachinePrivate::generate();
        let mut resp = [0u8; RESPONSE_PAYLOAD_LEN];
        resp[..32].copy_from_slice(&server_eph.public().0);
        s.mix_hash(&server_eph.public().0);
        let _ = s.mix_dh(server_eph.shared_secret(&client_eph)); // ee
        let k = s.mix_dh(server_eph.shared_secret(&machine_pub)); // se
        let tag = s.encrypt_and_hash(&k, &[]);
        resp[32..].copy_from_slice(&tag);
        let (c1, c2) = s.split();
        let server_hash = s.h;
        // ---- end responder ----

        let keys = client.finish(&resp).expect("client accepts response");
        assert_eq!(keys.tx, c1, "client tx == server c1");
        assert_eq!(keys.rx, c2, "client rx == server c2");
        assert_eq!(keys.handshake_hash, server_hash);
    }

    #[test]
    fn tampered_response_rejected() {
        let machine = MachinePrivate::generate();
        let control = MachinePrivate::generate();
        let (_, client) = client_initiation(&machine, &control.public().0, 123);
        let garbage = [0x42u8; RESPONSE_PAYLOAD_LEN];
        assert!(matches!(
            client.finish(&garbage),
            Err(HandshakeError::BadServerTag)
        ));
    }
}
