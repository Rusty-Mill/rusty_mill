//! Sans-IO parsing and serialization of HTTP/1.1 request and response
//! heads -- the request-line/status-line plus headers, up to and
//! including the blank line that ends them. Inverted from
//! `rusty_request`'s `http1.rs` (donor 1 of the mission handoff), which
//! parsed the same shape against an async `BufReader` it owned; here the
//! caller owns the buffer and the bytes, and this module only ever looks
//! at what it's handed.
//!
//! The one behavior every adapter depends on: [`parse_request_head`] and
//! [`parse_response_head`] consume **exactly** the head and nothing past
//! it (`Outcome::Complete::consumed` is precise), so a caller mid-
//! protocol-upgrade (Noise, DERP, WebSocket-style flows) can take the
//! underlying connection over byte-exact. See `ts-control/controlhttp.rs`
//! (donor 4) for why this matters: over-reading even one byte into the
//! upgraded stream would corrupt it.

use crate::error::{Error, Result};
use crate::header::HeaderMap;
use crate::method::Method;
use crate::status::StatusCode;
use crate::util::next_line;
use crate::version::Version;

/// A sane default cap on head size: generous for real-world headers, far
/// tighter than a client-only parser needs (`rusty_request`'s donor used
/// 8 MiB, safe only because it never faced untrusted input) -- this core
/// also parses server-bound requests from arbitrary peers, so it defaults
/// to the same order of magnitude nginx/Apache use. Callers accepting
/// only trusted/known peers (e.g. a client parsing its own server's
/// response) can pass a larger bound explicitly.
pub const DEFAULT_MAX_HEAD_LEN: usize = 8 * 1024;

/// The result of one parse attempt against a byte buffer that may not yet
/// contain a full head.
#[derive(Debug)]
pub enum Outcome<T> {
    /// Not enough bytes in the buffer yet -- read more and retry with the
    /// extended buffer (not a fresh one; nothing already scanned is
    /// consumed on an `Incomplete` result).
    Incomplete,
    /// The head was fully parsed. `consumed` is exactly how many bytes of
    /// the input buffer it occupied; `&buf[consumed..]` is untouched and
    /// belongs to the body, a pipelined next message, or (mid-upgrade) an
    /// entirely different protocol.
    Complete {
        /// The parsed head.
        head: T,
        /// Exactly how many bytes of the input buffer the head occupied.
        consumed: usize,
    },
}

/// A parsed HTTP/1.1 request head: the request line plus headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHead {
    /// The request method.
    pub method: Method,
    /// The request-target verbatim (origin-form, absolute-form, or
    /// otherwise) -- not parsed as a [`crate::Url`]; a caller that needs
    /// one can parse it from context (origin-form has no scheme/host).
    pub target: String,
    /// The HTTP version named on the request line.
    pub version: Version,
    /// The request headers.
    pub headers: HeaderMap,
}

impl RequestHead {
    /// Serializes the request line and headers, ending with the blank
    /// line -- ready to prepend to the body on the wire.
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.method.as_str().as_bytes());
        out.push(b' ');
        out.extend_from_slice(self.target.as_bytes());
        out.push(b' ');
        out.extend_from_slice(self.version.to_string().as_bytes());
        out.extend_from_slice(b"\r\n");
        write_headers(&self.headers, out);
    }
}

/// A parsed HTTP/1.1 response head: the status line plus headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseHead {
    /// The status code.
    pub status: StatusCode,
    /// The reason phrase, verbatim (may be empty).
    pub reason: String,
    /// The HTTP version named on the status line.
    pub version: Version,
    /// The response headers.
    pub headers: HeaderMap,
}

impl ResponseHead {
    /// Serializes the status line and headers, ending with the blank
    /// line -- ready to prepend to the body on the wire.
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.version.to_string().as_bytes());
        out.push(b' ');
        out.extend_from_slice(self.status.as_u16().to_string().as_bytes());
        out.push(b' ');
        out.extend_from_slice(self.reason.as_bytes());
        out.extend_from_slice(b"\r\n");
        write_headers(&self.headers, out);
    }
}

