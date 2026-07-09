//! Noise transport: record framing + per-direction AEAD, mirroring Go
//! `control/controlbase/conn.go` (see PROTOCOL.md).
//!
//! Records are `[1B type=0x04][2B BE ciphertext-len][ciphertext]`, AEAD is
//! ChaCha20-Poly1305 with empty AAD and a nonce of 4 zero bytes plus an
//! 8-byte big-endian counter (Tailscale deviates from the Noise spec's
//! little-endian here). Implements tokio `AsyncRead`/`AsyncWrite` so an
//! HTTP/2 client connection can run directly on top.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::handshake::{MSG_TYPE_RECORD, SessionKeys};

/// Maximum size of a record on the wire, including the 3-byte header.
const MAX_MESSAGE_SIZE: usize = 4096;
const HEADER_LEN: usize = 3;
const TAG_LEN: usize = 16;
/// Maximum plaintext one record can carry.
const MAX_PLAINTEXT_SIZE: usize = MAX_MESSAGE_SIZE - HEADER_LEN - TAG_LEN;

/// One direction's cipher state: key + monotonically increasing counter.
struct DirectionState {
    cipher: Option<ChaCha20Poly1305>, // None once poisoned
    counter: u64,
}

impl DirectionState {
    fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: Some(ChaCha20Poly1305::new(Key::from_slice(key))),
            counter: 0,
        }
    }
}

/// A secured Noise connection over `T`.
pub struct Conn<T> {
    inner: T,
    tx: DirectionState,
    rx: DirectionState,
    handshake_hash: [u8; 32],

    // Read side: accumulated ciphertext, then decrypted plaintext.
    read_buf: Vec<u8>,
    plaintext: Vec<u8>,
    plaintext_off: usize,

    // Write side: at most one encrypted record awaiting flush.
    write_buf: Vec<u8>,
    write_off: usize,
}

impl<T> Conn<T> {
    pub fn new(inner: T, keys: SessionKeys) -> Self {
        Self {
            inner,
            tx: DirectionState::new(&keys.tx),
            rx: DirectionState::new(&keys.rx),
            handshake_hash: keys.handshake_hash,
            read_buf: Vec::new(),
            plaintext: Vec::new(),
            plaintext_off: 0,
            write_buf: Vec::new(),
            write_off: 0,
        }
    }

    /// The Noise handshake hash, for channel binding.
    pub fn handshake_hash(&self) -> [u8; 32] {
        self.handshake_hash
    }

