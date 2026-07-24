/// Stub module for response types.

/// An HTTP/2 response that has been received.
#[derive(Debug)]
pub struct Response {
    /// The HTTP status code.
    pub status: u16,
    /// The response headers.
    pub headers: Vec<crate::hpack::HeaderField>,
    /// Whether this frame carries END_STREAM.
    pub end_of_stream: bool,
}
