//! X.224 (ISO 8073) Class 0 transport PDUs as used by RDP.
//!
//! RDP only ever uses three TPDU kinds, all Class 0:
//!
//! * **Connection Request (CR, `0xE0`)** — sent by the client to open the
//!   ISO transport connection. Carries the optional cookie and
//!   [`Negotiation`] request.
//! * **Connection Confirm (CC, `0xD0`)** — the server's answer, carrying the
//!   [`Negotiation`] response or failure.
//! * **Data (DT, `0xF0`)** — every RDP PDU after the handshake is wrapped in
//!   one of these.
//!
//! The CR/CC layout after the TPKT header is:
//!
//! ```text
//! +----+------+---------+---------+-------+------------------+
//! | LI | code | DST-REF | SRC-REF | class |   variable...    |
//! | u8 | u8   | u16 BE  | u16 BE  | u8    | cookie + nego    |
//! +----+------+---------+---------+-------+------------------+
//! ```
//!
//! `LI` (length indicator) counts every byte after itself, i.e. the six
//! fixed bytes plus the variable part. A Data TPDU is just `LI=2`, `0xF0`,
//! `0x80` (EOT) followed by the user payload, which is *not* counted in `LI`.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};
use crate::nego::Negotiation;

/// TPDU code for a Connection Request (high nibble).
pub const TPDU_CR: u8 = 0xE0;
/// TPDU code for a Connection Confirm (high nibble).
pub const TPDU_CC: u8 = 0xD0;
/// TPDU code for a Data TPDU (high nibble).
pub const TPDU_DT: u8 = 0xF0;
/// End-of-transmission marker used in the Data TPDU's second header byte.
pub const EOT: u8 = 0x80;

/// Length in bytes of the CR/CC fixed header that follows the LI byte.
const CRCC_FIXED_LEN: usize = 6; // code + dst(2) + src(2) + class
/// Length of a Data TPDU header (LI + code + EOT).
const DT_HEADER_LEN: usize = 3;

const COOKIE_PREFIX: &[u8] = b"Cookie: ";
const MSTSHASH_PREFIX: &[u8] = b"mstshash=";
const ROUTINGTOKEN_PREFIX: &[u8] = b"msts=";

/// A routing/identification cookie in the CR variable part.
///
/// RDP defines two mutually exclusive forms, both ASCII lines terminated by
/// CRLF. `mstshash` carries the username hint used for load balancing;
/// `msts` carries an opaque routing token inserted by a broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cookie {
    /// `Cookie: mstshash=<value>\r\n`
    MsTsHash(String),
    /// `Cookie: msts=<value>\r\n`
    RoutingToken(String),
}

impl Cookie {
    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(COOKIE_PREFIX);
        match self {
            Cookie::MsTsHash(v) => {
                out.extend_from_slice(MSTSHASH_PREFIX);
                out.extend_from_slice(v.as_bytes());
            }
            Cookie::RoutingToken(v) => {
                out.extend_from_slice(ROUTINGTOKEN_PREFIX);
                out.extend_from_slice(v.as_bytes());
            }
        }
        out.extend_from_slice(b"\r\n");
    }

    /// Parse the portion after `"Cookie: "` and before the CRLF.
    fn parse_line(line: &[u8]) -> Result<Cookie> {
        if let Some(rest) = line.strip_prefix(MSTSHASH_PREFIX) {
            Ok(Cookie::MsTsHash(String::from_utf8_lossy(rest).into_owned()))
        } else if let Some(rest) = line.strip_prefix(ROUTINGTOKEN_PREFIX) {
            Ok(Cookie::RoutingToken(
                String::from_utf8_lossy(rest).into_owned(),
            ))
        } else {
            Err(Error::InvalidValue {
                field: "X.224 cookie",
                value: String::from_utf8_lossy(line).into_owned(),
            })
        }
    }
}

/// A Connection Request (client → server) or Connection Confirm
/// (server → client). The two share an identical layout; the direction is
/// encoded by the TPDU code and captured by [`X224::ConnectionRequest`] vs
/// [`X224::ConnectionConfirm`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnectionPdu {
    /// Destination reference (0 in the CR; echoed/assigned in the CC).
    pub dst_ref: u16,
    /// Source reference.
    pub src_ref: u16,
    /// Class and options byte (0 for Class 0, no options).
    pub class_options: u8,
    /// Optional routing/identification cookie (CR only in practice).
    pub cookie: Option<Cookie>,
    /// Optional RDP security negotiation structure.
    pub negotiation: Option<Negotiation>,
}

