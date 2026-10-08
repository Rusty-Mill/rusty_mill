//! Which alert each refusal sends — stage 4b-vi.
//!
//! The peer's only explanation for a dropped connection is the alert it gets,
//! and BoringSSL's suite (BoGo) checks the description for each fault. The
//! table is the contract: a refusal that maps to the wrong alert is still a
//! refusal, but it tells the peer the wrong thing, and tells an observer which
//! of two faults it was when RFC 8446 §6 asks implementations not to.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::client::{Alert, AlertDescription, AlertLevel, ClientError};
use rusty_tls::handrolled::handshake::{HandshakeError, HandshakeType};
use rusty_tls::handrolled::limits::Flood;
use rusty_tls::handrolled::path::PathError;
use rusty_tls::handrolled::record::{ContentType, RecordError};
use rusty_tls::handrolled::server::ServerError;

fn alert(level: AlertLevel, description: u8) -> Alert {
    Alert {
        level,
        description: AlertDescription(description),
    }
}

#[test]
fn what_a_client_tells_a_server() {
    use AlertDescription as A;
    let unexpected = ClientError::UnexpectedMessage {
        expected: "x",
        got: HandshakeType::Finished,
    };
    let cases: Vec<(ClientError, Option<A>)> = vec![
        // Shape wrong: decode_error. Value wrong: illegal_parameter.
        (
            ClientError::Handshake(HandshakeError::Empty("x")),
            Some(A::DECODE_ERROR),
        ),
        (
            ClientError::Handshake(HandshakeError::DuplicateExtension(1)),
            Some(A::DECODE_ERROR),
        ),
        (
            ClientError::Handshake(HandshakeError::UnexpectedCompression),
            Some(A::ILLEGAL_PARAMETER),
        ),
        (
            ClientError::MalformedRetryRequest(HandshakeError::DuplicateExtension(1)),
            Some(A::ILLEGAL_PARAMETER),
        ),
        (
            ClientError::MalformedRetryRequest(HandshakeError::Empty("x")),
            Some(A::DECODE_ERROR),
        ),
        // The wrong message, or the right one in the wrong place.
        (unexpected, Some(A::UNEXPECTED_MESSAGE)),
        (
            ClientError::UnexpectedContentType(ContentType::Handshake),
            Some(A::UNEXPECTED_MESSAGE),
        ),
        (
            ClientError::UnexpectedChangeCipherSpec,
            Some(A::UNEXPECTED_MESSAGE),
        ),
        (
            ClientError::Flood(Flood::EmptyRecords),
            Some(A::UNEXPECTED_MESSAGE),
        ),
        (
            ClientError::RepeatedHelloRetryRequest,
            Some(A::UNEXPECTED_MESSAGE),
        ),
        // Records.
        (
            ClientError::Record(RecordError::FragmentTooLong { len: 1, max: 0 }),
            Some(A::RECORD_OVERFLOW),
        ),
        (
            ClientError::Record(RecordError::EncryptedFragmentTooLong { len: 1 }),
            Some(A::RECORD_OVERFLOW),
        ),
        (
            ClientError::Record(RecordError::Decrypt),
            Some(A::BAD_RECORD_MAC),
        ),
        (
            ClientError::Record(RecordError::UnexpectedOuterType(22)),
            Some(A::BAD_RECORD_MAC),
        ),
        (
            ClientError::Record(RecordError::Truncated { len: 0, min: 5 }),
            Some(A::DECODE_ERROR),
        ),
        // Extensions.
        (
            ClientError::UnofferedExtension(1234),
            Some(A::UNSUPPORTED_EXTENSION),
        ),
        (ClientError::MissingKeyShare, Some(A::MISSING_EXTENSION)),
        (ClientError::EmptyRetryRequest, Some(A::ILLEGAL_PARAMETER)),
        // Versions and the downgrade sentinel.
        (ClientError::NotTls13, Some(A::PROTOCOL_VERSION)),
        (ClientError::NotTls12(0x0301), Some(A::PROTOCOL_VERSION)),
        (ClientError::DowngradeDetected, Some(A::ILLEGAL_PARAMETER)),
        // Authentication.
        (ClientError::BadFinished, Some(A::DECRYPT_ERROR)),
        (
            ClientError::Path(PathError::Expired { not_after: 0 }),
            Some(A::CERTIFICATE_EXPIRED),
        ),
        (
            ClientError::Path(PathError::NoPathToTrustAnchor),
            Some(A::UNKNOWN_CA),
        ),
        (
            ClientError::Path(PathError::KeyUsageForbidsSigning),
            Some(A::BAD_CERTIFICATE),
        ),
        (
            ClientError::MissingExtendedMasterSecret,
            Some(A::HANDSHAKE_FAILURE),
        ),
        // A warning TLS 1.3 forbids is a malformed alert; a level nobody
        // defined is a value the peer made up.
        (
            ClientError::BadAlert(alert(AlertLevel::Warning, 40)),
            Some(A::DECODE_ERROR),
        ),
        (
            ClientError::BadAlert(alert(AlertLevel::Unknown(7), 40)),
            Some(A::ILLEGAL_PARAMETER),
        ),
        // Nothing to say: the peer already did, or there is nobody left.
        (ClientError::PeerAlert(alert(AlertLevel::Fatal, 40)), None),
        (ClientError::Failed, None),
    ];
    for (error, want) in cases {
        assert_eq!(error.alert(), want, "{error:?}");
    }
}

