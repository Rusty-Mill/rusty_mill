/// Stub response.
use crate::hpack;

/// An HTTP/2 response.
pub struct Response {
    status: u16,
    headers: Vec<hpack::HeaderField>,
    body: Vec<u8>,
    end_of_stream: bool,
}

impl Response {
    /// Create a new response.
    pub fn new(status: u16, headers: Vec<hpack::HeaderField>, body: Vec<u8>, end_of_stream: bool) -> Self {
        Response {
            status,
            headers,
            body,
            end_of_stream,
        }
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn headers(&self) -> &[hpack::HeaderField] {
        &self.headers
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn end_of_stream(&self) -> bool {
        self.end_of_stream
    }
}
