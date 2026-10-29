//! `bridge`: joins this process's stdin and stdout to the server's socket,
//! byte for byte, so a client elsewhere can reach the server through a
//! command it runs here, such as `ssh host rusty-mux-ctl bridge`. The server
//! checks the bridge's own credentials, as for any local client.

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::path::Path;

use crate::conn;
use crate::stop::Stop;

/// The most read and forwarded at a time.
const CHUNK: usize = 64 * 1024;

/// Bridges stdin and stdout to the server at `socket` until the server
/// closes its side. The end of stdin is passed on as the end of the
/// client's writes, and the server then closes its side too.
pub(crate) fn run(socket: &Path) -> io::Result<()> {
    let stream = conn::connect(socket, &Stop::default())?;
    let writer = stream.try_clone()?;
    // Stays blocked on stdin when the server ends first; exiting ends it.
    std::thread::spawn(move || {
        let mut writer = writer;
        // A failed write means the server is gone, which the read below
        // reports.
        let _ = pump(io::stdin().lock(), &mut writer);
        let _ = writer.shutdown(Shutdown::Write);
    });
    let mut reader = stream;
    pump(&mut reader, io::stdout().lock())
}

/// Copies `from` to `to` until `from` ends, flushing after every read so
/// nothing waits in a buffer: stdout buffers by line, and frames are not
/// lines.
fn pump(mut from: impl Read, mut to: impl Write) -> io::Result<()> {
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let n = match from.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        to.write_all(&chunk[..n])?;
        to.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records each flush's position, and refuses writes once `limit`
    /// bytes are in.
    #[derive(Default)]
    struct Sink {
        bytes: Vec<u8>,
        flushed_at: Vec<usize>,
        limit: Option<usize>,
    }

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.limit.is_some_and(|l| self.bytes.len() >= l) {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushed_at.push(self.bytes.len());
            Ok(())
        }
    }

    /// Hands out its parts one read at a time, then ends.
    struct Reads(Vec<Vec<u8>>);

    impl Read for Reads {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                return Ok(0);
            }
            let part = self.0.remove(0);
            buf[..part.len()].copy_from_slice(&part);
            Ok(part.len())
        }
    }

    #[test]
    fn every_read_is_forwarded_and_flushed_at_once() {
        let mut sink = Sink::default();
        let reads = Reads(vec![b"\x01\x02".to_vec(), b"no newline".to_vec()]);
        pump(reads, &mut sink).unwrap();
        assert_eq!(sink.bytes, b"\x01\x02no newline");
        assert_eq!(sink.flushed_at, [2, 12]);
    }

    #[test]
    fn a_failed_write_stops_the_copy_with_its_error() {
        let mut sink = Sink {
            limit: Some(1),
            ..Sink::default()
        };
        let reads = Reads(vec![b"a".to_vec(), b"b".to_vec()]);
        let err = pump(reads, &mut sink).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }
}
