//! A second async adapter (feature `tokio`), alongside
//! [`crate::async_tokio`]'s: the same shape, but driving the sans-IO
//! core over real crates.io `tokio`'s `AsyncRead`/`AsyncWrite` instead of
//! `rusty_tokio`'s -- for a consumer (e.g. `rusty_tail`) built on real
//! tokio rather than this ecosystem's own from-scratch runtime. Adding
//! this wasn't in the original mission handoff's plan (which assumed a
//! sync adapter would cover every non-`rusty_tokio` consumer); it turned
//! out `rusty_tail`'s donor sites are themselves async, over real tokio,
//! which neither the sync nor the `rusty_tokio` adapter fits -- see
//! `ARCHITECTURE.md` for the finding that led here.
//!
//! Deliberately not shared code with [`crate::sync`]/[`crate::async_tokio`]:
//! Rust has no good sync/async (or runtime-to-runtime) code-sharing story
//! without a proc-macro crate or a maybe-async abstraction, and
//! `rusty_tls` already established the ecosystem's answer to the
//! sync/async half of that -- parallel, independently-readable adapter
//! modules, not one generic-over-blocking-ness (or generic-over-runtime)
//! one.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

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

    /// Consumes `self`, returning a [`BodyReader`] that pulls `framing`'s
    /// body incrementally via [`BodyReader::next_chunk`] instead of
    /// buffering it all upfront the way [`Self::read_body`] does -- for a
    /// caller (e.g. a large download) that wants to start processing the
    /// body before it's fully arrived.
    pub fn into_body_reader(self, framing: Framing) -> BodyReader<T> {
        let done = matches!(framing, Framing::None);
        let state = match framing {
            Framing::None => BodyReaderState::None,
            Framing::ContentLength(n) => BodyReaderState::ContentLength(n),
            Framing::Chunked => BodyReaderState::Chunked(ChunkedDecoder::new()),
            Framing::Close => BodyReaderState::Close,
        };
        BodyReader {
            transport: self,
            state,
            done,
        }
    }

    /// Returns up to `max` bytes, doing at most one transport read (none
    /// at all if already-buffered data can satisfy it) -- unlike
    /// [`Self::read_body`]'s eager readers, this deliberately doesn't loop
    /// to fill `max`, so a [`BodyReader`] caller sees data as soon as it's
    /// available rather than however long it takes to accumulate a full
    /// `max`-sized batch. Empty return means EOF.
    async fn read_some(&mut self, max: usize) -> Result<Vec<u8>> {
        if max == 0 {
            return Ok(Vec::new());
        }
        if self.start == self.end && self.fill_more().await? == 0 {
            return Ok(Vec::new());
        }
        let take = max.min(self.end - self.start);
        let data = self.buf[self.start..self.start + take].to_vec();
        self.start += take;
        Ok(data)
    }
}

/// Bytes at a time an incremental [`BodyReader`] hands back per
/// [`BodyReader::next_chunk`] call, at most, for `Close`/`ContentLength`
/// framing -- `Chunked` framing yields whatever the wire's own chunk
/// boundaries are instead.
const READ_CHUNK_SIZE: usize = 8192;

#[derive(Debug, Clone, Copy)]
enum BodyReaderState {
    None,
    ContentLength(u64),
    Chunked(ChunkedDecoder),
    Close,
}

/// A response body not yet (fully) read, produced by
/// [`AsyncTransport::into_body_reader`] and pulled incrementally via
/// [`Self::next_chunk`] instead of requiring the whole thing in memory
/// upfront. Owns the transport (and therefore the connection) for as long
/// as it's alive -- see [`Self::into_inner`] to get it back once done.
pub struct BodyReader<T> {
    transport: AsyncTransport<T>,
    state: BodyReaderState,
    done: bool,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> BodyReader<T> {
    /// The next chunk of body data, or `None` once the body is fully
    /// consumed. Chunk boundaries are an implementation detail (a wire
    /// chunk boundary for `Chunked` framing, or just "however much one
    /// read returned" otherwise) -- never rely on chunk size or count.
    pub async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        if self.done {
            return Ok(None);
        }
        match self.state {
            BodyReaderState::None => {
                self.done = true;
                Ok(None)
            }
            BodyReaderState::Close => {
                let data = self.transport.read_some(READ_CHUNK_SIZE).await?;
                if data.is_empty() {
                    self.done = true;
                    return Ok(None);
                }
                Ok(Some(data))
            }
            BodyReaderState::ContentLength(remaining) => {
                if remaining == 0 {
                    self.done = true;
                    return Ok(None);
                }
                let take = remaining.min(READ_CHUNK_SIZE as u64) as usize;
                let data = self.transport.read_some(take).await?;
                if data.is_empty() {
                    return Err(unexpected_eof(
                        "connection closed before the full body arrived",
                    ));
                }
                self.state = BodyReaderState::ContentLength(remaining - data.len() as u64);
                Ok(Some(data))
            }
            BodyReaderState::Chunked(mut decoder) => loop {
                match decoder.advance(
                    &self.transport.buf[self.transport.start..self.transport.end],
                    DEFAULT_MAX_LINE_LEN,
                )? {
                    Progress::Incomplete => {
                        if self.transport.fill_more().await? == 0 {
                            return Err(unexpected_eof(
                                "connection closed before the chunked body completed",
                            ));
                        }
                    }
                    Progress::Framing { consumed } => {
                        self.transport.start += consumed;
                    }
                    Progress::Data { len } => {
                        let data = self.transport.buf
                            [self.transport.start..self.transport.start + len]
                            .to_vec();
                        self.transport.start += len;
                        self.state = BodyReaderState::Chunked(decoder);
                        return Ok(Some(data));
                    }
                    Progress::Done { consumed } => {
                        self.transport.start += consumed;
                        self.done = true;
                        return Ok(None);
                    }
                }
            },
        }
    }

