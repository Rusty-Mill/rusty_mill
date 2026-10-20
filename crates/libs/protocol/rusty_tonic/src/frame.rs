use rusty_prost::Message;

use crate::Status;

/// Bytes before each message: a compression flag and a big-endian `u32` length.
pub const HEADER_LEN: usize = 5;

/// The default limit on a received message, 4 MiB.
pub const DEFAULT_MAX_RECV_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

/// One message with its 5-byte prefix, uncompressed.
pub fn encode_frame(message: &[u8]) -> Result<Vec<u8>, Status> {
    let len = u32::try_from(message.len())
        .map_err(|_| Status::resource_exhausted("message is too large to send"))?;
    let mut out = Vec::with_capacity(HEADER_LEN + message.len());
    out.push(0);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(message);
    Ok(out)
}

/// The encoded `m` as a frame.
pub fn encode_message<M: Message>(m: &M) -> Result<Vec<u8>, Status> {
    let payload = m
        .encode_to_vec()
        .map_err(|e| Status::internal(format!("failed to encode message: {e}")))?;
    encode_frame(&payload)
}

/// Decodes a message payload (without its prefix). A payload that does not decode is
/// `INTERNAL`, as in `tonic`.
pub fn decode_message<M: Message>(payload: &[u8]) -> Result<M, Status> {
    M::decode(payload).map_err(|e| Status::internal(format!("failed to decode message: {e}")))
}

/// Splits a byte stream into messages, however the bytes arrive.
///
/// After the first error it keeps returning that error and ignores what is pushed, so the
/// memory it holds stays bounded by the limit.
#[derive(Debug)]
pub struct FrameDecoder {
    buf: Vec<u8>,
    pos: usize,
    ingest_pos: usize,
    ingest_frame_end: Option<usize>,
    max: usize,
    failed: Option<Status>,
}

impl FrameDecoder {
    /// A decoder that refuses a message longer than `max` bytes.
    pub fn new(max: usize) -> FrameDecoder {
        FrameDecoder {
            buf: Vec::new(),
            pos: 0,
            ingest_pos: 0,
            ingest_frame_end: None,
            max,
            failed: None,
        }
    }

    /// Adds received bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.failed.is_some() {
            return;
        }
        if let Err(status) = self.ingest(bytes) {
            self.fail(status);
        }
    }

    /// The next whole message, `Ok(None)` if more bytes are needed.
    pub fn next_message(&mut self) -> Result<Option<Vec<u8>>, Status> {
        if let Some(status) = &self.failed {
            return Err(status.clone());
        }
        self.take().map_err(|status| {
            self.fail(status.clone());
            status
        })
    }

    fn ingest(&mut self, mut bytes: &[u8]) -> Result<(), Status> {
        while !bytes.is_empty() {
            if let Some(end) = self.ingest_frame_end {
                let count = bytes.len().min(end - self.buf.len());
                self.buf.extend_from_slice(&bytes[..count]);
                bytes = &bytes[count..];
                if self.buf.len() < end {
                    return Ok(());
                }
                self.ingest_pos = end;
                self.ingest_frame_end = None;
                continue;
            }

            let prefix_needed = HEADER_LEN - (self.buf.len() - self.ingest_pos);
            let count = bytes.len().min(prefix_needed);
            self.buf.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.buf.len() - self.ingest_pos < HEADER_LEN {
                return Ok(());
            }

            let total = checked_frame_len(
                self.buf[self.ingest_pos],
                self.buf[self.ingest_pos + 1..self.ingest_pos + HEADER_LEN]
                    .try_into()
                    .expect("four-byte length slice"),
                self.max,
            )?;
            let end = self
                .ingest_pos
                .checked_add(total)
                .ok_or_else(length_overflow)?;
            self.ingest_frame_end = Some(end);
            if end == self.buf.len() {
                self.ingest_pos = end;
                self.ingest_frame_end = None;
            }
        }
        Ok(())
    }

    fn fail(&mut self, status: Status) {
        self.failed = Some(status);
        self.buf.clear();
        self.pos = 0;
        self.ingest_pos = 0;
        self.ingest_frame_end = None;
    }

    /// The next message if its prefix and body are all here. The prefix is checked as soon as
    /// it arrives, so a length over the limit is refused before any body is waited for.
    fn take(&mut self) -> Result<Option<Vec<u8>>, Status> {
        let avail = self.buf.get(self.pos..).unwrap_or_default();
        let [flag, a, b, c, d, ..] = *avail else {
            return Ok(None);
        };
        let total = checked_frame_len(flag, [a, b, c, d], self.max)?;
        let Some(body) = avail.get(HEADER_LEN..total) else {
            return Ok(None);
        };
        let message = body.to_vec();
        self.pos += total;
        self.compact();
        Ok(Some(message))
    }

    /// Drops consumed bytes: all of them when nothing is left, otherwise once they are at
    /// least half the buffer, so taking messages off the front stays linear overall.
    fn compact(&mut self) {
        if self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
            self.ingest_pos = 0;
            self.ingest_frame_end = None;
        } else if self.pos >= 4096 && self.pos * 2 >= self.buf.len() {
            self.ingest_pos -= self.pos;
            self.ingest_frame_end = self.ingest_frame_end.map(|end| end - self.pos);
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
    }

    /// Call when the stream ends: bytes left over mean a message was cut short.
    pub fn finish(&self) -> Result<(), Status> {
        if let Some(status) = &self.failed {
            return Err(status.clone());
        }
        if self.buffered() > 0 {
            return Err(Status::internal(
                "the stream ended in the middle of a message",
            ));
        }
        Ok(())
    }

    /// Bytes held that are not yet part of a returned message.
    pub fn buffered(&self) -> usize {
        self.buf.len() - self.pos
    }
}

fn length_overflow() -> Status {
    Status::resource_exhausted("message length does not fit in memory")
}

fn checked_frame_len(flag: u8, length: [u8; 4], max: usize) -> Result<usize, Status> {
    match flag {
        0 => {}
        1 => {
            return Err(Status::unimplemented(
                "compressed messages are not supported",
            ))
        }
        _ => {
            return Err(Status::internal(
                "invalid compression flag in a message prefix",
            ))
        }
    }
    let len = usize::try_from(u32::from_be_bytes(length)).map_err(|_| length_overflow())?;
    if len > max {
        return Err(Status::resource_exhausted(format!(
            "message of {len} bytes exceeds the limit of {max} bytes"
        )));
    }
    HEADER_LEN.checked_add(len).ok_or_else(length_overflow)
}

impl Default for FrameDecoder {
    fn default() -> FrameDecoder {
        FrameDecoder::new(DEFAULT_MAX_RECV_MESSAGE_SIZE)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// With a partial frame always pending the buffer never empties, so consumed bytes must be
    /// dropped by compaction, or a long stream would hold all of it.
    #[test]
    fn a_long_stream_with_a_partial_frame_pending_holds_a_bounded_buffer() {
        let mut d = FrameDecoder::default();
        let frame = encode_frame(&[7u8; 100]).unwrap();
        d.push(&frame[..3]); // the start of the first frame
        let mut most = 0;
        for _ in 0..20_000 {
            // finish the frame in flight and start the next one
            d.push(&frame[3..]);
            d.push(&frame[..3]);
            assert!(d.next_message().unwrap().is_some());
            most = most.max(d.buf.len());
        }
        assert!(most < 16 * 1024, "buffer grew to {most} bytes");
        assert_eq!(d.buffered(), 3);
    }
}
