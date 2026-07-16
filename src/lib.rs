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
//!             └─ MCS                  (crate::mcs)   Connect + domain PDUs
//!                  ├─ BER codec       (crate::ber)   Connect-Initial/Response
//!                  ├─ PER codec       (crate::per)   domain PDUs
//!                  └─ GCC             (crate::gcc)   T.124 conference + blocks
//!                       └─ security / capabilities ...  (future)
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

pub mod ber;
pub mod cursor;
pub mod error;
pub mod gcc;
pub mod mcs;
pub mod nego;
pub mod per;
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

    /// After the X.224 handshake, RDP PDUs ride as MCS domain PDUs inside
    /// X.224 Data TPDUs inside TPKT. Exercise that whole nesting.
    #[test]
    fn mcs_send_data_over_x224_over_tpkt() {
        use crate::mcs::{DomainPdu, MCS_GLOBAL_CHANNEL_ID};

        let rdp_payload = [0x11, 0x22, 0x33, 0x44];
        let mcs = DomainPdu::SendDataRequest {
            initiator: 1007,
            channel_id: MCS_GLOBAL_CHANNEL_ID,
            user_data: &rdp_payload,
        };
        let mcs_bytes = mcs.to_vec().unwrap();
        let tpdu = X224::data(&mcs_bytes).to_vec().unwrap();
        let packet = Tpkt::new(&tpdu).to_vec().unwrap();

        // Peel the layers back off.
        let tpkt = Tpkt::decode(&packet).unwrap();
        let x224 = X224::decode(tpkt.payload).unwrap();
        let inner = match x224 {
            X224::Data(payload) => payload,
            other => panic!("expected Data TPDU, got {other:?}"),
        };
        assert_eq!(DomainPdu::decode(inner).unwrap(), mcs);
    }

    /// The client's MCS Connect-Initial: GCC settings blocks wrapped in a
    /// Conference Create Request, wrapped in Connect-Initial, wrapped in an
    /// X.224 Data TPDU, wrapped in TPKT. Build it and peel it fully back.
    #[test]
    fn connect_initial_full_stack() {
        use crate::gcc::{
            self, ClientCoreData, ClientSecurityData, UserDataBlock, ENCRYPTION_METHOD_128BIT,
        };
        use crate::mcs::ConnectInitial;

        let blocks = vec![
            UserDataBlock::ClientCore(ClientCoreData::new(1280, 800, "rusty-rdp")),
            UserDataBlock::ClientSecurity(ClientSecurityData {
                encryption_methods: ENCRYPTION_METHOD_128BIT,
                ext_encryption_methods: 0,
            }),
        ];

        // Build bottom-up.
        let user_data = gcc::encode_user_data(&blocks).unwrap();
        let ccr = gcc::encode_conference_create_request(&user_data).unwrap();
        let connect_initial = ConnectInitial::new(ccr).to_vec();
        let tpdu = X224::data(&connect_initial).to_vec().unwrap();
        let packet = Tpkt::new(&tpdu).to_vec().unwrap();

        // Peel top-down.
        let tpkt = Tpkt::decode(&packet).unwrap();
        let inner = match X224::decode(tpkt.payload).unwrap() {
            X224::Data(payload) => payload,
            other => panic!("expected Data TPDU, got {other:?}"),
        };
        let ci = ConnectInitial::decode(inner).unwrap();
        let gcc_blocks = gcc::decode_conference_create_request(&ci.user_data).unwrap();
        assert_eq!(gcc::parse_user_data(&gcc_blocks).unwrap(), blocks);
    }
}
