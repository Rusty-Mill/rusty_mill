use std::fmt;

/// Everything that can go wrong parsing, serializing, or framing an
/// HTTP/1.1 message. Sans-IO: there is deliberately no `Io` variant here --
/// this crate never touches a socket, so I/O failures are the adapter's own
/// error type to define.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The URL couldn't be parsed at all.
    InvalidUrl(String),
    /// The URL's scheme isn't `http` or `https`.
    UnsupportedScheme(String),
    /// A header name or value contained bytes that can't go on the wire
    /// (e.g. a bare `\r` or `\n`, which would let a caller smuggle a
    /// second header/request into the stream).
    InvalidHeader(String),
    /// The request line, status line, or a header line didn't parse.
    InvalidHead(String),
    /// The head (request line/status line + headers, up to and including
    /// the blank line) exceeded the caller-supplied size bound before a
    /// terminator was found -- a line that never arrives can't be allowed
    /// to grow a caller's buffer forever.
    HeadTooLarge,
    /// A `Content-Length` header's value wasn't a valid, non-negative
    /// integer.
    InvalidContentLength(String),
    /// A chunk-size line (or its trailing CRLF) was malformed.
    InvalidChunkSize(String),
    /// A chunk-size line, chunk terminator, or trailer line exceeded the
    /// caller-supplied size bound before a terminator was found -- same
    /// reasoning as [`Error::HeadTooLarge`], applied to chunked-body
    /// framing lines instead of the head.
    ChunkFramingTooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidUrl(s) => write!(f, "invalid url: {s}"),
            Error::UnsupportedScheme(s) => write!(
                f,
                "unsupported url scheme: {s} (only http:// and https:// are supported)"
            ),
            Error::InvalidHeader(s) => write!(f, "invalid header: {s}"),
            Error::InvalidHead(s) => write!(f, "invalid http head: {s}"),
            Error::HeadTooLarge => write!(f, "http head exceeded the maximum allowed size"),
            Error::InvalidContentLength(s) => write!(f, "invalid Content-Length: {s}"),
            Error::InvalidChunkSize(s) => write!(f, "invalid chunk size: {s}"),
            Error::ChunkFramingTooLarge => {
                write!(
                    f,
                    "chunked body framing line exceeded the maximum allowed size"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

/// This crate's `Result` alias.
pub type Result<T> = std::result::Result<T, Error>;
