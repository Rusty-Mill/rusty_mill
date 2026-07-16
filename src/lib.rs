//! # rusty_rdp
//!
//! A minimal, **dependency-free** implementation of the Remote Desktop
//! Protocol (RDP) wire format.
//!
//! The crate has zero third-party dependencies — only the Rust standard
//! library — and every wire structure is encoded and decoded by hand with
//! bounds-checked cursors. `unsafe` is forbidden crate-wide.
//!
//! ## Layering
//!
//! RDP is a stack of nested framings carried over a single TCP connection.
//! This crate models the lower layers first; higher layers are added on top
//! without changing what is already here.
//!
//! ```text
//! TCP stream
//!   └─ TPKT           (crate::tpkt)   4-byte length framing
//!        └─ X.224     (crate::x224)   Class 0 CR / CC / Data TPDUs
//!             └─ RDP negotiation      (crate::nego)  security selection
//!             └─ MCS / PDUs ...       (future)
//! ```
//!
//! ## Design
//!
//! * **No I/O in the codec.** Every type encodes to and decodes from byte
//!   slices, so the same code works with blocking sockets, async runtimes,
//!   or in-memory tests. [`tpkt::Tpkt::peek_total_len`] lets a caller frame a
//!   TCP stream without committing to a runtime.
//! * **Explicit endianness.** [`cursor::Reader`] / [`cursor::Writer`] make
//!   every big/little-endian access visible at the call site, because RDP
//!   mixes the two.
//! * **Total decoding.** Malformed input yields an [`Error`], never a panic.
//!
//! ## Example: build a Connection Request
//!
//! ```
//! use rusty_rdp::nego::{Negotiation, SecurityProtocols};
//! use rusty_rdp::tpkt::Tpkt;
//! use rusty_rdp::x224::X224;
//!
//! // Client asks for TLS or CredSSP.
//! let neg = Negotiation::Request {
//!     flags: 0,
//!     protocols: SecurityProtocols::SSL | SecurityProtocols::HYBRID,
//! };
//! let x224 = X224::connection_request(neg);
//! let tpdu = x224.to_vec().unwrap();
//! let packet = Tpkt::new(&tpdu).to_vec().unwrap();
//!
//! // `packet` is now ready to write to a TCP socket. Decode the round trip:
//! let tpkt = Tpkt::decode(&packet).unwrap();
//! assert_eq!(X224::decode(tpkt.payload).unwrap(), x224);
//! ```

pub mod cursor;
pub mod error;
pub mod nego;
pub mod tpkt;
pub mod x224;

pub use error::{Error, Result};

#[cfg(test)]
mod integration_tests {
    use crate::nego::{Negotiation, SecurityProtocols};
    use crate::tpkt::Tpkt;
    use crate::x224::X224;

    /// Full stack: encode a CR inside TPKT, then decode both layers back.
    #[test]
    fn tpkt_x224_nego_full_stack() {
        let neg = Negotiation::Request {
            flags: 0,
            protocols: SecurityProtocols::SSL,
        };
        let x224 = X224::connection_request(neg);
        let tpdu = x224.to_vec().unwrap();
        let packet = Tpkt::new(&tpdu).to_vec().unwrap();

        // Frame it as if arriving on a stream.
        assert_eq!(Tpkt::peek_total_len(&packet).unwrap(), Some(packet.len()));
        let tpkt = Tpkt::decode(&packet).unwrap();
        assert_eq!(X224::decode(tpkt.payload).unwrap(), x224);
    }
}
