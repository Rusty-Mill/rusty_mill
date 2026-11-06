//! Blocking frame I/O.

use std::io::{self, Read, Write};

use rusty_mux_proto::{Message, decode, encode};

/// Blocking frame I/O over any byte stream.
pub struct Conn<S> {
    stream: S,
    buf: Vec<u8>,
}

impl<S: Read + Write> Conn<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buf: Vec::new(),
        }
    }

    /// The stream, to set its options.
    pub fn get_ref(&self) -> &S {
        &self.stream
    }

    pub fn send(&mut self, message: &Message) -> io::Result<()> {
        let frame = encode(message).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        self.stream.write_all(&frame)
    }

    /// The next message; `UnexpectedEof` when the server closes.
    pub fn recv(&mut self) -> io::Result<Message> {
        loop {
            if let Some(message) = self.try_recv()? {
                return Ok(message);
            }
        }
    }

    /// The next message if one is buffered or a single read completes
    /// one, else `None`, so a caller can check a deadline between reads
    /// however the bytes trickle in. What a read brings is kept, also when
    /// it fails (a read timeout, say), for the next call.
    pub fn try_recv(&mut self) -> io::Result<Option<Message>> {
        if let Some(message) = self.decoded()? {
            return Ok(Some(message));
        }
        let mut chunk = [0u8; 65536];
        let n = self.stream.read(&mut chunk)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.buf.extend_from_slice(&chunk[..n]);
        self.decoded()
    }

    /// Takes the buffered message, if a whole one is in.
    fn decoded(&mut self) -> io::Result<Option<Message>> {
        let decoded =
            decode(&self.buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(decoded.map(|(message, used)| {
            self.buf.drain(..used);
            message
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_mux_core::BlockId;
    use rusty_uuid::Uuid;

    /// Each read is its own step: a frame that arrives in parts is
    /// `None` until its last part, and nothing read is lost.
    #[cfg(unix)]
    #[test]
    fn try_recv_returns_after_each_read() {
        let (mut near, far) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut conn = Conn::new(far);
        let frame = rusty_mux_proto::encode(&Message::Sync { req: 7 }).unwrap();
        let (head, tail) = frame.split_at(3);
        near.write_all(head).unwrap();
        assert!(conn.try_recv().unwrap().is_none());
        near.write_all(tail).unwrap();
        assert!(matches!(
            conn.try_recv().unwrap(),
            Some(Message::Sync { req: 7 })
        ));
    }

    #[test]
    fn conn_reassembles_split_frames() {
        let block = BlockId::from_uuid(Uuid::nil());
        let message = Message::Output {
            block,
            data: b"abc".to_vec(),
        };
        let mut wire = encode(&message).unwrap();
        wire.extend(encode(&Message::Ready { block }).unwrap());
        let mut conn = Conn::new(io::Cursor::new(wire));
        assert_eq!(conn.recv().unwrap(), message);
        assert_eq!(conn.recv().unwrap(), Message::Ready { block });
        assert_eq!(
            conn.recv().unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
