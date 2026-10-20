use std::time::Duration;

use crate::{format_timeout, parse_timeout, Code, MetadataMap, Status};

/// Splits `/package.Service/Method` into its service and method.
pub fn split_path(path: &str) -> Option<(&str, &str)> {
    let (service, method) = path.strip_prefix('/')?.split_once('/')?;
    let valid =
        |s: &str| !s.is_empty() && s.bytes().all(|b| (0x21..=0x7e).contains(&b) && b != b'/');
    (valid(service) && valid(method)).then_some((service, method))
}

/// The gRPC code for an HTTP status on a response that has no `grpc-status`.
pub fn code_from_http_status(status: u16) -> Code {
    match status {
        400 => Code::Internal,
        401 => Code::Unauthenticated,
        403 => Code::PermissionDenied,
        404 => Code::Unimplemented,
        429 | 502 | 503 | 504 => Code::Unavailable,
        _ => Code::Unknown,
    }
}

/// Whether a `content-type` is gRPC with the protobuf codec: `application/grpc`, optionally
/// `+proto`, optionally followed by parameters.
fn is_grpc_proto(content_type: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(content_type) else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("application/grpc") else {
        return false;
    };
    let rest = rest.strip_prefix("+proto").unwrap_or(rest);
    rest.is_empty() || rest.starts_with(';')
}

/// The only encoding supported. Absent means identity.
fn is_identity(encoding: &[u8]) -> bool {
    encoding.eq_ignore_ascii_case(b"identity")
}

fn lossy(value: &[u8]) -> String {
    String::from_utf8_lossy(value).into_owned()
}

/// `:scheme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// `http`.
    Http,
    /// `https`.
    Https,
}

/// The start of a call, as a header list.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestHead {
    /// `/package.Service/Method`.
    pub path: String,
    /// The `:authority`.
    pub authority: String,
    /// The `:scheme`.
    pub scheme: Scheme,
    /// The caller's deadline, sent as `grpc-timeout`.
    pub timeout: Option<Duration>,
    /// The `user-agent`.
    pub user_agent: Option<String>,
    /// Custom metadata.
    pub metadata: MetadataMap,
    /// How many received headers could not be metadata and were skipped.
    pub metadata_dropped: usize,
}

impl RequestHead {
    /// A head for `path` on `authority`: `http`, no timeout, no metadata.
    pub fn new(path: impl Into<String>, authority: impl Into<String>) -> RequestHead {
        RequestHead {
            path: path.into(),
            authority: authority.into(),
            scheme: Scheme::Http,
            timeout: None,
            user_agent: None,
            metadata: MetadataMap::new(),
            metadata_dropped: 0,
        }
    }

    /// The headers to send, pseudo-headers first.
    pub fn to_headers(&self) -> Vec<(String, Vec<u8>)> {
        let scheme = match self.scheme {
            Scheme::Http => "http",
            Scheme::Https => "https",
        };
        let mut out = vec![
            (":method".to_string(), b"POST".to_vec()),
            (":scheme".to_string(), scheme.as_bytes().to_vec()),
            (":path".to_string(), self.path.clone().into_bytes()),
            (
                ":authority".to_string(),
                self.authority.clone().into_bytes(),
            ),
            ("content-type".to_string(), b"application/grpc".to_vec()),
            ("te".to_string(), b"trailers".to_vec()),
            ("grpc-accept-encoding".to_string(), b"identity".to_vec()),
        ];
        if let Some(timeout) = self.timeout {
            out.push((
                "grpc-timeout".to_string(),
                format_timeout(timeout).into_bytes(),
            ));
        }
        if let Some(agent) = &self.user_agent {
            out.push(("user-agent".to_string(), agent.clone().into_bytes()));
        }
        out.extend(self.metadata.to_headers());
        out
    }

