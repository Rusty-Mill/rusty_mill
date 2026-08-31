//! RDP licensing PDUs (MS-RDPBCGR 2.2.1.12).
//!
//! Immediately after the Client Info PDU the server runs a short licensing
//! exchange. For the common case — a per-device CAL is not required, or the
//! client already has one — the server simply sends a **License Error
//! Message** with `STATUS_VALID_CLIENT`, which tells the client to proceed to
//! capability exchange.
//!
//! Every licensing PDU is prefixed with a `LICENSE_PREAMBLE` and, on the
//! wire, wrapped in a Basic Security Header carrying `SEC_LICENSE_PKT`
//! ([`crate::security`]). This module models the preamble, the License Error
//! Message (the branch a minimal client must understand), and keeps the other
//! message types as raw bodies.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// Length of a `LICENSE_PREAMBLE` in bytes.
pub const LICENSE_PREAMBLE_LEN: usize = 4;

// Licensing message types (the preamble `bMsgType`).
/// `LICENSE_REQUEST` — server requests licensing information.
pub const LICENSE_REQUEST: u8 = 0x01;
/// `PLATFORM_CHALLENGE` — server platform challenge.
pub const PLATFORM_CHALLENGE: u8 = 0x02;
/// `NEW_LICENSE` — server issues a new license.
pub const NEW_LICENSE: u8 = 0x03;
/// `UPGRADE_LICENSE` — server upgrades an existing license.
pub const UPGRADE_LICENSE: u8 = 0x04;
/// `LICENSE_INFO` — client license information.
pub const LICENSE_INFO: u8 = 0x12;
/// `NEW_LICENSE_REQUEST` — client requests a new license.
pub const NEW_LICENSE_REQUEST: u8 = 0x13;
/// `PLATFORM_CHALLENGE_RESPONSE` — client challenge response.
pub const PLATFORM_CHALLENGE_RESPONSE: u8 = 0x15;
/// `ERROR_ALERT` — a License Error Message.
pub const ERROR_ALERT: u8 = 0xFF;

// Preamble flags.
/// `PREAMBLE_VERSION_3_0`.
pub const PREAMBLE_VERSION_3_0: u8 = 0x03;
/// `EXTENDED_ERROR_MSG_SUPPORTED`.
pub const EXTENDED_ERROR_MSG_SUPPORTED: u8 = 0x80;

// License Error Message error codes (2.2.1.12.1.3).
/// `ERR_INVALID_SERVER_CERTIFICATE`.
pub const ERR_INVALID_SERVER_CERTIFICATE: u32 = 0x0000_0001;
/// `ERR_NO_LICENSE`.
pub const ERR_NO_LICENSE: u32 = 0x0000_0002;
/// `STATUS_VALID_CLIENT` — licensing is complete; proceed.
pub const STATUS_VALID_CLIENT: u32 = 0x0000_0007;

// State transition values.
/// `ST_TOTAL_ABORT`.
pub const ST_TOTAL_ABORT: u32 = 0x0000_0001;
/// `ST_NO_TRANSITION` — remain in the current state (used with a valid client).
pub const ST_NO_TRANSITION: u32 = 0x0000_0002;
/// `ST_RESET_PHASE_TO_START`.
pub const ST_RESET_PHASE_TO_START: u32 = 0x0000_0003;

/// A License Error Message (`bMsgType == ERROR_ALERT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicenseErrorMessage {
    /// `dwErrorCode`.
    pub error_code: u32,
    /// `dwStateTransition`.
    pub state_transition: u32,
    /// `wBlobType` of the attached binary blob (0 when empty).
    pub blob_type: u16,
    /// The binary blob contents (usually empty).
    pub blob: Vec<u8>,
}

impl LicenseErrorMessage {
    /// The message a server sends when no license is needed:
    /// `STATUS_VALID_CLIENT` with `ST_NO_TRANSITION` and an empty blob.
    pub fn valid_client() -> Self {
        LicenseErrorMessage {
            error_code: STATUS_VALID_CLIENT,
            state_transition: ST_NO_TRANSITION,
            blob_type: 0,
            blob: Vec::new(),
        }
    }

    /// Returns `true` if this message signals that the client may proceed to
    /// capability exchange.
    pub fn is_valid_client(&self) -> bool {
        self.error_code == STATUS_VALID_CLIENT
    }

    fn encode_body(&self, w: &mut Writer) {
        w.write_u32_le(self.error_code);
        w.write_u32_le(self.state_transition);
        // LICENSE_BINARY_BLOB.
        w.write_u16_le(self.blob_type);
        w.write_u16_le(self.blob.len() as u16);
        w.write_bytes(&self.blob);
    }

    fn decode_body(r: &mut Reader<'_>) -> Result<LicenseErrorMessage> {
        let error_code = r.read_u32_le()?;
        let state_transition = r.read_u32_le()?;
        let blob_type = r.read_u16_le()?;
        let blob_len = r.read_u16_le()? as usize;
        let blob = r.read_bytes(blob_len)?.to_vec();
        Ok(LicenseErrorMessage {
            error_code,
            state_transition,
            blob_type,
            blob,
        })
    }
}

