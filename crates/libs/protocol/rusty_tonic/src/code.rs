use std::fmt;

/// A gRPC status code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Code {
    /// Success.
    Ok = 0,
    /// The caller cancelled the call.
    Cancelled = 1,
    /// An unknown error, or one with no better code.
    Unknown = 2,
    /// The caller sent a bad argument.
    InvalidArgument = 3,
    /// The deadline passed before the call finished.
    DeadlineExceeded = 4,
    /// A requested entity does not exist.
    NotFound = 5,
    /// An entity the caller tried to create already exists.
    AlreadyExists = 6,
    /// The caller is not allowed to do this.
    PermissionDenied = 7,
    /// A quota or resource is exhausted.
    ResourceExhausted = 8,
    /// The system is not in a state this needs.
    FailedPrecondition = 9,
    /// The call was aborted, usually by a concurrency conflict.
    Aborted = 10,
    /// An argument was outside the valid range.
    OutOfRange = 11,
    /// The method is not implemented.
    Unimplemented = 12,
    /// An internal invariant broke.
    Internal = 13,
    /// The service is unavailable; retrying may help.
    Unavailable = 14,
    /// Data was lost or corrupted.
    DataLoss = 15,
    /// The caller has no valid credentials.
    Unauthenticated = 16,
}

impl Code {
    /// The code for a wire number. A number outside 0..=16 is [`Code::Unknown`], as the spec says.
    pub fn from_i32(n: i32) -> Code {
        match n {
            0 => Code::Ok,
            1 => Code::Cancelled,
            3 => Code::InvalidArgument,
            4 => Code::DeadlineExceeded,
            5 => Code::NotFound,
            6 => Code::AlreadyExists,
            7 => Code::PermissionDenied,
            8 => Code::ResourceExhausted,
            9 => Code::FailedPrecondition,
            10 => Code::Aborted,
            11 => Code::OutOfRange,
            12 => Code::Unimplemented,
            13 => Code::Internal,
            14 => Code::Unavailable,
            15 => Code::DataLoss,
            16 => Code::Unauthenticated,
            _ => Code::Unknown,
        }
    }

    /// The wire number.
    pub fn as_i32(self) -> i32 {
        self as i32
    }

    /// The canonical name, for example `NOT_FOUND`.
    pub fn name(self) -> &'static str {
        match self {
            Code::Ok => "OK",
            Code::Cancelled => "CANCELLED",
            Code::Unknown => "UNKNOWN",
            Code::InvalidArgument => "INVALID_ARGUMENT",
            Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
            Code::NotFound => "NOT_FOUND",
            Code::AlreadyExists => "ALREADY_EXISTS",
            Code::PermissionDenied => "PERMISSION_DENIED",
            Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
            Code::FailedPrecondition => "FAILED_PRECONDITION",
            Code::Aborted => "ABORTED",
            Code::OutOfRange => "OUT_OF_RANGE",
            Code::Unimplemented => "UNIMPLEMENTED",
            Code::Internal => "INTERNAL",
            Code::Unavailable => "UNAVAILABLE",
            Code::DataLoss => "DATA_LOSS",
            Code::Unauthenticated => "UNAUTHENTICATED",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