    /// Decrypts the first complete record in `read_buf`, if any, appending
    /// plaintext. Returns true if a record was consumed.
    fn decrypt_one(&mut self) -> io::Result<bool> {
        if self.read_buf.len() < HEADER_LEN {
            return Ok(false);
        }
        let msg_type = self.read_buf[0];
        let ct_len = u16::from_be_bytes([self.read_buf[1], self.read_buf[2]]) as usize;
        let total = HEADER_LEN + ct_len;
        if total > MAX_MESSAGE_SIZE {
            self.rx.cipher = None;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("noise record of {total} bytes exceeds maximum"),
            ));
        }
        if msg_type != MSG_TYPE_RECORD {
            self.rx.cipher = None;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected noise message type {msg_type}"),
            ));
        }
        if self.read_buf.len() < total {
            return Ok(false);
        }

        let Some(cipher) = self.rx.cipher.as_ref() else {
            return Err(io::Error::other("noise rx state poisoned"));
        };
        let nonce = {
            // next_nonce borrows mutably; compute before using cipher ref.
            let mut nonce = [0u8; 12];
            nonce[4..].copy_from_slice(&self.rx.counter.to_be_bytes());
            Nonce::from(nonce)
        };
        let pt = cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &self.read_buf[HEADER_LEN..total],
                    aad: &[],
                },
            )
            .map_err(|_| {
                // Once a decrypt fails we are desynchronized; poison rx.
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "noise record failed to authenticate",
                )
            });
        let pt = match pt {
            Ok(pt) => pt,
            Err(e) => {
                self.rx.cipher = None;
                return Err(e);
            }
        };
        self.rx.counter += 1;
        if self.rx.counter == u64::MAX {
            self.rx.cipher = None;
        }
        self.read_buf.drain(..total);
        if self.plaintext_off == self.plaintext.len() {
            self.plaintext.clear();
            self.plaintext_off = 0;
        }
        self.plaintext.extend_from_slice(&pt);
        Ok(true)
    }

    /// Encrypts one plaintext chunk into `write_buf` as a complete record.
    fn encrypt_record(&mut self, plaintext: &[u8]) -> io::Result<()> {
        debug_assert!(plaintext.len() <= MAX_PLAINTEXT_SIZE);
        debug_assert!(self.write_off == self.write_buf.len());
        let Some(cipher) = self.tx.cipher.as_ref() else {
            return Err(io::Error::other("noise tx state poisoned"));
        };
        let nonce = {
            let mut nonce = [0u8; 12];
            nonce[4..].copy_from_slice(&self.tx.counter.to_be_bytes());
            Nonce::from(nonce)
        };
        let ct = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: &[],
                },
            )
            .expect("encryption is infallible for in-memory buffers");
        self.tx.counter += 1;
        if self.tx.counter == u64::MAX {
            self.tx.cipher = None;
        }
        self.write_buf.clear();
        self.write_off = 0;
        self.write_buf.push(MSG_TYPE_RECORD);
        self.write_buf
            .extend_from_slice(&(ct.len() as u16).to_be_bytes());
        self.write_buf.extend_from_slice(&ct);
        Ok(())
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Conn<T> {
    /// Flushes any buffered ciphertext to the inner writer.
    fn poll_flush_write_buf(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.write_off < self.write_buf.len() {
            let n = ready!(
                Pin::new(&mut self.inner).poll_write(cx, &self.write_buf[self.write_off..])
            )?;
            if n == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            self.write_off += n;
        }
        self.write_buf.clear();
        self.write_off = 0;
        Poll::Ready(Ok(()))
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncRead for Conn<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            // Serve buffered plaintext first.
            let avail = &this.plaintext[this.plaintext_off..];
            if !avail.is_empty() {
                let n = avail.len().min(buf.remaining());
                buf.put_slice(&avail[..n]);
                this.plaintext_off += n;
                return Poll::Ready(Ok(()));
            }

            // Decrypt any complete buffered records (zero-length records
            // produce no plaintext, hence the loop).
            if this.decrypt_one()? {
                continue;
            }

            // Need more ciphertext.
            let mut tmp = [0u8; MAX_MESSAGE_SIZE];
            let mut tmp_buf = ReadBuf::new(&mut tmp);
            ready!(Pin::new(&mut this.inner).poll_read(cx, &mut tmp_buf))?;
            let filled = tmp_buf.filled();
            if filled.is_empty() {
                // EOF from below.
                if this.read_buf.is_empty() {
                    return Poll::Ready(Ok(())); // clean EOF
                }
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF mid noise record",
                )));
            }
            this.read_buf.extend_from_slice(filled);
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncWrite for Conn<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        // Finish flushing the previous record before accepting more input,
        // so we never buffer unboundedly.
        ready!(this.poll_flush_write_buf(cx))?;

        let chunk = &buf[..buf.len().min(MAX_PLAINTEXT_SIZE)];
        this.encrypt_record(chunk)?;
        // Opportunistically try to flush; if the inner writer is not ready
        // the record stays buffered and the next write/flush finishes it.
        let _ = this.poll_flush_write_buf(cx)?;
        Poll::Ready(Ok(chunk.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_flush_write_buf(cx))?;
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_flush_write_buf(cx))?;
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controlbase::handshake::SessionKeys;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn pair() -> (Conn<tokio::io::DuplexStream>, Conn<tokio::io::DuplexStream>) {
        let (a, b) = tokio::io::duplex(64 * 1024);
        let k1 = [1u8; 32];
        let k2 = [2u8; 32];
        let client = Conn::new(
            a,
            SessionKeys {
                tx: k1,
                rx: k2,
                handshake_hash: [0; 32],
            },
        );
        // The "server" has the directions swapped.
        let server = Conn::new(
            b,
            SessionKeys {
                tx: k2,
                rx: k1,
                handshake_hash: [0; 32],
            },
        );
        (client, server)
    }

    #[tokio::test]
    async fn round_trip_small_and_large() {
        let (mut client, mut server) = pair();
        // Larger than one record to exercise chunking.
        let payload: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let expected = payload.clone();

        let writer = tokio::spawn(async move {
            client.write_all(&payload).await.unwrap();
            client.flush().await.unwrap();
            client
        });
        let mut got = vec![0u8; expected.len()];
        server.read_exact(&mut got).await.unwrap();
        assert_eq!(got, expected);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn tampered_record_poisons_connection() {
        let (mut client, server) = pair();
        client.write_all(b"hello").await.unwrap();
        client.flush().await.unwrap();

        // Rip the framing apart: corrupt a ciphertext byte in transit.
        let mut raw = server.inner;
        let mut frame = vec![0u8; HEADER_LEN + 5 + TAG_LEN];
        raw.read_exact(&mut frame).await.unwrap();
        frame[HEADER_LEN] ^= 0xff;
        let mut server = Conn::new(
            raw,
            SessionKeys {
                tx: [2u8; 32],
                rx: [1u8; 32],
                handshake_hash: [0; 32],
            },
        );
        server.read_buf.extend_from_slice(&frame);
        let mut out = [0u8; 5];
        let err = server.read_exact(&mut out).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        // Subsequent reads keep failing (poisoned).
        assert!(server.read_exact(&mut out).await.is_err());
    }

    #[tokio::test]
    async fn oversize_record_rejected() {
        let (client, mut server) = pair();
        let mut raw_client = client.inner;
        // type=4, length 0xffff → total 65538 > 4096.
        raw_client
            .write_all(&[MSG_TYPE_RECORD, 0xff, 0xff])
            .await
            .unwrap();
        raw_client.flush().await.unwrap();
        let mut out = [0u8; 1];
        let err = server.read_exact(&mut out).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn zero_length_records_are_skipped() {
        let (mut client, mut server) = pair();
        // Manually produce an empty record followed by data.
        client.encrypt_record(&[]).unwrap();
        let empty_frame = client.write_buf.clone();
        client.write_buf.clear();
        client.inner.write_all(&empty_frame).await.unwrap();
        client.write_all(b"after-empty").await.unwrap();
        client.flush().await.unwrap();

        let mut got = [0u8; 11];
        server.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"after-empty");
    }
}