impl ConnectionPdu {
    /// Build the variable part (cookie followed by negotiation).
    fn encode_variable(&self) -> Vec<u8> {
        let mut var = Vec::new();
        if let Some(cookie) = &self.cookie {
            cookie.encode(&mut var);
        }
        if let Some(neg) = &self.negotiation {
            let mut w = Writer::new();
            neg.encode(&mut w);
            var.extend_from_slice(w.as_slice());
        }
        var
    }

    fn encode(&self, code: u8, w: &mut Writer) -> Result<()> {
        let var = self.encode_variable();
        let li = CRCC_FIXED_LEN + var.len();
        if li > u8::MAX as usize {
            return Err(Error::Overflow { field: "X.224 LI" });
        }
        w.write_u8(li as u8);
        w.write_u8(code);
        w.write_u16_be(self.dst_ref);
        w.write_u16_be(self.src_ref);
        w.write_u8(self.class_options);
        w.write_bytes(&var);
        Ok(())
    }

    /// Decode the body after the LI and code bytes have been read.
    fn decode(li: u8, r: &mut Reader<'_>) -> Result<ConnectionPdu> {
        let var_len = (li as usize)
            .checked_sub(CRCC_FIXED_LEN)
            .ok_or(Error::InvalidLength {
                field: "X.224 LI",
                length: li as usize,
            })?;
        let dst_ref = r.read_u16_be()?;
        let src_ref = r.read_u16_be()?;
        let class_options = r.read_u8()?;
        let var = r.read_bytes(var_len)?;
        let (cookie, negotiation) = Self::parse_variable(var)?;
        Ok(ConnectionPdu {
            dst_ref,
            src_ref,
            class_options,
            cookie,
            negotiation,
        })
    }

    fn parse_variable(var: &[u8]) -> Result<(Option<Cookie>, Option<Negotiation>)> {
        let mut body = var;
        let mut cookie = None;
        if let Some(after_prefix) = var.strip_prefix(COOKIE_PREFIX) {
            let crlf = find_crlf(after_prefix).ok_or(Error::InvalidValue {
                field: "X.224 cookie",
                value: "missing CRLF terminator".to_string(),
            })?;
            cookie = Some(Cookie::parse_line(&after_prefix[..crlf])?);
            body = &after_prefix[crlf + 2..];
        }
        let negotiation = if body.is_empty() {
            None
        } else {
            let mut r = Reader::new(body);
            Some(Negotiation::decode(&mut r)?)
        };
        Ok((cookie, negotiation))
    }
}

/// A decoded X.224 Class 0 TPDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum X224<'a> {
    /// Connection Request (`0xE0`).
    ConnectionRequest(ConnectionPdu),
    /// Connection Confirm (`0xD0`).
    ConnectionConfirm(ConnectionPdu),
    /// Data TPDU (`0xF0`) wrapping an opaque RDP payload.
    Data(&'a [u8]),
}

impl<'a> X224<'a> {
    /// Convenience constructor for a client Connection Request that only
    /// negotiates security (the common case).
    pub fn connection_request(negotiation: Negotiation) -> X224<'static> {
        X224::ConnectionRequest(ConnectionPdu {
            negotiation: Some(negotiation),
            ..Default::default()
        })
    }

    /// Convenience constructor for a Data TPDU wrapping `payload`.
    pub fn data(payload: &'a [u8]) -> X224<'a> {
        X224::Data(payload)
    }

    /// Encode this TPDU into `w`.
    pub fn encode(&self, w: &mut Writer) -> Result<()> {
        match self {
            X224::ConnectionRequest(pdu) => pdu.encode(TPDU_CR, w),
            X224::ConnectionConfirm(pdu) => pdu.encode(TPDU_CC, w),
            X224::Data(payload) => {
                w.write_u8((DT_HEADER_LEN - 1) as u8); // LI = 2
                w.write_u8(TPDU_DT);
                w.write_u8(EOT);
                w.write_bytes(payload);
                Ok(())
            }
        }
    }

    /// Encode into a fresh `Vec`.
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        self.encode(&mut w)?;
        Ok(w.into_vec())
    }

    /// Decode a single TPDU from `buf` (typically a TPKT payload).
    pub fn decode(buf: &'a [u8]) -> Result<X224<'a>> {
        let mut r = Reader::new(buf);
        let li = r.read_u8()?;
        let code = r.read_u8()?;
        match code & 0xF0 {
            TPDU_CR => Ok(X224::ConnectionRequest(ConnectionPdu::decode(li, &mut r)?)),
            TPDU_CC => Ok(X224::ConnectionConfirm(ConnectionPdu::decode(li, &mut r)?)),
            TPDU_DT => {
                // LI is 2 for a Class 0 Data TPDU; the third byte is EOT/nr.
                if li as usize != DT_HEADER_LEN - 1 {
                    return Err(Error::InvalidLength {
                        field: "X.224 DT LI",
                        length: li as usize,
                    });
                }
                let _eot = r.read_u8()?;
                Ok(X224::Data(r.peek_remaining()))
            }
            other => Err(Error::InvalidValue {
                field: "X.224 TPDU code",
                value: format!("0x{other:02X}"),
            }),
        }
    }
}