#[test]
fn what_a_server_tells_a_client() {
    use AlertDescription as A;
    let cases: Vec<(ServerError, Option<A>)> = vec![
        (
            ServerError::UnexpectedMessage {
                expected: "x",
                got: HandshakeType::Finished,
            },
            Some(A::UNEXPECTED_MESSAGE),
        ),
        (
            ServerError::UnexpectedContentType(ContentType::ApplicationData),
            Some(A::UNEXPECTED_MESSAGE),
        ),
        (
            ServerError::UnexpectedChangeCipherSpec,
            Some(A::UNEXPECTED_MESSAGE),
        ),
        (
            ServerError::Flood(Flood::WarningAlerts),
            Some(A::UNEXPECTED_MESSAGE),
        ),
        (
            ServerError::Handshake(HandshakeError::Empty("x")),
            Some(A::DECODE_ERROR),
        ),
        (
            ServerError::Handshake(HandshakeError::DuplicateKeyShare(0x001d)),
            Some(A::ILLEGAL_PARAMETER),
        ),
        (
            ServerError::Record(RecordError::FragmentTooLong { len: 1, max: 0 }),
            Some(A::RECORD_OVERFLOW),
        ),
        (
            ServerError::Record(RecordError::Decrypt),
            Some(A::BAD_RECORD_MAC),
        ),
        (
            ServerError::MissingExtension(10),
            Some(A::MISSING_EXTENSION),
        ),
        (ServerError::NotTls13, Some(A::PROTOCOL_VERSION)),
        (ServerError::NotTls12(0x0301), Some(A::PROTOCOL_VERSION)),
        (ServerError::NoSharedCipherSuite, Some(A::HANDSHAKE_FAILURE)),
        (ServerError::BadFinished, Some(A::DECRYPT_ERROR)),
        (
            ServerError::InappropriateFallback,
            Some(A::INAPPROPRIATE_FALLBACK),
        ),
        (
            ServerError::ClientCertificateRequired,
            Some(A::CERTIFICATE_REQUIRED),
        ),
        (
            ServerError::BadAlert(alert(AlertLevel::Unknown(66), 10)),
            Some(A::ILLEGAL_PARAMETER),
        ),
        (
            ServerError::BadAlert(alert(AlertLevel::Warning, 40)),
            Some(A::DECODE_ERROR),
        ),
        (ServerError::PeerAlert(alert(AlertLevel::Fatal, 40)), None),
        (ServerError::Failed, None),
    ];
    for (error, want) in cases {
        assert_eq!(error.alert(), want, "{error:?}");
    }
}
