/// An incoming HTTP/2 request, received by the server.
pub struct IncomingRequest {
    stream_id: u32,
    method: String,
    scheme: String,
    authority: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl IncomingRequest {
    /// Create a new incoming request.
    pub fn new(
        stream_id: u32,
        method: String,
        scheme: String,
        authority: String,
        path: String,
        headers: Vec<(String, String)>,
    ) -> Self {
        IncomingRequest {
            stream_id,
            method,
            scheme,
            authority,
            path,
            headers,
        }
    }

    pub fn stream_id(&self) -> u32 {
        self.stream_id
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
}
