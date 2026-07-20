//! Port of `src/comm` — framed TCP messaging.
//!
//! Wire format (byte-compatible with croc v10):
//! `b"croc" || u32-little-endian payload length || payload`.
//!
//! The Go version supports SOCKS5/HTTP proxies via global variables; proxy
//! support is deferred to a later phase (see MIGRATION.md).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const MAGIC_BYTES: &[u8; 4] = b"croc";

/// Matches Go's `maxReadMessageSize` guard against malformed streams.
pub const MAX_READ_MESSAGE_SIZE: u32 = 64 * 1024 * 1024;

/// Matches Go's long deadline "in case waiting for file".
pub const IDLE_READ_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);

/// Matches Go's `messageBodyReadTimeout` for reading a frame body.
pub const MESSAGE_BODY_READ_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// A framed connection. Mirrors `comm.Comm`.
pub struct Comm {
    stream: TcpStream,
}

impl Comm {
    /// Wrap an accepted/established stream, applying croc's default deadlines.
    pub fn new(stream: TcpStream) -> std::io::Result<Self> {
        stream.set_read_timeout(Some(IDLE_READ_TIMEOUT))?;
        stream.set_write_timeout(Some(IDLE_READ_TIMEOUT))?;
        Ok(Comm { stream })
    }

    /// Dial `address` with an optional connect time limit (default 30 s),
    /// mirroring `comm.NewConnection`.
    pub fn new_connection(address: &str, time_limit: Option<Duration>) -> std::io::Result<Self> {
        let limit = time_limit.unwrap_or(Duration::from_secs(30));
        let mut last_err =
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("cannot resolve {address}"));
        for addr in address.to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, limit) {
                Ok(stream) => return Comm::new(stream),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    pub fn stream(&self) -> &TcpStream {
        &self.stream
    }

    /// Clone the underlying socket handle (used by the relay to staple rooms).
    pub fn try_clone(&self) -> std::io::Result<Comm> {
        Ok(Comm {
            stream: self.stream.try_clone()?,
        })
    }

    /// Send one frame: magic, little-endian length, payload — in a single write.
    pub fn send(&mut self, payload: &[u8]) -> std::io::Result<()> {
        let mut frame = Vec::with_capacity(8 + payload.len());
        frame.extend_from_slice(MAGIC_BYTES);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload);
        self.stream.write_all(&frame)?;
        self.stream.flush()
    }

    /// Receive one frame, validating magic and size guard.
    pub fn receive(&mut self) -> std::io::Result<Vec<u8>> {
        self.stream.set_read_timeout(Some(IDLE_READ_TIMEOUT))?;
        let mut magic = [0u8; 4];
        self.stream.read_exact(&mut magic)?;
        if &magic != MAGIC_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("initial bytes are not magic: {:02x?}", magic),
            ));
        }
        let mut len_bytes = [0u8; 4];
        self.stream.read_exact(&mut len_bytes)?;
        let n = u32::from_le_bytes(len_bytes);
        if n > MAX_READ_MESSAGE_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("message too large: {n} > {MAX_READ_MESSAGE_SIZE}"),
            ));
        }
        // Shorten the deadline for the body, like the Go implementation.
        self.stream
            .set_read_timeout(Some(MESSAGE_BODY_READ_TIMEOUT))?;
        let mut buf = vec![0u8; n as usize];
        self.stream.read_exact(&mut buf)?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn frame_round_trip() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut comm = Comm::new(stream).unwrap();
            let got = comm.receive().unwrap();
            comm.send(&got).unwrap();
        });
        let mut comm = Comm::new_connection(&addr.to_string(), None).unwrap();
        comm.send(b"ping frame").unwrap();
        assert_eq!(comm.receive().unwrap(), b"ping frame");
        handle.join().unwrap();
    }

    // Exact frame bytes as croc's Go comm.Write would produce them.
    #[test]
    fn frame_layout_matches_go() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            stream.read_to_end(&mut raw).unwrap();
            raw
        });
        let mut comm = Comm::new_connection(&addr.to_string(), None).unwrap();
        comm.send(b"hi").unwrap();
        drop(comm);
        let raw = handle.join().unwrap();
        assert_eq!(raw, b"croc\x02\x00\x00\x00hi");
    }
}
