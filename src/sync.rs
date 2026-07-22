//! The sync adapter: drives the sans-IO core over any `std::io::Read +
//! Write` transport (a `TcpStream`, a Unix domain socket, an in-memory
//! duplex in tests). Thin by design -- [`SyncTransport`] owns a growable
//! buffer and the transport itself, but every parsing/framing decision is
//! delegated to [`crate::head`]/[`crate::body`]; this module never
//! second-guesses what they return.
//!
//! Buffering mirrors `rusty_request`'s donor `BufReader` (`http1.rs`):
//! `buf[start..end]` is the unconsumed, already-received data, and a
//! `read_*` call keeps pulling from the transport only until the core
//! reports `Complete`/enough bytes -- so leftover bytes (the start of a
//! body that arrived in the same packet as the head, or -- mid-upgrade --
//! bytes belonging to a different protocol entirely) are exactly
//! preserved for whatever reads next, never dropped or re-read from the
//! transport.

use std::io::{Read, Write};

use crate::body::{self, ChunkedDecoder, Framing, Progress, DEFAULT_MAX_LINE_LEN};
use crate::head::{self, Outcome, RequestHead, ResponseHead};
use crate::transport::{unexpected_eof, Result};

/// The initial buffer size a fresh [`SyncTransport`] allocates; it grows
/// (doubling) as needed up to whatever `max_head_len`/`max_line_len` the
/// caller passes to a `read_*` call.
const INITIAL_BUF_LEN: usize = 4096;

/// Drives [`crate::head`] and [`crate::body`] over a `T: Read + Write`.
pub struct SyncTransport<T> {
    io: T,
    buf: Vec<u8>,
    /// `buf[start..end]` is unconsumed, already-received data.
    start: usize,
    end: usize,
}

