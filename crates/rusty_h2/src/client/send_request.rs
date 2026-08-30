//! Client-side request builder and send handle.

use std::collections::BTreeMap;

use crate::error::Result;
use crate::frame;
use crate::frame::header::Flags;
use crate::hpack;
use crate::hpack::Encoder;

/// A client-side request builder.
///
/// Parses an HTTP URI, populates `:method`, `:scheme`, `:authority`,
/// and `:path`, then builds an HPACK-encoded header block suitable for
/// the wire.
pub struct RequestBuilder {
    method: &'static str,
    scheme: String,
    authority: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    end_of_stream: bool,
    priority: Option<frame::Priority>,
}

impl RequestBuilder {
    /// Parse a URI and create a new request builder.
    pub fn new(method: &'static str, uri: &str) -> Self {
        let (scheme, authority, path) = parse_uri(uri);
        RequestBuilder {
            method,
            scheme,
            authority,
            path,
            headers: Vec::new(),
            body: Vec::new(),
            end_of_stream: false,
            priority: None,
        }
    }

    pub fn method(&self) -> &str {
        self.method
    }

    pub fn uri(&self) -> String {
        format!("{}://{}{}", self.scheme, self.authority, self.path)
    }

    /// Add a header to the request.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Mark this request as complete (no body).
    pub fn end_of_stream(mut self) -> Self {
        self.end_of_stream = true;
        self
    }

    /// Set the request body.
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self.end_of_stream = false;
        self
    }

    /// Set the request priority.
    pub fn priority(mut self, priority: frame::Priority) -> Self {
        self.priority = Some(priority);
        self
    }

    /// Encode this request into a sequence of frames for the wire.
    pub fn encode(&self, stream_id: u32) -> Result<Vec<frame::Frame>> {
        let mut frames = Vec::new();

        // Build HPACK header block.
        let mut encoder = Encoder::new(4096);
        // RFC 7540 §8.1.2.3: method, scheme, authority must appear in order.
        let mut header_fields = vec![
            hpack::HeaderField::new(":method", self.method),
            hpack::HeaderField::new(":scheme", &*self.scheme),
            hpack::HeaderField::new(":authority", &*self.authority),
            hpack::HeaderField::new(":path", &*self.path),
        ];

        for (name, value) in &self.headers {
            header_fields.push(hpack::HeaderField::new(name.as_str(), value.as_str()));
        }

        let mut header_block = Vec::new();
        encoder.encode(&header_fields, &mut header_block);

        // Build HEADERS frame.
        let mut flags = Flags::NONE;
        flags |= Flags::END_HEADERS;
        if self.end_of_stream {
            flags |= Flags::END_STREAM;
        }

        let headers_frame = frame::HeadersFrame {
            stream_id,
            end_stream: self.end_of_stream,
            end_headers: true,
            priority: self.priority,
            header_block_fragment: header_block,
        };
        frames.push(frame::Frame::Headers(headers_frame));

        // If there's a body, add a DATA frame.
        if !self.body.is_empty() {
            let data_frame = frame::DataFrame {
                stream_id,
                data: self.body.clone(),
                end_stream: false,
            };
            frames.push(frame::Frame::Data(data_frame));
        }

        Ok(frames)
    }
}

/// A handle to send requests on a client connection.
#[derive(Debug)]
pub struct SendRequest {
    stream_id: u32,
    /// Maps request → response.
    pending: BTreeMap<u32, Vec<frame::Frame>>,
}

impl SendRequest {
    /// Create a new send request handle for the given stream.
    pub fn new(stream_id: u32) -> Self {
        SendRequest {
            stream_id,
            pending: BTreeMap::new(),
        }
    }

    /// Start a new request on this stream ID.
    pub fn send(&mut self, request: RequestBuilder) -> Result<()> {
        let stream_id = self.stream_id;
        let frames = request.encode(stream_id)?;
        self.pending.insert(stream_id, frames);
        self.stream_id += 2; // Next client stream ID must be odd.
        Ok(())
    }

    /// Get the frames queued for the given stream.
    pub fn take_frames(&mut self, stream_id: u32) -> Option<Vec<frame::Frame>> {
        self.pending.remove(&stream_id)
    }

    pub fn next_stream_id(&self) -> u32 {
        self.stream_id
    }
}

/// Parse a URI into (scheme, authority, path).
fn parse_uri(uri: &str) -> (String, String, String) {
    if let Some(slash_idx) = uri.find("://") {
        let scheme = &uri[..slash_idx];
        let rest = &uri[slash_idx + 3..];
        if let Some(slash_idx2) = rest.find('/') {
            let authority = &rest[..slash_idx2];
            let path = &rest[slash_idx2..];
            (scheme.to_string(), authority.to_string(), path.to_string())
        } else {
            (scheme.to_string(), rest.to_string(), "/".to_string())
        }
    } else {
        ("http".to_string(), String::new(), uri.to_string())
    }
}

/// Build an HPACK-encoded header block for parsing.
pub fn encode_header_block(headers: &[(String, String, bool)]) -> Vec<u8> {
    let mut encoder = Encoder::new(4096);
    let mut header_fields = Vec::new();

    for (name, value, sensitive) in headers {
        if *sensitive {
            header_fields.push(hpack::HeaderField::sensitive(name.as_str(), value.as_str()));
        } else {
            header_fields.push(hpack::HeaderField::new(name.as_str(), value.as_str()));
        }
    }

    let mut block = Vec::new();
    encoder.encode(&header_fields, &mut block);
    block
}