/// Locate the first `\r\n` in `buf`, returning the index of the `\r`.
fn find_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nego::{NegFailureCode, SecurityProtocols};

    #[test]
    fn connection_request_nego_only_roundtrip() {
        let neg = Negotiation::Request {
            flags: 0,
            protocols: SecurityProtocols::SSL | SecurityProtocols::HYBRID,
        };
        let cr = X224::connection_request(neg);
        let bytes = cr.to_vec().unwrap();
        // LI(0x0e) + CR(0xe0) + refs/class(5) + nego(8) = 15 bytes.
        assert_eq!(bytes[0], 0x0e);
        assert_eq!(bytes[1], 0xe0);
        assert_eq!(bytes.len(), 15);
        assert_eq!(X224::decode(&bytes).unwrap(), cr);
    }

    #[test]
    fn connection_request_with_cookie_roundtrip() {
        let pdu = ConnectionPdu {
            cookie: Some(Cookie::MsTsHash("eltons".to_string())),
            negotiation: Some(Negotiation::Request {
                flags: 0,
                protocols: SecurityProtocols::SSL,
            }),
            ..Default::default()
        };
        let cr = X224::ConnectionRequest(pdu);
        let bytes = cr.to_vec().unwrap();
        // Sanity: the cookie line is present verbatim.
        let text = b"Cookie: mstshash=eltons\r\n";
        assert!(bytes.windows(text.len()).any(|w| w == text));
        assert_eq!(X224::decode(&bytes).unwrap(), cr);
    }

    #[test]
    fn matches_known_wire_capture() {
        // A real CR with an mstshash cookie and an SSL nego request.
        let wire: &[u8] = &[
            0x27, 0xe0, 0x00, 0x00, 0x00, 0x00, 0x00, // LI, CR, dst, src, class
            0x43, 0x6f, 0x6f, 0x6b, 0x69, 0x65, 0x3a, 0x20, // "Cookie: "
            0x6d, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3d, // "mstshash="
            0x65, 0x6c, 0x74, 0x6f, 0x6e, 0x73, // "eltons"
            0x0d, 0x0a, // CRLF
            0x01, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00, // nego req, SSL
        ];
        let decoded = X224::decode(wire).unwrap();
        match decoded {
            X224::ConnectionRequest(ref pdu) => {
                assert_eq!(pdu.cookie, Some(Cookie::MsTsHash("eltons".to_string())));
                assert_eq!(
                    pdu.negotiation,
                    Some(Negotiation::Request {
                        flags: 0,
                        protocols: SecurityProtocols::SSL
                    })
                );
            }
            other => panic!("expected CR, got {other:?}"),
        }
        // Re-encoding must reproduce the exact bytes.
        assert_eq!(decoded.to_vec().unwrap(), wire);
    }

    #[test]
    fn connection_confirm_failure_roundtrip() {
        let cc = X224::ConnectionConfirm(ConnectionPdu {
            negotiation: Some(Negotiation::Failure {
                code: NegFailureCode::HybridRequiredByServer,
            }),
            ..Default::default()
        });
        let bytes = cc.to_vec().unwrap();
        assert_eq!(bytes[1], 0xd0);
        assert_eq!(X224::decode(&bytes).unwrap(), cc);
    }

    #[test]
    fn data_tpdu_roundtrip() {
        let payload = [0xAA, 0xBB, 0xCC, 0xDD];
        let dt = X224::data(&payload);
        let bytes = dt.to_vec().unwrap();
        assert_eq!(&bytes[..3], &[0x02, 0xf0, 0x80]);
        assert_eq!(X224::decode(&bytes).unwrap(), X224::Data(&payload));
    }

    #[test]
    fn rejects_unknown_tpdu_code() {
        let bytes = [0x02, 0x70, 0x80];
        assert!(matches!(
            X224::decode(&bytes).unwrap_err(),
            Error::InvalidValue {
                field: "X.224 TPDU code",
                ..
            }
        ));
    }
}
