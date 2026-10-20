use std::collections::BTreeMap;
use std::fmt;

use rusty_prost::rpc::{ErrorInfo, Status as RpcStatus};
use rusty_prost::wkt::Any;
use rusty_prost::Message;

use crate::{Code, MetadataMap};

/// The outcome of a call: a code, a message, typed details and trailing metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    code: Code,
    message: String,
    details: Vec<Any>,
    metadata: MetadataMap,
}

macro_rules! constructors {
    ($($(#[$doc:meta])* $name:ident => $code:ident),* $(,)?) => {
        $($(#[$doc])* pub fn $name(message: impl Into<String>) -> Status {
            Status::new(Code::$code, message)
        })*
    };
}

impl Status {
    /// A status with no details or metadata.
    pub fn new(code: Code, message: impl Into<String>) -> Status {
        Status {
            code,
            message: message.into(),
            details: Vec::new(),
            metadata: MetadataMap::new(),
        }
    }

    /// `OK` with no message.
    pub fn ok() -> Status {
        Status::new(Code::Ok, "")
    }

    constructors! {
        /// `CANCELLED`.
        cancelled => Cancelled,
        /// `UNKNOWN`.
        unknown => Unknown,
        /// `INVALID_ARGUMENT`.
        invalid_argument => InvalidArgument,
        /// `DEADLINE_EXCEEDED`.
        deadline_exceeded => DeadlineExceeded,
        /// `NOT_FOUND`.
        not_found => NotFound,
        /// `ALREADY_EXISTS`.
        already_exists => AlreadyExists,
        /// `PERMISSION_DENIED`.
        permission_denied => PermissionDenied,
        /// `RESOURCE_EXHAUSTED`.
        resource_exhausted => ResourceExhausted,
        /// `FAILED_PRECONDITION`.
        failed_precondition => FailedPrecondition,
        /// `ABORTED`.
        aborted => Aborted,
        /// `OUT_OF_RANGE`.
        out_of_range => OutOfRange,
        /// `UNIMPLEMENTED`.
        unimplemented => Unimplemented,
        /// `INTERNAL`.
        internal => Internal,
        /// `UNAVAILABLE`.
        unavailable => Unavailable,
        /// `DATA_LOSS`.
        data_loss => DataLoss,
        /// `UNAUTHENTICATED`.
        unauthenticated => Unauthenticated,
    }

    /// A status carrying a `google.rpc.ErrorInfo` detail.
    pub fn with_error_info(
        code: Code,
        message: impl Into<String>,
        reason: impl Into<String>,
        domain: impl Into<String>,
        metadata: BTreeMap<String, String>,
    ) -> Status {
        let info = ErrorInfo {
            reason: reason.into(),
            domain: domain.into(),
            metadata,
        };
        // Packing fails only past the format's 2 GiB limit; the status then has no detail.
        let details = Any::pack(&info).into_iter().collect();
        Status::new(code, message).with_details(details)
    }

    /// Replaces the details.
    pub fn with_details(mut self, details: Vec<Any>) -> Status {
        self.details = details;
        self
    }

    /// Replaces the trailing metadata.
    pub fn with_metadata(mut self, metadata: MetadataMap) -> Status {
        self.metadata = metadata;
        self
    }

    /// The code.
    pub fn code(&self) -> Code {
        self.code
    }

    /// The message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The typed details.
    pub fn details(&self) -> &[Any] {
        &self.details
    }

    /// The trailing metadata.
    pub fn metadata(&self) -> &MetadataMap {
        &self.metadata
    }

    /// The first detail that is an `ErrorInfo`. A detail that names `ErrorInfo` but does not
    /// decode is skipped.
    pub fn error_info(&self) -> Option<ErrorInfo> {
        self.details
            .iter()
            .find_map(|d| d.unpack::<ErrorInfo>().ok().flatten())
    }

    /// The trailers for this status: `grpc-status`, `grpc-message` (percent-encoded, when not
    /// empty), `grpc-status-details-bin` (when there are details), then the metadata.
    pub fn to_trailers(&self) -> Vec<(String, Vec<u8>)> {
        let mut out = vec![(
            "grpc-status".to_string(),
            self.code.as_i32().to_string().into_bytes(),
        )];
        if !self.message.is_empty() {
            out.push((
                "grpc-message".to_string(),
                encode_message_text(&self.message),
            ));
        }
        if self.code != Code::Ok && !self.details.is_empty() {
            let rpc = RpcStatus {
                code: self.code.as_i32(),
                message: self.message.clone(),
                details: self.details.clone(),
            };
            // Encoding fails only past the format's 2 GiB limit; the header is then left out.
            if let Ok(bytes) = rpc.encode_to_vec() {
                let b64 = rusty_base64::encode_standard(&bytes);
                out.push((
                    "grpc-status-details-bin".to_string(),
                    b64.trim_end_matches('=').as_bytes().to_vec(),
                ));
            }
        }
        out.extend(self.metadata.to_headers());
        out
    }

    /// The status in a header list, or `None` if there is no `grpc-status`. An unparseable
    /// `grpc-status` is `UNKNOWN`. A malformed `grpc-status-details-bin` is dropped: the call
    /// keeps the code and message the peer sent. Metadata the peer sent that cannot be
    /// metadata is skipped.
    pub fn from_headers<'a>(
        headers: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    ) -> Option<Status> {
        let mut code = None;
        let mut message = String::new();
        let mut details_header = None;
        let mut rest = Vec::new();
        for (name, value) in headers {
            match name {
                "grpc-status" => code = Some(parse_code(value)),
                "grpc-message" => {
                    message = rusty_percent::decode(&String::from_utf8_lossy(value)).into_owned();
                }
                "grpc-status-details-bin" => details_header = Some(value),
                _ => rest.push((name, value)),
            }
        }
        let code = code?;
        let details = if code == Code::Ok {
            Vec::new()
        } else {
            details_header
                .and_then(|value| decode_details(value, code))
                .unwrap_or_default()
        };
        let (metadata, _skipped) = MetadataMap::from_headers(rest);
        Some(Status {
            code,
            message,
            details,
            metadata,
        })
    }
}

/// The `grpc-status` value: decimal digits. Anything else, or a number past the codes, is
/// `UNKNOWN`.
fn parse_code(value: &[u8]) -> Code {
    let digits = !value.is_empty() && value.iter().all(u8::is_ascii_digit);
    let number = digits
        .then(|| std::str::from_utf8(value).ok()?.parse::<i32>().ok())
        .flatten();
    number.map_or(Code::Unknown, Code::from_i32)
}

/// The details of a `grpc-status-details-bin` value, or `None` if it is not base64 of a
/// `google.rpc.Status`.
fn decode_details(value: &[u8], outer_code: Code) -> Option<Vec<Any>> {
    let (map, _) = MetadataMap::from_headers([("x-details-bin", value)]);
    let raw = map.get_bin("x-details-bin")?;
    let status = RpcStatus::decode(raw).ok()?;
    (status.code == outer_code.as_i32()).then_some(status.details)
}

/// Percent-encodes everything outside printable ASCII, and `%` itself, as the spec says.
fn encode_message_text(message: &str) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::with_capacity(message.len());
    for b in message.bytes() {
        if (0x20..=0x7e).contains(&b) && b != b'%' {
            out.push(b);
        } else {
            out.extend_from_slice(&[b'%', HEX[usize::from(b >> 4)], HEX[usize::from(b & 0x0f)]]);
        }
    }
    out
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.message.is_empty() {
            write!(f, "{}", self.code)
        } else {
            write!(f, "{}: {}", self.code, self.message)
        }
    }
}

impl std::error::Error for Status {}
