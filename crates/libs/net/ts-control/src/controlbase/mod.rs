//! The base transport of the ts2021 control protocol: Noise IK,
//! instantiated with Curve25519, ChaCha20-Poly1305, and BLAKE2s, plus
//! Tailscale's record framing. Mirrors Go `control/controlbase`.

mod conn;
mod handshake;
mod io;

pub use conn::Conn;
pub use handshake::{
    Accepted, ClientHandshake, HandshakeError, INITIATION_LEN, MSG_TYPE_ERROR, MSG_TYPE_RESPONSE,
    RESPONSE_PAYLOAD_LEN, SessionKeys, client_initiation, server_respond,
};
pub use io::{ConnectError, accept, accept_deferred, connect, connect_deferred};
