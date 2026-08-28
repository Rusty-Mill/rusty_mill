//! How an HTTP/1.1 body's end is determined (RFC 7230 §3.3.3), and the
//! incremental chunked-body decoder -- inverted from `rusty_request`'s
//! `http1.rs` `Framing`/`ChunkedState` (donor 1), which drove the same
//! state machine against an owned async reader. Here the caller owns the
//! buffer: [`ChunkedDecoder::advance`] looks only at the bytes it's
//! handed and reports what to do next, the same sans-IO shape as
//! [`crate::head`]'s parsers.

use crate::error::{Error, Result};
use crate::header::HeaderMap;
use crate::method::Method;
use crate::status::StatusCode;
use crate::util::next_line;

/// Default cap on a single chunked-framing line (a chunk-size line, its
/// terminator, or one trailer header line) -- same reasoning and same
/// value as [`crate::head::DEFAULT_MAX_HEAD_LEN`], applied per-line
/// instead of to the whole head.
pub const DEFAULT_MAX_LINE_LEN: usize = 8 * 1024;

/// How a message body's end is determined once the head is parsed.
/// Requests and responses use this differently (see
/// [`request_framing`]/[`response_framing`]): a request without a framing
/// header simply has no body, while a response without one is read to
/// EOF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// No body, regardless of what any header claims (HEAD, 204, 304,
    /// 1xx responses; a request with no `Content-Length`/
    /// `Transfer-Encoding`).
    None,
    /// The body is exactly this many bytes, from the `Content-Length`
    /// header.
    ContentLength(u64),
    /// `Transfer-Encoding: chunked`; see [`ChunkedDecoder`].
    Chunked,
    /// No framing header at all on a response: read to EOF. Legal for an
    /// HTTP/1.0-style response; the connection can never be reused
    /// afterward either way, since reaching EOF is the peer having
    /// already closed it. Never returned for a request -- see
    /// [`request_framing`].
    Close,
}

/// Body framing for a request, per RFC 7230 §3.3.3: `Transfer-Encoding:
/// chunked` wins if present, else `Content-Length`, else the request has
/// no body at all. Unlike a response, a bodyless request is never
/// close-delimited -- the connection doesn't need to close to signal
/// "no body here".
pub fn request_framing(headers: &HeaderMap) -> Result<Framing> {
    if is_chunked(headers) {
        return Ok(Framing::Chunked);
    }
    if let Some(len) = headers.get("content-length") {
        return Ok(Framing::ContentLength(parse_content_length(len)?));
    }
    Ok(Framing::None)
}

/// Body framing for a response, per RFC 7230 §3.3/§3.3.3: HEAD requests
/// and 204/304/1xx responses never carry a body regardless of headers;
/// otherwise chunked wins over `Content-Length`, and a response with
/// neither is read to EOF (legal for HTTP/1.0-flavored servers).
pub fn response_framing(
    headers: &HeaderMap,
    method: &Method,
    status: StatusCode,
) -> Result<Framing> {
    if *method == Method::Head
        || status.as_u16() == 204
        || status.as_u16() == 304
        || status.is_informational()
    {
        return Ok(Framing::None);
    }
    if is_chunked(headers) {
        return Ok(Framing::Chunked);
    }
    if let Some(len) = headers.get("content-length") {
        return Ok(Framing::ContentLength(parse_content_length(len)?));
    }
    Ok(Framing::Close)
}

/// A `Transfer-Encoding` header can list multiple tokens; `chunked` only
/// counts as the wire's actual framing when it's the last one (RFC 7230
/// §3.3.1).
fn is_chunked(headers: &HeaderMap) -> bool {
    headers
        .get("transfer-encoding")
        .map(|v| {
            v.split(',')
                .next_back()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("chunked")
        })
        .unwrap_or(false)
}

fn parse_content_length(raw: &str) -> Result<u64> {
    raw.trim()
        .parse()
        .map_err(|_| Error::InvalidContentLength(raw.to_string()))
}

