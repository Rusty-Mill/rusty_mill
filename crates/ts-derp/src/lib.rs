//! DERP relay client: frame codec, HTTP upgrade, NaCl-box handshake, and a
//! send/receive path keyed by node public key. Protocol spec: PROTOCOL.md;
//! ground truth is Go `derp/` at v1.86.2.

pub mod frame;

mod client;

pub use client::{DerpClient, DerpError, DerpSender, RelayedPacket};