impl<T: Read + Write> SyncTransport<T> {
    /// Wraps an already-connected transport. Performs no I/O itself.
    pub fn new(io: T) -> Self {
        SyncTransport {
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

    /// Consumes `self`, returning the underlying transport. Any bytes
    /// already buffered but unconsumed (e.g. the start of a body that
    /// arrived in the same packet as the head) are discarded -- a caller
    /// switching protocols mid-connection (Noise, DERP) should finish
    /// reading everything it needs through this adapter first, the same
    /// discipline `rusty_request`'s donor `BufReader::into_stream` relies
    /// on today.
    pub fn into_inner(self) -> T {
        self.io
    }

    /// Reads more bytes from the transport into the buffer, growing it
    /// first if it's full. Returns `0` only on a clean transport EOF.
    fn fill_more(&mut self) -> Result<usize> {
        if self.start > 0 {
            self.buf.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
        if self.end == self.buf.len() {
            self.buf.resize(self.buf.len() * 2, 0);
        }
        let n = self.io.read(&mut self.buf[self.end..])?;
        self.end += n;
        Ok(n)
    }

    /// Reads and parses a request head, growing the buffer until
    /// [`head::parse_request_head`] reports `Complete`. `max_head_len`
    /// is the same bound that function takes.
    pub fn read_request_head(&mut self, max_head_len: usize) -> Result<RequestHead> {
        loop {
            match head::parse_request_head(&self.buf[self.start..self.end], max_head_len)? {
                Outcome::Complete { head, consumed } => {
                    self.start += consumed;
                    return Ok(head);
                }
                Outcome::Incomplete => {
                    if self.fill_more()? == 0 {
                        return Err(unexpected_eof(
                            "connection closed before the request head completed",
                        ));
                    }
                }
            }
        }
    }

    /// Reads and parses a response head. Same contract as
    /// [`Self::read_request_head`].
    pub fn read_response_head(&mut self, max_head_len: usize) -> Result<ResponseHead> {
        loop {
            match head::parse_response_head(&self.buf[self.start..self.end], max_head_len)? {
                Outcome::Complete { head, consumed } => {
                    self.start += consumed;
                    return Ok(head);
                }
                Outcome::Incomplete => {
                    if self.fill_more()? == 0 {
                        return Err(unexpected_eof(
                            "connection closed before the response head completed",
                        ));
                    }
                }
            }
        }
    }

    /// Serializes and writes a request head.
    pub fn write_request_head(&mut self, head: &RequestHead) -> Result<()> {
        let mut out = Vec::new();
        head.write(&mut out);
        self.io.write_all(&out)?;
        Ok(())
    }

    /// Serializes and writes a response head.
    pub fn write_response_head(&mut self, head: &ResponseHead) -> Result<()> {
        let mut out = Vec::new();
        head.write(&mut out);
        self.io.write_all(&out)?;
        Ok(())
    }

    /// Reads a whole body into memory according to `framing`. For
    /// [`Framing::Chunked`], uses [`Self::read_chunked_body`] with
    /// [`crate::body::DEFAULT_MAX_LINE_LEN`] -- call that directly for a
    /// different bound.
    pub fn read_body(&mut self, framing: Framing) -> Result<Vec<u8>> {
        match framing {
            Framing::None => Ok(Vec::new()),
            Framing::ContentLength(len) => self.read_content_length_body(len),
            Framing::Close => self.read_close_delimited_body(),
            Framing::Chunked => self.read_chunked_body(DEFAULT_MAX_LINE_LEN),
        }
    }

    fn read_content_length_body(&mut self, len: u64) -> Result<Vec<u8>> {
        // Cap the *initial* reservation regardless of the declared length
        // (which an untrusted peer controls) -- the `Vec` still grows to
        // fit the real body via `extend_from_slice`, this just avoids
        // pre-allocating gigabytes on a bogus `Content-Length`. Same
        // mitigation the donor (`rusty_request`'s `http1.rs`) already
        // used.
        let cap = usize::try_from(len).unwrap_or(usize::MAX).min(1 << 20);
        let mut out = Vec::with_capacity(cap);
        while (out.len() as u64) < len {
            if self.start == self.end && self.fill_more()? == 0 {
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

    fn read_close_delimited_body(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            if self.start < self.end {
                out.extend_from_slice(&self.buf[self.start..self.end]);
                self.start = self.end;
            }
            if self.fill_more()? == 0 {
                return Ok(out);
            }
        }
    }

    /// Reads a `Transfer-Encoding: chunked` body into memory, bounding
    /// each framing line (chunk-size line, terminator, trailer line) by
    /// `max_line_len` -- see [`ChunkedDecoder::advance`].
    pub fn read_chunked_body(&mut self, max_line_len: usize) -> Result<Vec<u8>> {
        let mut decoder = ChunkedDecoder::new();
        let mut out = Vec::new();
        loop {
            match decoder.advance(&self.buf[self.start..self.end], max_line_len)? {
                Progress::Incomplete => {
                    if self.fill_more()? == 0 {
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

    /// Writes `body` verbatim -- for `Content-Length`-framed or
    /// close-delimited bodies, where the wire bytes are just the body
    /// itself.
    pub fn write_body(&mut self, body: &[u8]) -> Result<()> {
        self.io.write_all(body)?;
        Ok(())
    }

    /// Writes one `Transfer-Encoding: chunked` chunk. A no-op for an
    /// empty `data` -- see [`crate::body::write_chunk`].
    pub fn write_chunk(&mut self, data: &[u8]) -> Result<()> {
        let mut out = Vec::new();
        body::write_chunk(&mut out, data);
        self.io.write_all(&out)?;
        Ok(())
    }

    /// Writes the terminating zero-size chunk, ending a chunked body.
    pub fn write_chunked_end(&mut self) -> Result<()> {
        let mut out = Vec::new();
        body::write_chunked_end(&mut out);
        self.io.write_all(&out)?;
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
    fn read_some(&mut self, max: usize) -> Result<Vec<u8>> {
        if max == 0 {
            return Ok(Vec::new());
        }
        if self.start == self.end && self.fill_more()? == 0 {
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
/// [`SyncTransport::into_body_reader`] and pulled incrementally via
/// [`Self::next_chunk`] instead of requiring the whole thing in memory
/// upfront. Owns the transport (and therefore the connection) for as long
/// as it's alive -- see [`Self::into_inner`] to get it back once done.
pub struct BodyReader<T> {
    transport: SyncTransport<T>,
    state: BodyReaderState,
    done: bool,
}

impl<T: Read + Write> BodyReader<T> {
    /// The next chunk of body data, or `None` once the body is fully
    /// consumed. Chunk boundaries are an implementation detail (a wire
    /// chunk boundary for `Chunked` framing, or just "however much one
    /// read returned" otherwise) -- never rely on chunk size or count.
    pub fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        if self.done {
            return Ok(None);
        }
        match self.state {
            BodyReaderState::None => {
                self.done = true;
                Ok(None)
            }
            BodyReaderState::Close => {
                let data = self.transport.read_some(READ_CHUNK_SIZE)?;
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
                let data = self.transport.read_some(take)?;
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
                        if self.transport.fill_more()? == 0 {
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
    pub fn into_inner(self) -> SyncTransport<T> {
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
    use std::io::Cursor;

    /// An in-memory `Read + Write` transport: reads come from a fixed
    /// input buffer, writes go to a growable output buffer -- enough to
    /// exercise `SyncTransport` without a real socket.
    struct Loopback {
        input: Cursor<Vec<u8>>,
        pub output: Vec<u8>,
    }

    impl Loopback {
        fn new(input: &[u8]) -> Self {
            Loopback {
                input: Cursor::new(input.to_vec()),
                output: Vec::new(),
            }
        }
    }

    impl Read for Loopback {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Loopback {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.output.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn reads_request_head_then_content_length_body() {
        let wire = b"POST /a HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhello";
        let mut t = SyncTransport::new(Loopback::new(wire));
        let head = t.read_request_head(8192).unwrap();
        assert_eq!(head.method, Method::Post);
        assert_eq!(head.target, "/a");
        let framing = body::request_framing(&head.headers).unwrap();
        let body = t.read_body(framing).unwrap();
        assert_eq!(body, b"hello");
    }

    #[test]
    fn reads_response_head_then_chunked_body() {
        let wire = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let mut t = SyncTransport::new(Loopback::new(wire));
        let head = t.read_response_head(8192).unwrap();
        let framing =
            body::response_framing(&head.headers, &Method::Get, StatusCode::from_u16(200)).unwrap();
        let body = t.read_body(framing).unwrap();
        assert_eq!(body, b"hello");
    }

    #[test]
    fn reads_close_delimited_body_to_eof() {
        let wire = b"HTTP/1.0 200 OK\r\n\r\nwhatever is left";
        let mut t = SyncTransport::new(Loopback::new(wire));
        let head = t.read_response_head(8192).unwrap();
        assert_eq!(head.version, Version::Http10);
        let framing =
            body::response_framing(&head.headers, &Method::Get, StatusCode::from_u16(200)).unwrap();
        assert_eq!(framing, Framing::Close);
        let body = t.read_body(framing).unwrap();
        assert_eq!(body, b"whatever is left");
    }

    #[test]
    fn leftover_bytes_after_the_head_survive_into_the_upgraded_protocol() {
        // The exact-head-consumption guarantee, exercised through the
        // buffering adapter this time instead of the bare parser: bytes
        // that arrive in the same read as the head (here: Noise-style
        // upgrade payload) must come back through `into_inner`'s buffered
        // leftover untouched, not be dropped by the adapter's own
        // buffer-growing logic.
        let head =
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: tailscale-control-protocol\r\n\r\n";
        let trailing = b"NOISE-BYTES-NOT-HTTP";
        let mut wire = Vec::new();
        wire.extend_from_slice(head);
        wire.extend_from_slice(trailing);

        let mut t = SyncTransport::new(Loopback::new(&wire));
        let parsed = t.read_response_head(8192).unwrap();
        assert_eq!(parsed.status.as_u16(), 101);
        // Nothing has framing to read (a 101 has no body), so the
        // leftover bytes are still sitting in the adapter's buffer,
        // exactly the way a caller about to hand the raw transport to a
        // different protocol needs them preserved.
        assert_eq!(&t.buf[t.start..t.end], trailing);
    }

    #[test]
    fn writes_request_head() {
        let mut t = SyncTransport::new(Loopback::new(b""));
        let mut headers = HeaderMap::new();
        headers.insert("Host", "example.com").unwrap();
        let head = RequestHeadForTest::get("/a", headers);
        t.write_request_head(&head).unwrap();
        assert_eq!(
            t.get_ref().output,
            b"GET /a HTTP/1.1\r\nHost: example.com\r\n\r\n"
        );
    }

    #[test]
    fn writes_chunked_body() {
        let mut t = SyncTransport::new(Loopback::new(b""));
        t.write_chunk(b"hello").unwrap();
        t.write_chunk(b"").unwrap(); // no-op, not an end marker
        t.write_chunked_end().unwrap();
        assert_eq!(t.get_ref().output, b"5\r\nhello\r\n0\r\n\r\n");
    }

    #[test]
    fn errors_on_truncated_content_length_body() {
        let wire = b"POST /a HTTP/1.1\r\nContent-Length: 10\r\n\r\nshort";
        let mut t = SyncTransport::new(Loopback::new(wire));
        let head = t.read_request_head(8192).unwrap();
        let framing = body::request_framing(&head.headers).unwrap();
        assert!(t.read_body(framing).is_err());
    }

    fn collect_all(mut reader: BodyReader<Loopback>) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(chunk) = reader.next_chunk().unwrap() {
            out.extend_from_slice(&chunk);
        }
        out
    }

    #[test]
    fn body_reader_pulls_a_content_length_body_incrementally() {
        let wire = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        let mut t = SyncTransport::new(Loopback::new(wire));
        let head = t.read_response_head(8192).unwrap();
        let framing = body::response_framing(&head.headers, &Method::Get, head.status).unwrap();
        let reader = t.into_body_reader(framing);
        assert_eq!(collect_all(reader), b"hello");
    }

    #[test]
    fn body_reader_pulls_a_chunked_body_incrementally() {
        let wire =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let mut t = SyncTransport::new(Loopback::new(wire));
        let head = t.read_response_head(8192).unwrap();
        let framing = body::response_framing(&head.headers, &Method::Get, head.status).unwrap();
        let reader = t.into_body_reader(framing);
        assert_eq!(collect_all(reader), b"hello world");
    }

    #[test]
    fn body_reader_pulls_a_close_delimited_body_incrementally() {
        let wire = b"HTTP/1.0 200 OK\r\n\r\nwhatever is left";
        let mut t = SyncTransport::new(Loopback::new(wire));
        let head = t.read_response_head(8192).unwrap();
        let framing = body::response_framing(&head.headers, &Method::Get, head.status).unwrap();
        assert_eq!(framing, Framing::Close);
        let reader = t.into_body_reader(framing);
        assert_eq!(collect_all(reader), b"whatever is left");
    }

    #[test]
    fn body_reader_none_framing_yields_no_chunks() {
        let wire = b"HTTP/1.1 204 No Content\r\n\r\n";
        let mut t = SyncTransport::new(Loopback::new(wire));
        let head = t.read_response_head(8192).unwrap();
        let framing = body::response_framing(&head.headers, &Method::Get, head.status).unwrap();
        assert_eq!(framing, Framing::None);
        let mut reader = t.into_body_reader(framing);
        assert_eq!(reader.next_chunk().unwrap(), None);
    }

    #[test]
    fn body_reader_errors_on_truncated_chunked_body() {
        let wire = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhel";
        let mut t = SyncTransport::new(Loopback::new(wire));
        let head = t.read_response_head(8192).unwrap();
        let framing = body::response_framing(&head.headers, &Method::Get, head.status).unwrap();
        let mut reader = t.into_body_reader(framing);
        // The first call may legitimately return the partial chunk data
        // that did arrive ("hel", 3 of the declared 5 bytes) -- the error
        // only surfaces once the reader asks for more and the transport
        // has nothing left to give.
        let mut saw_error = false;
        for _ in 0..4 {
            match reader.next_chunk() {
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

    /// Test-only helper so the write tests don't need to hand-build a
    /// `RequestHead` inline everywhere.
    struct RequestHeadForTest;
    impl RequestHeadForTest {
        fn get(target: &str, headers: HeaderMap) -> RequestHead {
            RequestHead {
                method: Method::Get,
                target: target.to_string(),
                version: Version::Http11,
                headers,
            }
        }
    }
}
