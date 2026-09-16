//! Connection preface processing (RFC 9113 §3.4).

use crate::error::{ErrorCode, H2Error, Result};
use crate::frame;

/// The canonical client connection preface.
pub const CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// Process a client connection preface.
///
/// Returns the remaining bytes after the preface (typically a SETTINGS frame).
/// The preface must be exactly 24 bytes and must appear at the start of the
/// connection.
pub fn process_preface(data: &[u8]) -> Result<&[u8]> {
    if data.len() < CONNECTION_PREFACE.len() {
        return Err(H2Error::Connection(
            ErrorCode::ProtocolError,
            "connection preface incomplete",
        ));
    }

    if &data[..CONNECTION_PREFACE.len()] != CONNECTION_PREFACE {
        return Err(H2Error::Connection(
            ErrorCode::ProtocolError,
            "invalid connection preface",
        ));
    }

    Ok(&data[CONNECTION_PREFACE.len()..])
}

/// Process a server connection preface.
///
/// The server's preface is the same 24-octet string, but the first frame
/// MUST be a SETTINGS frame (not the client's SETTINGS following the preface).
/// This function validates the preface and returns the remaining bytes.
pub fn process_server_preface(data: &[u8]) -> Result<(&[u8], frame::SettingsFrame)> {
    let remaining = process_preface(data)?;

    // The first frame after the server preface must be the client's SETTINGS.
    let header = frame::header::FrameHeader::decode(remaining)?;

    if header.frame_type != frame::FrameType::Settings {
        return Err(H2Error::Connection(
            ErrorCode::ProtocolError,
            "first frame after server preface must be SETTINGS",
        ));
    }

    let settings =
        frame::SettingsFrame::decode(&header, &remaining[frame::header::FRAME_HEADER_LEN..])?;

    Ok((remaining, settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_preface_accepted() {
        let remaining = process_preface(CONNECTION_PREFACE).unwrap();
        assert!(remaining.is_empty());
    }

    #[test]
    fn valid_preface_with_additional_bytes() {
        let mut preface_bytes = vec![];
        preface_bytes.extend_from_slice(CONNECTION_PREFACE);
        preface_bytes.extend_from_slice(b"\x00\x00\x00\x04\x00\x00\x00\x00\x00"); // empty SETTINGS frame
        let remaining = process_preface(&preface_bytes).unwrap();
        assert_eq!(remaining.len(), 9);
    }

    #[test]
    fn truncated_preface_rejected() {
        assert!(process_preface(&[0x50]).is_err());
    }

    #[test]
    fn wrong_preface_rejected() {
        let wrong = b"PRI * HTTP/1.1\r\n\r\nSM\r\n\r\n";
        assert!(process_preface(wrong).is_err());
    }
}
