//! RDP security negotiation (MS-RDPBCGR §2.2.1.1.1 / §2.2.1.2.1).
//!
//! During the connection handshake the client places an `RDP_NEG_REQ` in the
//! variable part of the X.224 Connection Request, advertising which security
//! protocols it supports. The server answers in the Connection Confirm with
//! either an `RDP_NEG_RSP` naming the chosen protocol or an
//! `RDP_NEG_FAILURE` explaining why negotiation could not proceed.
//!
//! All three structures are a fixed eight bytes:
//!
//! ```text
//! +------+-------+-------------+---------------------+
//! | type | flags | length (LE) |  payload (LE u32)   |
//! | u8   | u8    | u16 = 8     |  protocols/code     |
//! +------+-------+-------------+---------------------+
//! ```

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// Fixed encoded size of every negotiation structure.
pub const NEG_LEN: usize = 8;

const TYPE_NEG_REQ: u8 = 0x01;
const TYPE_NEG_RSP: u8 = 0x02;
const TYPE_NEG_FAILURE: u8 = 0x03;

/// Security protocols that may be requested or selected.
///
/// The wire representation is a bitmask (`requestedProtocols`); the server's
/// `selectedProtocol` names exactly one. The flag values are defined by
/// MS-RDPBCGR §2.2.1.1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SecurityProtocols(pub u32);

impl SecurityProtocols {
    /// Standard RDP security (no external TLS). Value `0x0000_0000`.
    pub const RDP: SecurityProtocols = SecurityProtocols(0x0000_0000);
    /// TLS 1.x. Value `0x0000_0001`.
    pub const SSL: SecurityProtocols = SecurityProtocols(0x0000_0001);
    /// CredSSP (NLA). Value `0x0000_0002`.
    pub const HYBRID: SecurityProtocols = SecurityProtocols(0x0000_0002);
    /// RDSTLS. Value `0x0000_0004`.
    pub const RDSTLS: SecurityProtocols = SecurityProtocols(0x0000_0004);
    /// CredSSP with early user authorization. Value `0x0000_0008`.
    pub const HYBRID_EX: SecurityProtocols = SecurityProtocols(0x0000_0008);

    /// Returns `true` if every protocol in `other` is set in `self`.
    pub fn contains(self, other: SecurityProtocols) -> bool {
        // RDP (0) is only "contained" when self is also exactly RDP.
        if other.0 == 0 {
            return self.0 == 0;
        }
        self.0 & other.0 == other.0
    }

    /// Union of two protocol sets.
    pub fn union(self, other: SecurityProtocols) -> SecurityProtocols {
        SecurityProtocols(self.0 | other.0)
    }
}

impl core::ops::BitOr for SecurityProtocols {
    type Output = SecurityProtocols;
    fn bitor(self, rhs: SecurityProtocols) -> SecurityProtocols {
        self.union(rhs)
    }
}

/// Reasons a server rejects negotiation (MS-RDPBCGR §2.2.1.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegFailureCode {
    /// The server requires SSL/TLS and the client did not offer it.
    SslRequiredByServer,
    /// The server is configured to reject SSL/TLS connections.
    SslNotAllowedByServer,
    /// The server cannot find the certificate it needs for SSL/TLS.
    SslCertNotOnServer,
    /// The client requested a protocol the server did not expect.
    InconsistentFlags,
    /// The server requires credentials-based (Hybrid) security.
    HybridRequiredByServer,
    /// SSL was selected but the certificate could not authenticate the server.
    SslWithUserAuthRequiredByServer,
    /// A failure code not defined by this crate.
    Unknown(u32),
}

impl NegFailureCode {
    fn from_u32(v: u32) -> NegFailureCode {
        match v {
            0x0000_0001 => NegFailureCode::SslRequiredByServer,
            0x0000_0002 => NegFailureCode::SslNotAllowedByServer,
            0x0000_0003 => NegFailureCode::SslCertNotOnServer,
            0x0000_0004 => NegFailureCode::InconsistentFlags,
            0x0000_0005 => NegFailureCode::HybridRequiredByServer,
            0x0000_0006 => NegFailureCode::SslWithUserAuthRequiredByServer,
            other => NegFailureCode::Unknown(other),
        }
    }

    fn to_u32(self) -> u32 {
        match self {
            NegFailureCode::SslRequiredByServer => 0x0000_0001,
            NegFailureCode::SslNotAllowedByServer => 0x0000_0002,
            NegFailureCode::SslCertNotOnServer => 0x0000_0003,
            NegFailureCode::InconsistentFlags => 0x0000_0004,
            NegFailureCode::HybridRequiredByServer => 0x0000_0005,
            NegFailureCode::SslWithUserAuthRequiredByServer => 0x0000_0006,
            NegFailureCode::Unknown(v) => v,
        }
    }
}