/// A licensing PDU (preamble plus body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicensePdu {
    /// A License Error Message, including the `STATUS_VALID_CLIENT` case.
    ErrorAlert(LicenseErrorMessage),
    /// Any other licensing message, kept as its raw body for later handling.
    Other {
        /// The preamble `bMsgType`.
        msg_type: u8,
        /// The preamble `flags`.
        flags: u8,
        /// The message body after the preamble.
        body: Vec<u8>,
    },
}

impl LicensePdu {
    /// Encode this PDU with a `LICENSE_PREAMBLE`.
    ///
    /// `flags` defaults are applied for [`LicensePdu::ErrorAlert`]; the
    /// [`LicensePdu::Other`] variant carries its own flags.
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let (msg_type, flags, body) = match self {
            LicensePdu::ErrorAlert(msg) => {
                let mut b = Writer::new();
                msg.encode_body(&mut b);
                (
                    ERROR_ALERT,
                    PREAMBLE_VERSION_3_0 | EXTENDED_ERROR_MSG_SUPPORTED,
                    b.into_vec(),
                )
            }
            LicensePdu::Other {
                msg_type,
                flags,
                body,
            } => (*msg_type, *flags, body.clone()),
        };

        let total = LICENSE_PREAMBLE_LEN + body.len();
        if total > u16::MAX as usize {
            return Err(Error::Overflow {
                field: "license wMsgSize",
            });
        }
        let mut w = Writer::with_capacity(total);
        w.write_u8(msg_type);
        w.write_u8(flags);
        w.write_u16_le(total as u16);
        w.write_bytes(&body);
        Ok(w.into_vec())
    }

    /// Decode a licensing PDU from its preamble onward.
    pub fn decode(buf: &[u8]) -> Result<LicensePdu> {
        let mut r = Reader::new(buf);
        let msg_type = r.read_u8()?;
        let flags = r.read_u8()?;
        let msg_size = r.read_u16_le()? as usize;
        if msg_size < LICENSE_PREAMBLE_LEN || msg_size > buf.len() {
            return Err(Error::InvalidLength {
                field: "license wMsgSize",
                length: msg_size,
            });
        }
        let body = r.read_bytes(msg_size - LICENSE_PREAMBLE_LEN)?;

        if msg_type == ERROR_ALERT {
            let mut br = Reader::new(body);
            Ok(LicensePdu::ErrorAlert(LicenseErrorMessage::decode_body(
                &mut br,
            )?))
        } else {
            Ok(LicensePdu::Other {
                msg_type,
                flags,
                body: body.to_vec(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_client_roundtrip() {
        let pdu = LicensePdu::ErrorAlert(LicenseErrorMessage::valid_client());
        let bytes = pdu.to_vec().unwrap();
        // Preamble: ERROR_ALERT, flags 0x83, wMsgSize.
        assert_eq!(bytes[0], ERROR_ALERT);
        assert_eq!(bytes[1], 0x83);
        // Body: errorCode 0x07, stateTransition 0x02, blobType 0, blobLen 0.
        assert_eq!(&bytes[4..8], &STATUS_VALID_CLIENT.to_le_bytes());
        assert_eq!(&bytes[8..12], &ST_NO_TRANSITION.to_le_bytes());

        match LicensePdu::decode(&bytes).unwrap() {
            LicensePdu::ErrorAlert(msg) => {
                assert!(msg.is_valid_client());
                assert_eq!(msg, LicenseErrorMessage::valid_client());
            }
            other => panic!("expected ErrorAlert, got {other:?}"),
        }
    }

    #[test]
    fn error_alert_with_blob_roundtrip() {
        let msg = LicenseErrorMessage {
            error_code: ERR_NO_LICENSE,
            state_transition: ST_RESET_PHASE_TO_START,
            blob_type: 0x0009,
            blob: vec![0xDE, 0xAD, 0xBE, 0xEF],
        };
        let pdu = LicensePdu::ErrorAlert(msg.clone());
        let bytes = pdu.to_vec().unwrap();
        assert_eq!(
            LicensePdu::decode(&bytes).unwrap(),
            LicensePdu::ErrorAlert(msg)
        );
    }

    #[test]
    fn other_message_kept_raw() {
        let pdu = LicensePdu::Other {
            msg_type: LICENSE_REQUEST,
            flags: PREAMBLE_VERSION_3_0,
            body: vec![0x01, 0x02, 0x03],
        };
        let bytes = pdu.to_vec().unwrap();
        assert_eq!(bytes[0], LICENSE_REQUEST);
        assert_eq!(LicensePdu::decode(&bytes).unwrap(), pdu);
    }

    #[test]
    fn rejects_bad_msg_size() {
        let bytes = [ERROR_ALERT, 0x83, 0xFF, 0x00]; // claims 255 bytes
        assert!(matches!(
            LicensePdu::decode(&bytes).unwrap_err(),
            Error::InvalidLength {
                field: "license wMsgSize",
                ..
            }
        ));
    }
}