    /// Reads the headers of a received call, refusing one that is not a gRPC call.
    pub fn from_headers<'a>(
        headers: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    ) -> Result<RequestHead, Status> {
        let mut head = RequestHead::new("", "");
        let (mut method, mut path, mut content_type, mut encoding) = (None, None, None, None);
        let mut timeout = None;
        let mut rest = Vec::new();
        for (name, value) in headers {
            match name {
                ":method" => method = Some(value),
                ":path" => path = Some(value),
                ":authority" => head.authority = lossy(value),
                ":scheme" => {
                    head.scheme = if value == b"https" {
                        Scheme::Https
                    } else {
                        Scheme::Http
                    };
                }
                "content-type" => content_type = Some(value),
                "grpc-encoding" => encoding = Some(value),
                "grpc-timeout" => timeout = Some(value),
                "user-agent" => head.user_agent = Some(lossy(value)),
                _ => rest.push((name, value)),
            }
        }
        if method != Some(b"POST") {
            return Err(Status::internal("a gRPC call must use the POST method"));
        }
        let path = path
            .and_then(|p| std::str::from_utf8(p).ok())
            .filter(|p| split_path(p).is_some())
            .ok_or_else(|| Status::unimplemented("the path is not /package.Service/Method"))?;
        head.path = path.to_string();
        if !content_type.is_some_and(is_grpc_proto) {
            return Err(Status::unimplemented(
                "the content-type is not application/grpc",
            ));
        }
        if encoding.is_some_and(|e| !is_identity(e)) {
            return Err(Status::unimplemented(
                "only the identity message encoding is supported",
            ));
        }
        if let Some(value) = timeout {
            head.timeout = Some(
                parse_timeout(value)
                    .map_err(|e| Status::internal(format!("invalid grpc-timeout: {e}")))?,
            );
        }
        (head.metadata, head.metadata_dropped) = MetadataMap::from_headers(rest);
        Ok(head)
    }
}

/// A response's initial metadata.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResponseHead {
    /// Custom metadata.
    pub metadata: MetadataMap,
    /// How many received headers could not be metadata and were skipped.
    pub metadata_dropped: usize,
}

/// What a response's first headers amount to.
#[derive(Debug, Clone, PartialEq)]
pub enum ResponseStart {
    /// A normal start: messages and trailers follow.
    Headers(ResponseHead),
    /// A "trailers-only" response: the call is already over.
    TrailersOnly(Status),
}

impl ResponseHead {
    /// The headers to send for a successful start: `:status 200` and a content type.
    pub fn to_headers(&self) -> Vec<(String, Vec<u8>)> {
        let mut out = vec![
            (":status".to_string(), b"200".to_vec()),
            ("content-type".to_string(), b"application/grpc".to_vec()),
        ];
        out.extend(self.metadata.to_headers());
        out
    }

    /// Reads a response's first headers. A response that is not gRPC is mapped to a status.
    pub fn from_headers<'a>(
        http_status: u16,
        headers: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    ) -> Result<ResponseStart, Status> {
        let headers: Vec<_> = headers.into_iter().collect();
        let trailers_only =
            || Status::from_headers(headers.iter().copied()).map(ResponseStart::TrailersOnly);
        if http_status != 200 {
            return trailers_only().ok_or_else(|| {
                Status::new(
                    code_from_http_status(http_status),
                    format!("the server answered with HTTP status {http_status}"),
                )
            });
        }
        let content_type = headers.iter().find(|(name, _)| *name == "content-type");
        if !content_type.is_some_and(|(_, v)| is_grpc_proto(v)) {
            return Err(Status::unknown("the response is not application/grpc"));
        }
        let encoding = headers.iter().find(|(name, _)| *name == "grpc-encoding");
        if encoding.is_some_and(|(_, v)| !is_identity(v)) {
            return Err(Status::unimplemented(
                "only the identity message encoding is supported",
            ));
        }
        if let Some(start) = trailers_only() {
            return Ok(start);
        }
        let rest = headers
            .iter()
            .copied()
            .filter(|(name, _)| *name != "grpc-encoding");
        let (metadata, metadata_dropped) = MetadataMap::from_headers(rest);
        Ok(ResponseStart::Headers(ResponseHead {
            metadata,
            metadata_dropped,
        }))
    }
}