/// Which step of chunked-body framing [`ChunkedDecoder::advance`] is
/// currently in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Expecting a chunk-size line next.
    Size,
    /// `u64` bytes of the current chunk's data remain to be read.
    Data(u64),
    /// Just finished a chunk's data; expecting its trailing CRLF.
    DataTerminator,
    /// The zero-size chunk was seen; reading 0+ trailer header lines
    /// then the final blank line.
    Trailers,
    /// The blank line ending the trailer section was seen; nothing left.
    Done,
}

/// The result of one [`ChunkedDecoder::advance`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Not enough bytes in `buf` yet to make progress -- read more from
    /// the transport and call again with the extended buffer.
    Incomplete,
    /// `consumed` bytes of pure chunk framing (a chunk-size line, a
    /// chunk's trailing CRLF, or one trailer line) were consumed; no body
    /// data was produced.
    Framing {
        /// How many bytes of `buf` were consumed.
        consumed: usize,
    },
    /// `len` bytes of body data sit at the start of `buf` -- the caller
    /// should treat them as body output and advance past them.
    Data {
        /// How many bytes at the start of `buf` are body data.
        len: usize,
    },
    /// The body is complete. `consumed` bytes (the trailer section's
    /// final blank line) were consumed; nothing more will ever be
    /// produced by this decoder.
    Done {
        /// How many bytes of `buf` were consumed.
        consumed: usize,
    },
}

/// An incremental `Transfer-Encoding: chunked` body decoder. Sans-IO:
/// owns only its own state enum, never a buffer or a socket -- the
/// caller keeps the bytes and drives [`Self::advance`] in a loop,
/// exactly the shape [`crate::head`]'s parsers use.
///
/// ```
/// use rusty_http::body::{ChunkedDecoder, Progress, DEFAULT_MAX_LINE_LEN};
///
/// let wire = b"5\r\nhello\r\n0\r\n\r\n";
/// let mut decoder = ChunkedDecoder::new();
/// let mut pos = 0;
/// let mut body = Vec::new();
/// loop {
///     match decoder.advance(&wire[pos..], DEFAULT_MAX_LINE_LEN).unwrap() {
///         Progress::Incomplete => panic!("wire is a complete example"),
///         Progress::Framing { consumed } => pos += consumed,
///         Progress::Data { len } => {
///             body.extend_from_slice(&wire[pos..pos + len]);
///             pos += len;
///         }
///         Progress::Done { consumed } => {
///             pos += consumed;
///             break;
///         }
///     }
/// }
/// assert_eq!(body, b"hello");
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ChunkedDecoder {
    state: State,
}

impl Default for ChunkedDecoder {
    fn default() -> Self {
        ChunkedDecoder { state: State::Size }
    }
}

impl ChunkedDecoder {
    /// A decoder positioned at the start of a chunked body.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the body has been fully decoded -- once `true`, further
    /// [`Self::advance`] calls only ever return `Done`.
    pub fn is_done(&self) -> bool {
        self.state == State::Done
    }

