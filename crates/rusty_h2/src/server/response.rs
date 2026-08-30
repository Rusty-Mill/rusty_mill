use crate::error::Result;
use crate::frame;
use crate::frame::header::Flags;
use crate::hpack;
use crate::hpack::Decoder;
use crate::stream;

/// A server-side response builder.
pub struct ResponseBuilder {
    status: u16,
    headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) end_of_stream: bool,
}

impl ResponseBuilder {
    pub fn new(status: u16) -> Self {
        ResponseBuilder {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            end_of_stream: false,
        }
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    pub fn end_of_stream(mut self) -> Self {
        self.end_of_stream = true;
        self
    }

    /// Encode this response into frames.
    pub fn encode(
        &self,
        stream_id: u32,
    ) -> Result<Vec<frame::Frame>> {
        let mut frames = Vec::new();

        let mut encoder = crate::hpack::Encoder::new(4096);
        let mut header_fields = Vec::new();

        header_fields.push(hpack::HeaderField::new(":status", &self.status.to_string()));
        for (name, value) in &self.headers {
            header_fields.push(hpack::HeaderField::new(name, value));
        }

        let mut header_block = Vec::new();
        encoder.encode(&header_fields, &mut header_block);

        let mut flags = Flags::NONE;
        flags |= Flags::END_HEADERS;
        if self.end_of_stream {
            flags |= Flags::END_STREAM;
        }

        let headers_frame = frame::HeadersFrame {
            stream_id,
            end_stream: self.end_of_stream,
            end_headers: true,
            priority: None,
            header_block_fragment: header_block,
        };
        frames.push(frame::Frame::Headers(headers_frame));

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

/// An incoming server request, decoded from frames.
pub struct IncomingRequest {
    pub stream_id: u32,
    pub method: String,
    pub scheme: String,
    pub authority: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl IncomingRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Decode a HEADERS frame (with optional body) into an IncomingRequest.
pub fn decode_request(frame: &frame::Frame, decoder: &mut Decoder) -> Result<IncomingRequest> {
    let headers_frame = match frame {
        frame::Frame::Headers(f) => f,
        _ => panic!("expected HEADERS frame"),
    };

    let decoded_headers = decoder.decode(&headers_frame.header_block_fragment)?;

    let mut method = String::new();
    let mut scheme = String::new();
    let mut authority = String::new();
    let mut path = String::new();
    let mut extra_headers = Vec::new();
    let mut body = Vec::new();

    for header in &decoded_headers {
        if header.name == b":method" {
            method += std::str::from_utf8(&header.value)?;
        } else if header.name == b":scheme" {
            scheme += std::str::from_utf8(&header.value)?;
        } else if header.name == b":authority" {
            authority += std::str::from_utf8(&header.value)?;
        } else if header.name == b":path" {
            path += std::str::from_utf8(&header.value)?;
        } else {
            extra_headers.push((
                String::from_utf8_lossy(&header.name).to_string(),
                String::from_utf8_lossy(&header.value).to_string(),
            ));
        }
    }

    Ok(IncomingRequest {
        stream_id: headers_frame.stream_id,
        method,
        scheme,
        authority,
        path,
        headers: extra_headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_builder_encodes_status() {
        let resp = ResponseBuilder::new(200)
            .header("content-type", "text/html");
        let frames = resp.encode(1).unwrap();
        assert_eq!(frames.len(), 1);

        let headers_frame = match &frames[0] {
            frame::Frame::Headers(f) => f,
            _ => panic!("expected HEADERS"),
        };
        assert!(!headers_frame.header_block_fragment.is_empty());

        let mut decoder = Decoder::new(4096);
        let decoded = decoder.decode(&headers_frame.header_block_fragment).unwrap();
        let status_header = decoded
            .iter()
            .find(|h| h.name == b":status")
            .unwrap();
        assert_eq!(status_header.value, b"200");
    }

    #[test]
    fn response_with_body() {
        let body_data = b"<html><body>Hello</body></html>";
        let resp = ResponseBuilder::new(200)
            .header("content-type", "text/html")
            .body(body_data.to_vec())
            .end_of_stream();

        let frames = resp.encode(1).unwrap();
        assert_eq!(frames.len(), 2);

        let data_frame = match &frames[1] {
            frame::Frame::Data(f) => f,
            _ => panic!("expected DATA"),
        };
        assert_eq!(data_frame.data, body_data);
        assert!(!data_frame.end_stream);
    }

    #[test]
    fn response_headers_only_frame() {
        let resp = ResponseBuilder::new(404)
            .end_of_stream();
        let frames = resp.encode(3).unwrap();

        let headers_frame = match &frames[0] {
            frame::Frame::Headers(f) => f,
            _ => panic!("expected HEADERS"),
        };
        assert_eq!(headers_frame.stream_id, 3);
        assert!(headers_frame.end_stream);
        assert!(headers_frame.end_headers);
    }

    #[test]
    fn response_headers_with_content_length() {
        let resp = ResponseBuilder::new(200)
            .header("content-type", "application/json")
            .header("content-length", "12")
            .body(b"{\"ok\": true}".to_vec())
            .end_of_stream();

        let frames = resp.encode(5).unwrap();
        let headers_frame = match &frames[0] {
            frame::Frame::Headers(f) => f,
            _ => panic!("expected HEADERS"),
        };

        let mut decoder = Decoder::new(4096);
        let decoded = decoder.decode(&headers_frame.header_block_fragment).unwrap();
        let content_length = decoded
            .iter()
            .find(|h| h.name == b"content-length")
            .unwrap();
        assert_eq!(content_length.value, b"12");
    }

    #[test]
    fn incoming_request_decode_headers() {
        let method_val = b":method";
        let scheme_val = b":scheme";
        let authority_val = b":authority";
        let path_val = b":path";

        let mut encoder = crate::hpack::Encoder::new(4096);
        let headers = vec![
            hpack::HeaderField::new(":method", "GET"),
            hpack::HeaderField::new(":scheme", "https"),
            hpack::HeaderField::new(":authority", "example.com"),
            hpack::HeaderField::new(":path", "/test"),
        ];
        let mut block = Vec::new();
        encoder.encode(&headers, &mut block);

        let headers_frame = frame::HeadersFrame {
            stream_id: 1,
            end_stream: false,
            end_headers: true,
            priority: None,
            header_block_fragment: block,
        };
        let frame = frame::Frame::Headers(headers_frame);

        let mut decoder = Decoder::new(4096);
        let req = decode_request(&frame, &mut decoder).unwrap();
        assert_eq!(req.stream_id, 1);
        assert_eq!(req.method, "GET");
        assert_eq!(req.scheme, "https");
        assert_eq!(req.authority, "example.com");
        assert_eq!(req.path, "/test");
    }

    #[test]
    fn incoming_request_with_extra_headers() {
        let mut encoder = crate::hpack::Encoder::new(4096);
        let headers = vec![
            hpack::HeaderField::new(":method", "POST"),
            hpack::HeaderField::new(":scheme", "http"),
            hpack::HeaderField::new(":authority", "localhost:8080"),
            hpack::HeaderField::new(":path", "/api/data"),
            hpack::HeaderField::new("content-type", "application/json"),
            hpack::HeaderField::new("x-custom", "value"),
        ];
        let mut block = Vec::new();
        encoder.encode(&headers, &mut block);

        let headers_frame = frame::HeadersFrame {
            stream_id: 3,
            end_stream: false,
            end_headers: true,
            priority: None,
            header_block_fragment: block,
        };
        let frame = frame::Frame::Headers(headers_frame);

        let mut decoder = Decoder::new(4096);
        let req = decode_request(&frame, &mut decoder).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/data");
        assert_eq!(req.headers.len(), 2);
        assert_eq!(req.header("content-type"), Some("application/json"));
        assert_eq!(req.header("x-custom"), Some("value"));
    }

    #[test]
    fn response_header_name_case_insensitive_lookup() {
        let resp = ResponseBuilder::new(200)
            .header("Content-Length", "512")
            .end_of_stream();

        let frames = resp.encode(1).unwrap();
        let headers_frame = match &frames[0] {
            frame::Frame::Headers(f) => f,
            _ => panic!("expected HEADERS"),
        };

        let mut decoder = Decoder::new(4096);
        let decoded = decoder.decode(&headers_frame.header_block_fragment).unwrap();
        let content_length = decoded
            .iter()
            .find(|h| h.name == b"content-length")
            .unwrap();
        assert_eq!(content_length.value, b"512");
    }

    #[test]
    fn response_encoder_roundtrip() {
        let resp = ResponseBuilder::new(200)
            .header("content-type", "text/html; charset=utf-8")
            .header("cache-control", "no-cache")
            .body(b"Hello, world!".to_vec())
            .end_of_stream();

        let frames = resp.encode(9).unwrap();
        let headers_frame = match &frames[0] {
            frame::Frame::Headers(f) => f,
            _ => panic!("expected HEADERS"),
        };

        let mut decoder = Decoder::new(4096);
        let decoded = decoder.decode(&headers_frame.header_block_fragment).unwrap();
        assert!(!decoded.is_empty());

        let content_type = decoded
            .iter()
            .find(|h| h.name == b"content-type")
            .unwrap();
        assert_eq!(content_type.value, b"text/html; charset=utf-8");
    }
}
