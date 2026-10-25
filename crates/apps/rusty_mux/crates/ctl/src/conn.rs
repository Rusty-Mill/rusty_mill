//! A blocking framed connection to the server.

use std::io::{self, Read, Write};

use rusty_mux_proto::{Message, decode, encode};

pub(crate) struct Conn<S> {
    stream: S,
    buf: Vec<u8>,
}

impl<S: Read + Write> Conn<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self {
            stream,
            buf: Vec::new(),
        }
    }

    pub(crate) fn send(&mut self, message: &Message) -> io::Result<()> {
        let frame = encode(message).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        self.stream.write_all(&frame)
    }

    /// The next message; `UnexpectedEof` when the server closes, and
    /// `InvalidData` for a frame that does not decode.
    pub(crate) fn recv(&mut self) -> io::Result<Message> {
        let mut chunk = [0u8; 65536];
        loop {
            let decoded =
                decode(&self.buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if let Some((message, used)) = decoded {
                self.buf.drain(..used);
                return Ok(message);
            }
            let n = self.stream.read(&mut chunk)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}