fn write_headers(headers: &HeaderMap, out: &mut Vec<u8>) {
    for (name, value) in headers.iter() {
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
}

/// Parses a request line + headers from the start of `buf`. `max_head_len`
/// bounds the total size of the head (see [`DEFAULT_MAX_HEAD_LEN`]); a
/// head that never completes within that bound is
/// [`Error::HeadTooLarge`], not an endless `Incomplete`.
pub fn parse_request_head(buf: &[u8], max_head_len: usize) -> Result<Outcome<RequestHead>> {
    let Some((line, first_line_len)) = next_line(buf) else {
        return incomplete_or_too_large(buf.len(), max_head_len);
    };
    let line_str = std::str::from_utf8(line)
        .map_err(|_| Error::InvalidHead("non-UTF-8 request line".into()))?;
    let mut parts = line_str.splitn(3, ' ');
    let method_tok = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::InvalidHead("empty request line".into()))?;
    let target = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::InvalidHead("missing request target".into()))?;
    let version_tok = parts
        .next()
        .ok_or_else(|| Error::InvalidHead("missing http version".into()))?;
    let version = Version::parse(version_tok)?;

    let Some((headers, headers_len)) = parse_header_lines(&buf[first_line_len..])? else {
        return incomplete_or_too_large(buf.len(), max_head_len);
    };

    Ok(Outcome::Complete {
        head: RequestHead {
            method: Method::parse(method_tok),
            target: target.to_string(),
            version,
            headers,
        },
        consumed: first_line_len + headers_len,
    })
}

/// Parses a status line + headers from the start of `buf`. Same
/// `max_head_len` contract as [`parse_request_head`].
pub fn parse_response_head(buf: &[u8], max_head_len: usize) -> Result<Outcome<ResponseHead>> {
    let Some((line, first_line_len)) = next_line(buf) else {
        return incomplete_or_too_large(buf.len(), max_head_len);
    };
    let line_str = std::str::from_utf8(line)
        .map_err(|_| Error::InvalidHead("non-UTF-8 status line".into()))?;
    let mut parts = line_str.splitn(3, ' ');
    let version_tok = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::InvalidHead("empty status line".into()))?;
    let version = Version::parse(version_tok)?;
    let code_tok = parts
        .next()
        .ok_or_else(|| Error::InvalidHead("missing status code".into()))?;
    let code: u16 = code_tok
        .parse()
        .map_err(|_| Error::InvalidHead(format!("invalid status code `{code_tok}`")))?;
    let reason = parts.next().unwrap_or("").to_string();

    let Some((headers, headers_len)) = parse_header_lines(&buf[first_line_len..])? else {
        return incomplete_or_too_large(buf.len(), max_head_len);
    };

    Ok(Outcome::Complete {
        head: ResponseHead {
            status: StatusCode::from_u16(code),
            reason,
            version,
            headers,
        },
        consumed: first_line_len + headers_len,
    })
}

/// Parses zero or more `Name: value` lines up to and including the blank
/// line that ends them. `None` means incomplete (no blank line found
/// yet); the caller checks the total-buffer-size bound itself, same as
/// the request/status line callers above.
fn parse_header_lines(buf: &[u8]) -> Result<Option<(HeaderMap, usize)>> {
    let mut headers = HeaderMap::new();
    let mut consumed = 0;
    loop {
        let Some((line, line_len)) = next_line(&buf[consumed..]) else {
            return Ok(None);
        };
        consumed += line_len;
        if line.is_empty() {
            return Ok(Some((headers, consumed)));
        }
        let line_str = std::str::from_utf8(line)
            .map_err(|_| Error::InvalidHead("non-UTF-8 header line".into()))?;
        let (name, value) = line_str
            .split_once(':')
            .ok_or_else(|| Error::InvalidHead(format!("malformed header line `{line_str}`")))?;
        headers.append(name.trim(), value.trim())?;
    }
}