/// The negotiation structure carried in a Connection Request or Confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Negotiation {
    /// Client request advertising supported protocols and request flags.
    Request {
        /// `flags` byte (e.g. `RESTRICTED_ADMIN_MODE_REQUIRED`).
        flags: u8,
        /// Bitmask of protocols the client supports.
        protocols: SecurityProtocols,
    },
    /// Server response naming the selected protocol.
    Response {
        /// `flags` byte (e.g. `EXTENDED_CLIENT_DATA_SUPPORTED`).
        flags: u8,
        /// The single protocol the server selected.
        selected: SecurityProtocols,
    },
    /// Server response rejecting the negotiation.
    Failure {
        /// Why the negotiation was rejected.
        code: NegFailureCode,
    },
}

impl Negotiation {
    /// Encode this structure (always [`NEG_LEN`] bytes) into `w`.
    pub fn encode(&self, w: &mut Writer) {
        let (ty, flags, payload) = match *self {
            Negotiation::Request { flags, protocols } => (TYPE_NEG_REQ, flags, protocols.0),
            Negotiation::Response { flags, selected } => (TYPE_NEG_RSP, flags, selected.0),
            Negotiation::Failure { code } => (TYPE_NEG_FAILURE, 0, code.to_u32()),
        };
        w.write_u8(ty);
        w.write_u8(flags);
        w.write_u16_le(NEG_LEN as u16);
        w.write_u32_le(payload);
    }

    /// Decode a negotiation structure from `r`.
    ///
    /// Validates the `type` byte and the fixed `length` field.
    pub fn decode(r: &mut Reader<'_>) -> Result<Negotiation> {
        let ty = r.read_u8()?;
        let flags = r.read_u8()?;
        let length = r.read_u16_le()?;
        if length as usize != NEG_LEN {
            return Err(Error::InvalidLength {
                field: "RDP_NEG length",
                length: length as usize,
            });
        }
        let payload = r.read_u32_le()?;
        match ty {
            TYPE_NEG_REQ => Ok(Negotiation::Request {
                flags,
                protocols: SecurityProtocols(payload),
            }),
            TYPE_NEG_RSP => Ok(Negotiation::Response {
                flags,
                selected: SecurityProtocols(payload),
            }),
            TYPE_NEG_FAILURE => Ok(Negotiation::Failure {
                code: NegFailureCode::from_u32(payload),
            }),
            other => Err(Error::InvalidValue {
                field: "RDP_NEG type",
                value: format!("0x{other:02X}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let neg = Negotiation::Request {
            flags: 0,
            protocols: SecurityProtocols::SSL | SecurityProtocols::HYBRID,
        };
        let mut w = Writer::new();
        neg.encode(&mut w);
        let bytes = w.into_vec();
        assert_eq!(bytes, [0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00, 0x00]);
        let mut r = Reader::new(&bytes);
        assert_eq!(Negotiation::decode(&mut r).unwrap(), neg);
    }

    #[test]
    fn response_roundtrip() {
        let neg = Negotiation::Response {
            flags: 0x02,
            selected: SecurityProtocols::SSL,
        };
        let mut w = Writer::new();
        neg.encode(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(Negotiation::decode(&mut r).unwrap(), neg);
    }

    #[test]
    fn failure_maps_known_code() {
        let neg = Negotiation::Failure {
            code: NegFailureCode::HybridRequiredByServer,
        };
        let mut w = Writer::new();
        neg.encode(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(Negotiation::decode(&mut r).unwrap(), neg);
    }

    #[test]
    fn rejects_wrong_length() {
        let bytes = [0x01, 0x00, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00];
        let mut r = Reader::new(&bytes);
        assert!(matches!(
            Negotiation::decode(&mut r).unwrap_err(),
            Error::InvalidLength { .. }
        ));
    }

    #[test]
    fn protocol_contains_semantics() {
        let both = SecurityProtocols::SSL | SecurityProtocols::HYBRID;
        assert!(both.contains(SecurityProtocols::SSL));
        assert!(both.contains(SecurityProtocols::HYBRID));
        assert!(!both.contains(SecurityProtocols::RDSTLS));
        // Plain RDP (0) is only contained in an all-zero set.
        assert!(SecurityProtocols::RDP.contains(SecurityProtocols::RDP));
        assert!(!both.contains(SecurityProtocols::RDP));
    }
}
