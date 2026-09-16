use std::fmt;

/// HTTP/2 error codes as defined in RFC 9113 Section 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    NoError,
    ProtocolError,
    InternalError,
    FlowControlError,
    SettingsTimeout,
    StreamClosed,
    FrameSizeError,
    RefusedStream,
    Cancel,
    CompressionError,
    ConnectError,
    EnhanceYourCalm,
    InadequateSecurity,
    Http11Required,
    Unknown(u32),
}

impl ErrorCode {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0x0 => ErrorCode::NoError,
            0x1 => ErrorCode::ProtocolError,
            0x2 => ErrorCode::InternalError,
            0x3 => ErrorCode::FlowControlError,
            0x4 => ErrorCode::SettingsTimeout,
            0x5 => ErrorCode::StreamClosed,
            0x6 => ErrorCode::FrameSizeError,
            0x7 => ErrorCode::RefusedStream,
            0x8 => ErrorCode::Cancel,
            0x9 => ErrorCode::CompressionError,
            0xa => ErrorCode::ConnectError,
            0xb => ErrorCode::EnhanceYourCalm,
            0xc => ErrorCode::InadequateSecurity,
            0xd => ErrorCode::Http11Required,
            other => ErrorCode::Unknown(other),
        }
    }

    pub fn as_u32(self) -> u32 {
        match self {
            ErrorCode::NoError => 0x0,
            ErrorCode::ProtocolError => 0x1,
            ErrorCode::InternalError => 0x2,
            ErrorCode::FlowControlError => 0x3,
            ErrorCode::SettingsTimeout => 0x4,
            ErrorCode::StreamClosed => 0x5,
            ErrorCode::FrameSizeError => 0x6,
            ErrorCode::RefusedStream => 0x7,
            ErrorCode::Cancel => 0x8,
            ErrorCode::CompressionError => 0x9,
            ErrorCode::ConnectError => 0xa,
            ErrorCode::EnhanceYourCalm => 0xb,
            ErrorCode::InadequateSecurity => 0xc,
            ErrorCode::Http11Required => 0xd,
            ErrorCode::Unknown(v) => v,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} (0x{:x})", self, self.as_u32())
    }
}

/// Errors that can occur while decoding a frame or a header block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum H2Error {
    /// Not enough bytes were available to decode a complete frame/value.
    Incomplete,
    /// The frame violates a connection-level invariant (RFC 9113 §5.4.1).
    Connection(ErrorCode, &'static str),
    /// The frame violates a stream-level invariant; only that stream is affected.
    Stream(u32, ErrorCode, &'static str),
}

impl fmt::Display for H2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            H2Error::Incomplete => write!(f, "incomplete input"),
            H2Error::Connection(code, msg) => write!(f, "connection error {code}: {msg}"),
            H2Error::Stream(id, code, msg) => {
                write!(f, "stream error on stream {id}, {code}: {msg}")
            }
        }
    }
}

impl std::error::Error for H2Error {}

pub type Result<T> = std::result::Result<T, H2Error>;