fn incomplete_or_too_large<T>(buf_len: usize, max_head_len: usize) -> Result<Outcome<T>> {
    if buf_len >= max_head_len {
        Err(Error::HeadTooLarge)
    } else {
        Ok(Outcome::Incomplete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_request_head() {
        let buf = b"GET /a/b?x=1 HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let Outcome::Complete { head, consumed } = parse_request_head(buf, 1024).unwrap() else {
            panic!("expected Complete");
        };
        assert_eq!(head.method, Method::Get);
        assert_eq!(head.target, "/a/b?x=1");
        assert_eq!(head.version, Version::Http11);
        assert_eq!(head.headers.get("host"), Some("example.com"));
        assert_eq!(consumed, buf.len());
    }

    #[test]
    fn parses_simple_response_head() {
        let buf = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n";
        let Outcome::Complete { head, consumed } = parse_response_head(buf, 1024).unwrap() else {
            panic!("expected Complete");
        };
        assert_eq!(head.status.as_u16(), 200);
        assert_eq!(head.reason, "OK");
        assert_eq!(head.headers.get("content-length"), Some("2"));
        assert_eq!(consumed, buf.len());
    }

    #[test]
    fn request_head_over_reads_nothing_past_the_blank_line() {
        // The core requirement donor 4 (ts-control/controlhttp.rs) needs:
        // whatever comes after the head must be left byte-exact, so a
        // protocol-upgrade caller (Noise, DERP) can take the connection
        // over without losing or corrupting a single byte.
        let head =
            b"POST /ts2021 HTTP/1.1\r\nHost: x\r\nUpgrade: tailscale-control-protocol\r\n\r\n";
        let trailing = b"\x01\x02NOISE-HANDSHAKE-BYTES-NOT-HTTP\xff\x00";
        let mut buf = Vec::new();
        buf.extend_from_slice(head);
        buf.extend_from_slice(trailing);

        let Outcome::Complete {
            head: parsed,
            consumed,
        } = parse_request_head(&buf, 1024).unwrap()
        else {
            panic!("expected Complete");
        };
        assert_eq!(consumed, head.len());
        assert_eq!(&buf[consumed..], trailing);
        assert_eq!(parsed.target, "/ts2021");
    }

    #[test]
    fn response_head_over_reads_nothing_past_the_blank_line() {
        let head = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: DERP\r\n\r\n";
        let trailing = b"first-derp-frame-bytes";
        let mut buf = Vec::new();
        buf.extend_from_slice(head);
        buf.extend_from_slice(trailing);

        let Outcome::Complete {
            head: parsed,
            consumed,
        } = parse_response_head(&buf, 1024).unwrap()
        else {
            panic!("expected Complete");
        };
        assert_eq!(consumed, head.len());
        assert_eq!(&buf[consumed..], trailing);
        assert_eq!(parsed.status.as_u16(), 101);
    }

    #[test]
    fn incomplete_request_head_reports_incomplete_not_error() {
        let buf = b"GET / HTTP/1.1\r\nHost: exa";
        assert!(matches!(
            parse_request_head(buf, 1024).unwrap(),
            Outcome::Incomplete
        ));
    }

    #[test]
    fn incomplete_response_head_reports_incomplete_not_error() {
        let buf = b"HTTP/1.1 200 OK\r\nContent-Le";
        assert!(matches!(
            parse_response_head(buf, 1024).unwrap(),
            Outcome::Incomplete
        ));
    }

    #[test]
    fn request_head_over_max_len_without_terminator_is_an_error() {
        let buf = b"GET / HTTP/1.1\r\nHost: example.com\r\n"; // no blank line yet
        assert_eq!(parse_request_head(buf, 8).unwrap_err(), Error::HeadTooLarge);
    }

    #[test]
    fn parses_extension_method() {
        let buf = b"PROPFIND /a HTTP/1.1\r\n\r\n";
        let Outcome::Complete { head, .. } = parse_request_head(buf, 1024).unwrap() else {
            panic!("expected Complete");
        };
        assert_eq!(head.method, Method::Extension("PROPFIND".to_string()));
    }

    #[test]
    fn rejects_malformed_request_line() {
        let buf = b"GET\r\n\r\n";
        assert!(parse_request_head(buf, 1024).is_err());
    }

    #[test]
    fn rejects_unsupported_version() {
        let buf = b"GET / HTTP/2.0\r\n\r\n";
        assert!(parse_request_head(buf, 1024).is_err());
    }

    #[test]
    fn request_head_round_trips_through_write() {
        let buf = b"GET /a?x=1 HTTP/1.1\r\nHost: example.com\r\nX-A: 1\r\n\r\n";
        let Outcome::Complete { head, .. } = parse_request_head(buf, 1024).unwrap() else {
            panic!("expected Complete");
        };
        let mut out = Vec::new();
        head.write(&mut out);
        assert_eq!(out, buf);
    }

    #[test]
    fn response_head_round_trips_through_write() {
        let buf = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let Outcome::Complete { head, .. } = parse_response_head(buf, 1024).unwrap() else {
            panic!("expected Complete");
        };
        let mut out = Vec::new();
        head.write(&mut out);
        assert_eq!(out, buf);
    }
}
