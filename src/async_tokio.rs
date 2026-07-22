//! The async adapter (feature `rusty-tokio`): the same shape as
//! [`crate::sync`]'s [`crate::sync::SyncTransport`], but driving the
//! sans-IO core over `rusty_tokio`'s `AsyncRead`/`AsyncWrite` instead of
//! a blocking `Read + Write`. Per the mission this crate implements, the
//! async adapter lives here, behind this feature -- `rusty_tokio` itself
//! stays HTTP-free, mirroring how `rusty_tls`'s own async adapter is laid
//! out.
//!
//! Deliberately not shared code with [`crate::sync`]: Rust has no good
//! sync/async code-sharing story without a proc-macro crate or a
//! maybe-async abstraction, and `rusty_tls` already established the
//! ecosystem's answer to that -- two parallel, independently-readable
//! adapter modules, not one generic-over-blocking-ness one.

use rusty_tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::body::{self, ChunkedDecoder, Framing, Progress, DEFAULT_MAX_LINE_LEN};
use crate::head::{self, Outcome, RequestHead, ResponseHead};
use crate::transport::{unexpected_eof, Result};

const INITIAL_BUF_LEN: usize = 4096;

/// Drives [`crate::head`] and [`crate::body`] over a `T: AsyncRead +
/// AsyncWrite + Unpin + Send`. See [`crate::sync::SyncTransport`] for the
/// buffering contract -- identical here, just `.await`ed.
pub struct AsyncTransport<T> {
    io: T,
    buf: Vec<u8>,
    /// `buf[start..end]` is unconsumed, already-received data.
    start: usize,
    end: usize,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncTransport<T> {
    /// Wraps an already-connected transport. Performs no I/O itself.
    pub fn new(io: T) -> Self {
        AsyncTransport {
            io,
            buf: vec![0u8; INITIAL_BUF_LEN],
            start: 0,
            end: 0,
        }
    }

    /// Borrows the underlying transport.
    pub fn get_ref(&self) -> &T {
        &self.io
    }

    /// Consumes `self`, returning the underlying transport. See
    /// [`crate::sync::SyncTransport::into_inner`]'s docs -- the same
    /// caution about unconsumed buffered bytes applies.
    pub fn into_inner(self) -> T {
        self.io
    }

    async fn fill_more(&mut self) -> Result<usize> {
        if self.start > 0 {
            self.buf.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
        if self.end == self.buf.len() {
            self.buf.resize(self.buf.len() * 2, 0);
        }
        let n = self.io.read(&mut self.buf[self.end..]).await?;
        self.end += n;
        Ok(n)
    }

    /// Reads and parses a request head. See
    /// [`crate::sync::SyncTransport::read_request_head`].
    pub async fn read_request_head(&mut self, max_head_len: usize) -> Result<RequestHead> {
        loop {
            match head::parse_request_head(&self.buf[self.start..self.end], max_head_len)? {
                Outcome::Complete { head, consumed } => {
                    self.start += consumed;
                    return Ok(head);
                }
                Outcome::Incomplete => {
                    if self.fill_more().await? == 0 {
                        return Err(unexpected_eof(
                            "connection closed before the request head completed",
                        ));
                    }
                }
            }
        }
    }

    /// Reads and parses a response head. See
    /// [`crate::sync::SyncTransport::read_response_head`].
    pub async fn read_response_head(&mut self, max_head_len: usize) -> Result<ResponseHead> {
        loop {
            match head::parse_response_head(&self.buf[self.start..self.end], max_head_len)? {
                Outcome::Complete { head, consumed } => {
                    self.start += consumed;
                    return Ok(head);
                }
                Outcome::Incomplete => {
                    if self.fill_more().await? == 0 {
                        return Err(unexpected_eof(
                            "connection closed before the response head completed",
                        ));
                    }
                }
            }
        }
    }

    /// Serializes and writes a request head.
    pub async fn write_request_head(&mut self, head: &RequestHead) -> Result<()> {
        let mut out = Vec::new();
        head.write(&mut out);
        self.io.write_all(&out).await?;
        Ok(())
    }

    /// Serializes and writes a response head.
    pub async fn write_response_head(&mut self, head: &ResponseHead) -> Result<()> {
        let mut out = Vec::new();
        head.write(&mut out);
        self.io.write_all(&out).await?;
        Ok(())
    }

    /// Reads a whole body into memory according to `framing`. See
    /// [`crate::sync::SyncTransport::read_body`].
    pub async fn read_body(&mut self, framing: Framing) -> Result<Vec<u8>> {
        match framing {
            Framing::None => Ok(Vec::new()),
            Framing::ContentLength(len) => self.read_content_length_body(len).await,
            Framing::Close => self.read_close_delimited_body().await,
            Framing::Chunked => self.read_chunked_body(DEFAULT_MAX_LINE_LEN).await,
        }
    }

    async fn read_content_length_body(&mut self, len: u64) -> Result<Vec<u8>> {
        let cap = usize::try_from(len).unwrap_or(usize::MAX).min(1 << 20);
        let mut out = Vec::with_capacity(cap);
        while (out.len() as u64) < len {
            if self.start == self.end && self.fill_more().await? == 0 {
                return Err(unexpected_eof(
                    "connection closed before the full body arrived",
                ));
            }
            let remaining = len - out.len() as u64;
            let take = remaining.min((self.end - self.start) as u64) as usize;
            out.extend_from_slice(&self.buf[self.start..self.start + take]);
            self.start += take;
        }
        Ok(out)
    }

    async fn read_close_delimited_body(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            if self.start < self.end {
                out.extend_from_slice(&self.buf[self.start..self.end]);
                self.start = self.end;
            }
            if self.fill_more().await? == 0 {
                return Ok(out);
            }
        }
    }

    /// Reads a `Transfer-Encoding: chunked` body into memory. See
    /// [`crate::sync::SyncTransport::read_chunked_body`].
    pub async fn read_chunked_body(&mut self, max_line_len: usize) -> Result<Vec<u8>> {
        let mut decoder = ChunkedDecoder::new();
        let mut out = Vec::new();
        loop {
            match decoder.advance(&self.buf[self.start..self.end], max_line_len)? {
                Progress::Incomplete => {
                    if self.fill_more().await? == 0 {
                        return Err(unexpected_eof(
                            "connection closed before the chunked body completed",
                        ));
                    }
                }
                Progress::Framing { consumed } => self.start += consumed,
                Progress::Data { len } => {
                    out.extend_from_slice(&self.buf[self.start..self.start + len]);
                    self.start += len;
                }
                Progress::Done { consumed } => {
                    self.start += consumed;
                    return Ok(out);
                }
            }
        }
    }

    /// Writes `body` verbatim.
    pub async fn write_body(&mut self, body: &[u8]) -> Result<()> {
        self.io.write_all(body).await?;
        Ok(())
    }

    /// Writes one `Transfer-Encoding: chunked` chunk.
    pub async fn write_chunk(&mut self, data: &[u8]) -> Result<()> {
        let mut out = Vec::new();
        body::write_chunk(&mut out, data);
        self.io.write_all(&out).await?;
        Ok(())
    }

    /// Writes the terminating zero-size chunk, ending a chunked body.
    pub async fn write_chunked_end(&mut self) -> Result<()> {
        let mut out = Vec::new();
        body::write_chunked_end(&mut out);
        self.io.write_all(&out).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::HeaderMap;
    use crate::method::Method;
    use crate::status::StatusCode;
    use crate::version::Version;
    use rusty_tokio::io::duplex;

    /// Feeds `wire` through one half of an in-memory duplex pair and
    /// returns the other half, already primed for `AsyncTransport` to
    /// read from -- enough to exercise the adapter without a real
    /// socket, using `rusty_tokio`'s own reactor and executor.
    async fn wired_with(wire: &'static [u8]) -> impl AsyncRead + AsyncWrite + Unpin + Send {
        let (mut feeder, reader) = duplex(wire.len().max(64));
        feeder.write_all(wire).await.unwrap();
        drop(feeder); // signals EOF to `reader` once its buffered data is drained
        reader
    }

    #[rusty_tokio::test]
    async fn reads_request_head_then_content_length_body() {
        let io = wired_with(b"POST /a HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhello").await;
        let mut t = AsyncTransport::new(io);
        let head = t.read_request_head(8192).await.unwrap();
        assert_eq!(head.method, Method::Post);
        assert_eq!(head.target, "/a");
        let framing = body::request_framing(&head.headers).unwrap();
        let body = t.read_body(framing).await.unwrap();
        assert_eq!(body, b"hello");
    }

    #[rusty_tokio::test]
    async fn reads_response_head_then_chunked_body() {
        let io = wired_with(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n",
        )
        .await;
        let mut t = AsyncTransport::new(io);
        let head = t.read_response_head(8192).await.unwrap();
        let framing =
            body::response_framing(&head.headers, &Method::Get, StatusCode::from_u16(200)).unwrap();
        let body = t.read_body(framing).await.unwrap();
        assert_eq!(body, b"hello");
    }

    #[rusty_tokio::test]
    async fn reads_close_delimited_body_to_eof() {
        let io = wired_with(b"HTTP/1.0 200 OK\r\n\r\nwhatever is left").await;
        let mut t = AsyncTransport::new(io);
        let head = t.read_response_head(8192).await.unwrap();
        assert_eq!(head.version, Version::Http10);
        let framing =
            body::response_framing(&head.headers, &Method::Get, StatusCode::from_u16(200)).unwrap();
        assert_eq!(framing, Framing::Close);
        let body = t.read_body(framing).await.unwrap();
        assert_eq!(body, b"whatever is left");
    }

    #[rusty_tokio::test]
    async fn writes_request_head() {
        let (feeder, mut sink) = duplex(256);
        let mut t = AsyncTransport::new(feeder);
        let mut headers = HeaderMap::new();
        headers.insert("Host", "example.com").unwrap();
        let head = RequestHead {
            method: Method::Get,
            target: "/a".to_string(),
            version: Version::Http11,
            headers,
        };
        t.write_request_head(&head).await.unwrap();
        drop(t);
        let mut got = Vec::new();
        sink.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, b"GET /a HTTP/1.1\r\nHost: example.com\r\n\r\n");
    }

    #[rusty_tokio::test]
    async fn writes_chunked_body() {
        let (feeder, mut sink) = duplex(256);
        let mut t = AsyncTransport::new(feeder);
        t.write_chunk(b"hello").await.unwrap();
        t.write_chunk(b"").await.unwrap(); // no-op, not an end marker
        t.write_chunked_end().await.unwrap();
        drop(t);
        let mut got = Vec::new();
        sink.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, b"5\r\nhello\r\n0\r\n\r\n");
    }

    #[rusty_tokio::test]
    async fn errors_on_truncated_content_length_body() {
        let io = wired_with(b"POST /a HTTP/1.1\r\nContent-Length: 10\r\n\r\nshort").await;
        let mut t = AsyncTransport::new(io);
        let head = t.read_request_head(8192).await.unwrap();
        let framing = body::request_framing(&head.headers).unwrap();
        assert!(t.read_body(framing).await.is_err());
    }
}
