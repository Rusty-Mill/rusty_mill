use crate::error::{ErrorCode, H2Error, Result};
use crate::frame::header::{Flags, FrameHeader};

/// Strip the optional pad-length prefix and trailing padding from a frame
/// payload (used by DATA, HEADERS and PUSH_PROMISE; RFC 9113 §6.1/§6.2/§6.6).
///
/// Returns the unpadded body slice and the pad length that was removed.
pub fn strip_padding<'a>(header: &FrameHeader, payload: &'a [u8]) -> Result<(&'a [u8], u8)> {
    if !header.flags.contains(Flags::PADDED) {
        return Ok((payload, 0));
    }
    if payload.is_empty() {
        return Err(H2Error::Connection(
            ErrorCode::ProtocolError,
            "PADDED flag set but no pad length octet present",
        ));
    }
    let pad_len = payload[0] as usize;
    let body = &payload[1..];
    if pad_len > body.len() {
        return Err(H2Error::Connection(
            ErrorCode::ProtocolError,
            "padding length exceeds frame payload",
        ));
    }
    Ok((&body[..body.len() - pad_len], pad_len as u8))
}

/// Build a padded payload: `[pad_len][body][pad_len zero bytes]`.
pub fn add_padding(out: &mut Vec<u8>, body: &[u8], pad_len: u8) {
    out.push(pad_len);
    out.extend_from_slice(body);
    out.extend(std::iter::repeat_n(0u8, pad_len as usize));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::header::FrameType;

    #[test]
    fn no_padding_flag_returns_whole_payload() {
        let header = FrameHeader::new(3, FrameType::Data, Flags::NONE, 1);
        let (body, pad) = strip_padding(&header, b"abc").unwrap();
        assert_eq!(body, b"abc");
        assert_eq!(pad, 0);
    }

    #[test]
    fn padded_roundtrip() {
        let mut buf = Vec::new();
        add_padding(&mut buf, b"abc", 4);
        let header = FrameHeader::new(buf.len() as u32, FrameType::Data, Flags::PADDED, 1);
        let (body, pad) = strip_padding(&header, &buf).unwrap();
        assert_eq!(body, b"abc");
        assert_eq!(pad, 4);
    }

    #[test]
    fn pad_length_too_large_errors() {
        let payload = [5u8, b'a', b'b'];
        let header = FrameHeader::new(payload.len() as u32, FrameType::Data, Flags::PADDED, 1);
        assert!(strip_padding(&header, &payload).is_err());
    }
}
