//! Framing: every message on the socket is a `u16` big-endian length followed by that many
//! bytes of FlatBuffers payload.

use std::io::{self, Read};

use crate::Error;

/// `payload` preceded by its `u16` big-endian length.
pub fn frame(payload: &[u8]) -> Result<Vec<u8>, Error> {
    let len = u16::try_from(payload.len()).map_err(|_| Error::FrameTooLarge(payload.len()))?;
    let mut out = Vec::with_capacity(2 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// Reads one frame from `reader` into `payload` (cleared first). Blocks until the whole frame
/// has arrived; an end of stream part-way through is [`io::ErrorKind::UnexpectedEof`].
pub fn read_frame<R: Read>(reader: &mut R, payload: &mut Vec<u8>) -> io::Result<()> {
    let mut len = [0u8; 2];
    reader.read_exact(&mut len)?;
    payload.clear();
    payload.resize(usize::from(u16::from_be_bytes(len)), 0);
    reader.read_exact(payload)
}

/// Splits a byte stream that arrives in arbitrary pieces (a non-blocking socket) into frames.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    pending: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> FrameDecoder {
        FrameDecoder::default()
    }

    /// Adds bytes just read from the stream.
    pub fn push(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
    }

    /// The next complete payload, if the bytes pushed so far hold one.
    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        let len = usize::from(u16::from_be_bytes([
            *self.pending.first()?,
            *self.pending.get(1)?,
        ]));
        if self.pending.len() < 2 + len {
            return None;
        }
        let rest = self.pending.split_off(2 + len);
        let mut frame = std::mem::replace(&mut self.pending, rest);
        frame.drain(..2);
        Some(frame)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips_and_an_oversize_payload_is_rejected() {
        let framed = frame(&[1, 2, 3]).unwrap();
        assert_eq!(framed, [0, 3, 1, 2, 3]);
        let mut out = vec![9; 10];
        read_frame(&mut framed.as_slice(), &mut out).unwrap();
        assert_eq!(out, [1, 2, 3]);
        assert_eq!(
            frame(&vec![0; 65_536]).err(),
            Some(Error::FrameTooLarge(65_536))
        );
        assert_eq!(frame(&vec![0; 65_535]).unwrap().len(), 65_537);
    }

    #[test]
    fn a_truncated_frame_is_an_eof_not_a_hang() {
        let mut out = Vec::new();
        let e = read_frame(&mut [0u8, 5, 1, 2].as_slice(), &mut out).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
        let e = read_frame(&mut [0u8].as_slice(), &mut out).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn the_decoder_reassembles_frames_split_and_joined_arbitrarily() {
        let mut stream = frame(&[1, 2, 3]).unwrap();
        stream.extend(frame(&[]).unwrap());
        stream.extend(frame(&[7; 300]).unwrap());
        for chunk in [1usize, 2, 5, 1000] {
            let mut d = FrameDecoder::new();
            let mut got = Vec::new();
            for piece in stream.chunks(chunk) {
                d.push(piece);
                while let Some(f) = d.next_frame() {
                    got.push(f);
                }
            }
            assert_eq!(got, [vec![1, 2, 3], vec![], vec![7; 300]], "chunk {chunk}");
        }
        assert_eq!(FrameDecoder::new().next_frame(), None);
    }
}
