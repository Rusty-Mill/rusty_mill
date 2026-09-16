/// Stub for request.
use crate::frame;
use crate::error::Result;

/// An HTTP/2 request.
pub struct Request {
    method: String,
    scheme: String,
    authority: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    /// Create a new request.
    pub fn new(
        method: String,
        scheme: String,
        authority: String,
        path: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Self {
        Request {
            method,
            scheme,
            authority,
            path,
            headers,
            body,
        }
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn authority(&self) -> &str {
        &self.authority
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Encode this request into a frame.
    pub fn encode(&self) -> Result<frame::Frame> {
        Ok(frame::Frame::Data(frame::DataFrame {
            stream_id: 0,
            data: self.body.clone(),
            end_stream: false,
        }))
    }
}
