//! A blocking framed connection to the server.

use std::io::{self, Read, Write};

use rusty_mux_proto::{Message, decode, encode};

use crate::stop::{Stop, is_wait};

/// A connection whose reads and writes give up when `stop` says so; the
/// stream must time out (see [`crate::stop::POLL`]) for a silent server to
/// be noticed.
pub(crate) struct Conn<S> {
    stream: S,
    buf: Vec<u8>,
    stop: Stop,
}

impl<S: Read + Write> Conn<S> {
    pub(crate) fn new(stream: S, stop: Stop) -> Self {
        Self {
            stream,
            buf: Vec::new(),
            stop,
        }
    }

    pub(crate) fn send(&mut self, message: &Message) -> io::Result<()> {
        let frame = encode(message).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let mut rest = frame.as_slice();
        while !rest.is_empty() {
            self.stop.check()?;
            match self.stream.write(rest) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => rest = &rest[n..],
                Err(e) if is_wait(&e) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// The next message; `UnexpectedEof` when the server closes,
    /// `InvalidData` for a frame that does not decode, and the
    /// [`Stop::check`] error once stopped.
    pub(crate) fn recv(&mut self) -> io::Result<Message> {
        let mut chunk = [0u8; 65536];
        loop {
            let decoded =
                decode(&self.buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if let Some((message, used)) = decoded {
                self.buf.drain(..used);
                return Ok(message);
            }
            self.stop.check()?;
            let n = match self.stream.read(&mut chunk) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => n,
                Err(e) if is_wait(&e) => continue,
                Err(e) => return Err(e),
            };
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}
