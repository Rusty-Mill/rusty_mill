//! Transport errors as gRPC statuses.

use rusty_h2::error::ErrorCode;
use rusty_hyper::H2Error;

use crate::{Code, Status};

/// The status for an HTTP/2 error on a call.
pub(crate) fn status_from_h2(e: &H2Error) -> Status {
    let code = match e {
        H2Error::Reset(ErrorCode::RefusedStream)
        | H2Error::GoAway(_)
        | H2Error::Io(_)
        | H2Error::Closed => Code::Unavailable,
        H2Error::Reset(ErrorCode::Cancel) => Code::Cancelled,
        H2Error::Reset(ErrorCode::EnhanceYourCalm) => Code::ResourceExhausted,
        H2Error::Reset(ErrorCode::InadequateSecurity) => Code::PermissionDenied,
        H2Error::Reset(_) | H2Error::Protocol(_) => Code::Internal,
    };
    Status::new(code, e.to_string())
}

/// The status for a failure to connect.
pub(crate) fn status_from_connect(e: &rusty_hyper::Error) -> Status {
    use rusty_hyper::Error;
    let code = match e {
        Error::InvalidUrl(_) | Error::UnsupportedScheme(_) => Code::InvalidArgument,
        _ => Code::Unavailable,
    };
    Status::new(code, e.to_string())
}

#[cfg(test)]
mod tests {
    use std::io;

    use rusty_h2::error::{ErrorCode, H2Error as Wire};
    use rusty_hyper::Error as Connect;

    use super::*;
    use crate::Code;

    fn h2(e: H2Error) -> Code {
        status_from_h2(&e).code()
    }

    #[test]
    fn a_reset_maps_by_its_reason() {
        let reset = |c| h2(H2Error::Reset(c));
        assert_eq!(reset(ErrorCode::RefusedStream), Code::Unavailable);
        assert_eq!(reset(ErrorCode::Cancel), Code::Cancelled);
        assert_eq!(reset(ErrorCode::EnhanceYourCalm), Code::ResourceExhausted);
        assert_eq!(reset(ErrorCode::InadequateSecurity), Code::PermissionDenied);
        for other in [
            ErrorCode::NoError,
            ErrorCode::ProtocolError,
            ErrorCode::InternalError,
            ErrorCode::FlowControlError,
            ErrorCode::StreamClosed,
            ErrorCode::FrameSizeError,
            ErrorCode::CompressionError,
            ErrorCode::ConnectError,
            ErrorCode::Http11Required,
            ErrorCode::Unknown(99),
        ] {
            assert_eq!(reset(other), Code::Internal, "{other:?}");
        }
    }

    #[test]
    fn a_connection_that_is_going_or_gone_is_unavailable() {
        assert_eq!(h2(H2Error::GoAway(ErrorCode::NoError)), Code::Unavailable);
        assert_eq!(
            h2(H2Error::GoAway(ErrorCode::InternalError)),
            Code::Unavailable
        );
        assert_eq!(h2(H2Error::Closed), Code::Unavailable);
        let io = H2Error::Io(io::Error::from(io::ErrorKind::ConnectionReset));
        assert_eq!(h2(io), Code::Unavailable);
    }

    #[test]
    fn a_protocol_error_is_internal() {
        let e = H2Error::Protocol(Wire::Connection(ErrorCode::ProtocolError, "bad"));
        assert_eq!(h2(e), Code::Internal);
    }

    #[test]
    fn the_message_says_what_happened() {
        let s = status_from_h2(&H2Error::Reset(ErrorCode::Cancel));
        assert!(s.message().contains("reset"), "{}", s.message());
    }

    #[test]
    fn connecting_fails_as_invalid_argument_or_unavailable() {
        let code = |e: Connect| status_from_connect(&e).code();
        assert_eq!(code(Connect::InvalidUrl("x".into())), Code::InvalidArgument);
        assert_eq!(
            code(Connect::UnsupportedScheme("ftp://x".into())),
            Code::InvalidArgument
        );
        assert_eq!(
            code(Connect::Io(io::Error::from(
                io::ErrorKind::ConnectionRefused
            ))),
            Code::Unavailable
        );
        assert_eq!(code(Connect::AlpnNotH2), Code::Unavailable);
        assert_eq!(code(Connect::Timeout("connect")), Code::Unavailable);
        assert_eq!(code(Connect::H2(H2Error::Closed)), Code::Unavailable);
        assert_eq!(code(Connect::Aborted), Code::Unavailable);
    }
}