    /// Advances the decoder using bytes at the start of `buf`.
    /// `max_line_len` bounds a single framing line (see
    /// [`DEFAULT_MAX_LINE_LEN`]) the same way `max_head_len` bounds
    /// [`crate::head::parse_request_head`] -- a line that never arrives
    /// is [`Error::ChunkFramingTooLarge`], not an endless `Incomplete`.
    pub fn advance(&mut self, buf: &[u8], max_line_len: usize) -> Result<Progress> {
        match self.state {
            State::Size => {
                let Some((line, consumed)) = next_line(buf) else {
                    return incomplete_or_too_large(buf.len(), max_line_len);
                };
                let line_str = std::str::from_utf8(line)
                    .map_err(|_| Error::InvalidChunkSize("non-UTF-8 chunk-size line".into()))?;
                let size_str = line_str.split(';').next().unwrap_or("").trim();
                let size = u64::from_str_radix(size_str, 16)
                    .map_err(|_| Error::InvalidChunkSize(size_str.to_string()))?;
                self.state = if size == 0 {
                    State::Trailers
                } else {
                    State::Data(size)
                };
                Ok(Progress::Framing { consumed })
            }
            State::Data(remaining) => {
                if buf.is_empty() {
                    return Ok(Progress::Incomplete);
                }
                let take = remaining.min(buf.len() as u64) as usize;
                let left = remaining - take as u64;
                self.state = if left == 0 {
                    State::DataTerminator
                } else {
                    State::Data(left)
                };
                Ok(Progress::Data { len: take })
            }
            State::DataTerminator => {
                let Some((line, consumed)) = next_line(buf) else {
                    return incomplete_or_too_large(buf.len(), max_line_len);
                };
                if !line.is_empty() {
                    return Err(Error::InvalidChunkSize(
                        "malformed chunk terminator".to_string(),
                    ));
                }
                self.state = State::Size;
                Ok(Progress::Framing { consumed })
            }
            State::Trailers => {
                let Some((line, consumed)) = next_line(buf) else {
                    return incomplete_or_too_large(buf.len(), max_line_len);
                };
                if line.is_empty() {
                    self.state = State::Done;
                    Ok(Progress::Done { consumed })
                } else {
                    Ok(Progress::Framing { consumed })
                }
            }
            State::Done => Ok(Progress::Done { consumed: 0 }),
        }
    }
}

fn incomplete_or_too_large(buf_len: usize, max_line_len: usize) -> Result<Progress> {
    if buf_len >= max_line_len {
        Err(Error::ChunkFramingTooLarge)
    } else {
        Ok(Progress::Incomplete)
    }
}

/// Writes one chunk of `data` in `Transfer-Encoding: chunked` framing
/// (size line in hex, the data, then its CRLF terminator). Writes nothing
/// for an empty `data` -- an empty chunk is the end-of-body marker
/// ([`write_chunked_end`]), not a valid mid-stream chunk.
pub fn write_chunk(out: &mut Vec<u8>, data: &[u8]) {
    if data.is_empty() {
        return;
    }
    out.extend_from_slice(format!("{:x}\r\n", data.len()).as_bytes());
    out.extend_from_slice(data);
    out.extend_from_slice(b"\r\n");
}

