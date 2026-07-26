//! Sovereign I/O traits and helpers for rusty_std.

use crate::error::Result;
use alloc::vec::Vec;

/// Standard trait for reading bytes from a source.
pub trait Read {
    /// Reads bytes into `buf` and returns the number of bytes read.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    /// Reads all remaining bytes until EOF into `buf`.
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let mut tmp = [0u8; 1024];
        let mut total = 0;
        loop {
            match self.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    total += n;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(total)
    }
}

/// Standard trait for writing bytes to a destination.
pub trait Write {
    /// Writes bytes from `buf` and returns the number of bytes written.
    fn write(&mut self, buf: &[u8]) -> Result<usize>;

    /// Flushes any buffered bytes.
    fn flush(&mut self) -> Result<()>;

    /// Writes all bytes in `buf` to destination.
    fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.write(buf) {
                Ok(0) => return Err(crate::error::Error::Io(0, alloc::string::String::from("Zero byte write"))),
                Ok(n) => buf = &buf[n..],
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}
