//! An HTTP status code.

/// An HTTP status code. Ported verbatim from `rusty_request`'s `status.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StatusCode(u16);

impl StatusCode {
    /// Wraps a raw status code. Never validates the range -- a peer can
    /// send anything in a status line, and rejecting it is the head
    /// parser's call, not this type's.
    pub fn from_u16(code: u16) -> Self {
        StatusCode(code)
    }

    /// The raw numeric status code.
    pub fn as_u16(&self) -> u16 {
        self.0
    }

    /// `1xx`.
    pub fn is_informational(&self) -> bool {
        (100..200).contains(&self.0)
    }

    /// `2xx`.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.0)
    }

    /// `3xx`.
    pub fn is_redirection(&self) -> bool {
        (300..400).contains(&self.0)
    }

    /// `4xx`.
    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.0)
    }

    /// `5xx`.
    pub fn is_server_error(&self) -> bool {
        (500..600).contains(&self.0)
    }
}

impl std::fmt::Display for StatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
