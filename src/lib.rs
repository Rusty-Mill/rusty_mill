//! `rusty_h2`: a from-scratch HTTP/2 implementation, built directly from
//! RFC 9113 (HTTP/2) and RFC 7541 (HPACK).
//!
//! This crate provides the wire-format building blocks — frame
//! encoding/decoding ([`frame`]), header compression ([`hpack`]), and the
//! per-stream state machine ([`stream`]) — plus a connection driver
//! ([`connect`]) that ties them together: SETTINGS negotiation,
//! connection/stream flow control, and frame dispatch, with thin
//! [`client`]/[`server`] wrappers for building requests/responses. It
//! does not yet include async I/O integration — callers own reading
//! frames off, and writing frames to, the wire themselves.

pub mod client;
pub mod connect;
pub mod error;
pub mod frame;
pub mod hpack;
pub mod server;
pub mod stream;

/// The 24-octet client connection preface (RFC 9113 §3.4) that every
/// HTTP/2 connection begins with, immediately followed by a SETTINGS frame.
pub const CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_preface_is_24_octets() {
        assert_eq!(CONNECTION_PREFACE.len(), 24);
    }
}