    /// Consumes `self`, returning the underlying transport.
    pub fn into_inner(self) -> AsyncTransport<T> {
        self.transport
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::HeaderMap;
    use crate::method::Method;
    use crate::status::StatusCode;
    use crate::version::Version;
    use tokio::io::duplex;

    /// Feeds `wire` through one half of an in-memory duplex pair and
    /// returns the other half, already primed for `AsyncTransport` to
    /// read from -- enough to exercise the adapter without a real
    /// socket, using real tokio's own reactor and executor.
    async fn wired_with(wire: &'static [u8]) -> impl AsyncRead + AsyncWrite + Unpin + Send {
        let (mut feeder, reader) = duplex(wire.len().max(64));
        feeder.write_all(wire).await.unwrap();
        drop(feeder); // signals EOF to `reader` once its buffered data is drained
        reader
    }

    #[tokio::test]
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

    #[tokio::test]
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

    #[tokio::test]
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

    #[tokio::test]
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

    #[tokio::test]
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

    #[tokio::test]
    async fn errors_on_truncated_content_length_body() {
        let io = wired_with(b"POST /a HTTP/1.1\r\nContent-Length: 10\r\n\r\nshort").await;
        let mut t = AsyncTransport::new(io);
        let head = t.read_request_head(8192).await.unwrap();
        let framing = body::request_framing(&head.headers).unwrap();
        assert!(t.read_body(framing).await.is_err());
    }

    async fn collect_all(
        mut reader: BodyReader<impl AsyncRead + AsyncWrite + Unpin + Send>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(chunk) = reader.next_chunk().await.unwrap() {
            out.extend_from_slice(&chunk);
        }
        out
    }

    #[tokio::test]
    async fn body_reader_pulls_a_content_length_body_incrementally() {
        let io = wired_with(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello").await;
        let mut t = AsyncTransport::new(io);
        let head = t.read_response_head(8192).await.unwrap();
        let framing = body::response_framing(&head.headers, &Method::Get, head.status).unwrap();
        let reader = t.into_body_reader(framing);
        assert_eq!(collect_all(reader).await, b"hello");
    }

    #[tokio::test]
    async fn body_reader_pulls_a_chunked_body_incrementally() {
        let io = wired_with(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n",
        )
        .await;
        let mut t = AsyncTransport::new(io);
        let head = t.read_response_head(8192).await.unwrap();
        let framing = body::response_framing(&head.headers, &Method::Get, head.status).unwrap();
        let reader = t.into_body_reader(framing);
        assert_eq!(collect_all(reader).await, b"hello world");
    }

    #[tokio::test]
    async fn body_reader_pulls_a_close_delimited_body_incrementally() {
        let io = wired_with(b"HTTP/1.0 200 OK\r\n\r\nwhatever is left").await;
        let mut t = AsyncTransport::new(io);
        let head = t.read_response_head(8192).await.unwrap();
        let framing = body::response_framing(&head.headers, &Method::Get, head.status).unwrap();
        assert_eq!(framing, Framing::Close);
        let reader = t.into_body_reader(framing);
        assert_eq!(collect_all(reader).await, b"whatever is left");
    }

    #[tokio::test]
    async fn body_reader_none_framing_yields_no_chunks() {
        let io = wired_with(b"HTTP/1.1 204 No Content\r\n\r\n").await;
        let mut t = AsyncTransport::new(io);
        let head = t.read_response_head(8192).await.unwrap();
        let framing = body::response_framing(&head.headers, &Method::Get, head.status).unwrap();
        assert_eq!(framing, Framing::None);
        let mut reader = t.into_body_reader(framing);
        assert_eq!(reader.next_chunk().await.unwrap(), None);
    }

    #[tokio::test]
    async fn body_reader_errors_on_truncated_chunked_body() {
        let io = wired_with(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhel").await;
        let mut t = AsyncTransport::new(io);
        let head = t.read_response_head(8192).await.unwrap();
        let framing = body::response_framing(&head.headers, &Method::Get, head.status).unwrap();
        let mut reader = t.into_body_reader(framing);
        // The first call may legitimately return the partial chunk data
        // that did arrive ("hel", 3 of the declared 5 bytes) -- the error
        // only surfaces once the reader asks for more and the transport
        // has nothing left to give.
        let mut saw_error = false;
        for _ in 0..4 {
            match reader.next_chunk().await {
                Ok(Some(_)) => continue,
                Ok(None) => break,
                Err(_) => {
                    saw_error = true;
                    break;
                }
            }
        }
        assert!(saw_error, "expected the truncated chunked body to error");
    }
}
