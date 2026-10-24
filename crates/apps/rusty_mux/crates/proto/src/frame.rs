//! Framing: `u32` big-endian length, `u8` type, fields.

use rusty_wire::{Reader, Writer};

use crate::field::Out;
use crate::{Error, Message};

/// The largest length prefix accepted: 16 MiB, room for a full snapshot
/// (about 2 MB for a filled terminal, `docs/projects/mux/CAPABILITIES.md`).
pub const MAX_FRAME: u32 = 16 << 20;

/// Encodes one message as a whole frame. A message whose frame would exceed
/// [`MAX_FRAME`] is refused; the sender splits large output first.
pub fn encode(message: &Message) -> Result<Vec<u8>, Error> {
    let mut w = Writer::new();
    w.write_u32_be(0);
    w.write_u8(0);
    let mut out = Out(w);
    let ty = message.encode_body(&mut out);
    let mut bytes = out.0.into_vec();
    let len = u32::try_from(bytes.len() - 4).unwrap_or(u32::MAX);
    if len > MAX_FRAME {
        return Err(Error::FrameTooLarge(len));
    }
    bytes[..4].copy_from_slice(&len.to_be_bytes());
    bytes[4] = ty;
    Ok(bytes)
}

/// Decodes the frame at the start of `buf`.
///
/// Returns `Ok(None)` until `buf` holds the whole frame, then the message and
/// the number of bytes it used. A length over [`MAX_FRAME`] is rejected as
/// soon as the prefix arrives, before any body is buffered.
pub fn decode(buf: &[u8]) -> Result<Option<(Message, usize)>, Error> {
    let mut r = Reader::new(buf);
    let Ok(len) = r.read_u32_be() else {
        return Ok(None);
    };
    if len > MAX_FRAME {
        return Err(Error::FrameTooLarge(len));
    }
    if len == 0 {
        return Err(Error::EmptyFrame);
    }
    let Ok(frame) = r.read_bytes(len as usize) else {
        return Ok(None);
    };
    let message = Message::decode_body(frame[0], &frame[1..])?;
    Ok(Some((message, 4 + len as usize)))
}

#[cfg(test)]
mod tests;