// --- tests ---
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_builder_parses_uri() {
        let req = RequestBuilder::new("GET", "https://www.example.com/foo?bar=1");
        assert_eq!(req.method, "GET");
        assert_eq!(req.scheme, "https");
        assert_eq!(req.authority, "www.example.com");
        assert_eq!(req.path, "/foo?bar=1");
    }

    #[test]
    fn request_builder_without_scheme() {
        let req = RequestBuilder::new("POST", "/bar");
        assert_eq!(req.scheme, "http");
        assert_eq!(req.path, "/bar");
    }

    #[test]
    fn encode_headers_only() {
        let req = RequestBuilder::new("GET", "https://example.com/")
            .header("host", "example.com")
            .end_of_stream();
        let frames = req.encode(1).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(matches!(frames[0], frame::Frame::Headers(_)));

        let headers_frame = match &frames[0] {
            frame::Frame::Headers(f) => f,
            _ => panic!("expected HEADERS"),
        };
        assert!(headers_frame.end_stream);
        assert!(headers_frame.end_headers);
        assert!(!headers_frame.header_block_fragment.is_empty());
    }

    #[test]
    fn encode_headers_with_body() {
        let req = RequestBuilder::new("POST", "https://example.com/api")
            .header("content-type", "application/json")
            .body(b"{\"key\": \"val\"}".to_vec());
        let frames = req.encode(3).unwrap();
        assert_eq!(frames.len(), 2);
        assert!(matches!(frames[0], frame::Frame::Headers(_)));
        assert!(matches!(frames[1], frame::Frame::Data(_)));

        let data_frame = match &frames[1] {
            frame::Frame::Data(f) => f,
            _ => panic!("expected DATA"),
        };
        assert_eq!(data_frame.data, b"{\"key\": \"val\"}");
        assert!(!data_frame.end_stream);
    }

    #[test]
    fn stream_ids_increment_by_two() {
        let mut req_builder = SendRequest::new(1);
        assert_eq!(req_builder.next_stream_id(), 1);

        let req = RequestBuilder::new("GET", "https://example.com/").end_of_stream();
        req_builder.send(req).unwrap();

        // Next client stream ID should be 3.
        assert_eq!(req_builder.next_stream_id(), 3);

        let req2 = RequestBuilder::new("GET", "https://example.com/other").end_of_stream();
        req_builder.send(req2).unwrap();
        assert_eq!(req_builder.next_stream_id(), 5);
    }

    #[test]
    fn send_then_take_frames() {
        let mut req_builder = SendRequest::new(1);
        let req = RequestBuilder::new("GET", "https://example.com/").end_of_stream();
        req_builder.send(req).unwrap();

        let frames = req_builder.take_frames(1).unwrap();
        assert!(!frames.is_empty());
        assert!(req_builder.take_frames(1).is_none());
    }

    #[test]
    fn headers_only_frame_is_correct() {
        let req = RequestBuilder::new("HEAD", "https://example.com/check").end_of_stream();
        let frames = req.encode(7).unwrap();
        let headers_frame = match &frames[0] {
            frame::Frame::Headers(f) => f,
            _ => panic!("expected HEADERS"),
        };
        assert_eq!(headers_frame.stream_id, 7);
        assert!(headers_frame.end_stream);
        assert!(headers_frame.end_headers);
        assert!(!headers_frame.header_block_fragment.is_empty());
    }

    #[test]
    fn encode_header_block_integration() {
        let data = vec![
            (":method".to_string(), "GET".to_string(), false),
            (":scheme".to_string(), "https".to_string(), false),
            (":authority".to_string(), "example.com".to_string(), false),
            (":path".to_string(), "/".to_string(), false),
        ];
        let block = encode_header_block(&data);
        assert!(!block.is_empty());

        // Decode and verify roundtrip.
        let mut decoder = crate::hpack::Decoder::new(4096);
        let decoded = decoder.decode(&block).unwrap();
        assert_eq!(decoded.len(), 4);
        assert_eq!(decoded[0].name, b":method");
        assert_eq!(decoded[0].value, b"GET");
        assert_eq!(decoded[1].name, b":scheme");
        assert_eq!(decoded[1].value, b"https");
    }

    #[test]
    fn hpack_roundtrip_with_authorization_header() {
        let mut encoder = Encoder::new(4096);
        let headers = vec![
            hpack::HeaderField::new(":method", "GET"),
            hpack::HeaderField::sensitive("authorization", "Bearer token"),
        ];
        let mut block = Vec::new();
        encoder.encode(&headers, &mut block);

        let mut decoder = crate::hpack::Decoder::new(4096);
        let decoded = decoder.decode(&block).unwrap();
        assert_eq!(decoded, headers);
        assert!(decoded[1].sensitive);
    }

    #[test]
    fn priority_flag_sets_flag() {
        let priority = frame::Priority {
            exclusive: true,
            dependency: 0,
            weight: 32,
        };
        let req = RequestBuilder::new("GET", "https://example.com/").priority(priority);
        let frames = req.encode(1).unwrap();
        let headers_frame = match &frames[0] {
            frame::Frame::Headers(f) => f,
            _ => panic!("expected HEADERS"),
        };
        assert!(headers_frame.priority.is_some());
    }
}