/// Writes the terminating zero-size chunk and the (empty) trailer
/// section's blank line, ending a chunked body.
pub fn write_chunked_end(out: &mut Vec<u8>) {
    out.extend_from_slice(b"0\r\n\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(k, v).unwrap();
        }
        h
    }

    #[test]
    fn request_with_no_framing_header_has_no_body() {
        assert_eq!(request_framing(&headers(&[])).unwrap(), Framing::None);
    }

    #[test]
    fn request_content_length_framing() {
        let h = headers(&[("Content-Length", "42")]);
        assert_eq!(request_framing(&h).unwrap(), Framing::ContentLength(42));
    }

    #[test]
    fn request_chunked_framing() {
        let h = headers(&[("Transfer-Encoding", "chunked")]);
        assert_eq!(request_framing(&h).unwrap(), Framing::Chunked);
    }

    #[test]
    fn response_head_request_never_has_a_body() {
        let h = headers(&[("Content-Length", "100")]);
        assert_eq!(
            response_framing(&h, &Method::Head, StatusCode::from_u16(200)).unwrap(),
            Framing::None
        );
    }

    #[test]
    fn response_204_never_has_a_body() {
        assert_eq!(
            response_framing(&headers(&[]), &Method::Get, StatusCode::from_u16(204)).unwrap(),
            Framing::None
        );
    }

    #[test]
    fn response_with_no_framing_header_is_close_delimited() {
        assert_eq!(
            response_framing(&headers(&[]), &Method::Get, StatusCode::from_u16(200)).unwrap(),
            Framing::Close
        );
    }

    #[test]
    fn response_chunked_wins_over_content_length() {
        let h = headers(&[("Transfer-Encoding", "chunked"), ("Content-Length", "10")]);
        assert_eq!(
            response_framing(&h, &Method::Get, StatusCode::from_u16(200)).unwrap(),
            Framing::Chunked
        );
    }

    #[test]
    fn invalid_content_length_is_an_error() {
        let h = headers(&[("Content-Length", "not-a-number")]);
        assert!(request_framing(&h).is_err());
    }

    fn decode_all(wire: &[u8]) -> Vec<u8> {
        let mut decoder = ChunkedDecoder::new();
        let mut pos = 0;
        let mut body = Vec::new();
        loop {
            match decoder.advance(&wire[pos..], DEFAULT_MAX_LINE_LEN).unwrap() {
                Progress::Incomplete => panic!("test input should be complete"),
                Progress::Framing { consumed } => pos += consumed,
                Progress::Data { len } => {
                    body.extend_from_slice(&wire[pos..pos + len]);
                    pos += len;
                }
                Progress::Done { consumed } => {
                    pos += consumed;
                    break;
                }
            }
        }
        assert!(decoder.is_done());
        assert_eq!(pos, wire.len());
        body
    }

    #[test]
    fn decodes_single_chunk() {
        assert_eq!(decode_all(b"5\r\nhello\r\n0\r\n\r\n"), b"hello");
    }

    #[test]
    fn decodes_multiple_chunks() {
        assert_eq!(
            decode_all(b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"),
            b"hello world"
        );
    }

    #[test]
    fn decodes_empty_body() {
        assert_eq!(decode_all(b"0\r\n\r\n"), b"");
    }

    #[test]
    fn decodes_with_trailer_headers() {
        assert_eq!(decode_all(b"3\r\nabc\r\n0\r\nX-Trailer: 1\r\n\r\n"), b"abc");
    }

    #[test]
    fn chunk_extension_after_semicolon_is_ignored() {
        assert_eq!(decode_all(b"5;ext=1\r\nhello\r\n0\r\n\r\n"), b"hello");
    }

    #[test]
    fn decoder_reports_incomplete_across_partial_reads() {
        let mut decoder = ChunkedDecoder::new();
        assert!(matches!(
            decoder.advance(b"5\r\nhel", DEFAULT_MAX_LINE_LEN).unwrap(),
            Progress::Framing { consumed: 3 }
        ));
        // Only 3 of the 5 declared bytes are available so far.
        assert!(matches!(
            decoder.advance(b"hel", DEFAULT_MAX_LINE_LEN).unwrap(),
            Progress::Data { len: 3 }
        ));
        assert!(matches!(
            decoder.advance(b"", DEFAULT_MAX_LINE_LEN).unwrap(),
            Progress::Incomplete
        ));
    }

    #[test]
    fn malformed_chunk_size_is_an_error() {
        let mut decoder = ChunkedDecoder::new();
        assert!(decoder
            .advance(b"not-hex\r\n", DEFAULT_MAX_LINE_LEN)
            .is_err());
    }

    #[test]
    fn malformed_chunk_terminator_is_an_error() {
        let mut decoder = ChunkedDecoder::new();
        assert!(matches!(
            decoder.advance(b"1\r\n", DEFAULT_MAX_LINE_LEN).unwrap(),
            Progress::Framing { .. }
        ));
        assert!(matches!(
            decoder.advance(b"a", DEFAULT_MAX_LINE_LEN).unwrap(),
            Progress::Data { len: 1 }
        ));
        assert!(decoder.advance(b"XX\r\n", DEFAULT_MAX_LINE_LEN).is_err());
    }

    #[test]
    fn chunk_size_line_over_max_len_without_terminator_is_an_error() {
        let mut decoder = ChunkedDecoder::new();
        assert_eq!(
            decoder.advance(b"12345678", 4).unwrap_err(),
            Error::ChunkFramingTooLarge
        );
    }

    #[test]
    fn write_chunk_then_end_round_trips_through_the_decoder() {
        let mut wire = Vec::new();
        write_chunk(&mut wire, b"hello");
        write_chunk(&mut wire, b"");
        write_chunked_end(&mut wire);
        assert_eq!(decode_all(&wire), b"hello");
    }
}
