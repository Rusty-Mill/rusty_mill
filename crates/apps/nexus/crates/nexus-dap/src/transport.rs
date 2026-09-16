//! Framed DAP messages over stdio.
//!
//! DAP wraps each JSON message in a Content-Length-prefixed envelope
//! identical to LSP's:
//!
//! ```text
//! Content-Length: 64\r\n
//! \r\n
//! {"seq":1,"type":"request","command":"initialize","arguments":{…}}
//! ```
//!
//! Optional `Content-Type` may appear and is ignored. The body is JSON
//! whose top-level `"type"` discriminator distinguishes request /
//! response / event — see [`crate::protocol`].
//!
//! This module is wire-layer only. Seq allocation, request/response
//! correlation, and event dispatch live in [`crate::client`].

use std::io;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::protocol::ProtocolMessage;

/// Errors raised by the transport.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// I/O failure on the underlying stream.
    #[error("io: {0}")]
    Io(#[from] io::Error),
    /// Header was missing or malformed (no `Content-Length`, garbage
    /// bytes, …).
    #[error("malformed header: {0}")]
    BadHeader(String),
    /// Body did not parse as DAP JSON.
    #[error("malformed body: {0}")]
    BadBody(#[from] serde_json::Error),
    /// Stream closed mid-message or before any bytes appeared.
    #[error("stream closed unexpectedly")]
    Eof,
}

/// Read one framed DAP message from `reader`.
///
/// # Errors
/// - [`TransportError::Eof`] when the stream returns 0 bytes before any
///   header bytes are seen — the canonical "child exited" path.
/// - [`TransportError::BadHeader`] for a malformed prelude.
/// - [`TransportError::BadBody`] when the body is not valid DAP JSON.
/// - [`TransportError::Io`] for read failures.
pub async fn read_message<R>(reader: &mut BufReader<R>) -> Result<ProtocolMessage, TransportError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    // 16 MiB ceiling — matches the LSP host's limit. A single DAP
    // message larger than this is almost certainly a protocol bug or
    // a runaway `variables` reply.
    const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
    // Guards the header block itself against a peer that never
    // terminates a header line (or never terminates the block at all),
    // independent of and prior to the `Content-Length` cap above.
    const MAX_HEADER_BYTES: usize = 8 * 1024;

    let mut content_length: Option<usize> = None;
    let mut header_bytes = 0usize;
    loop {
        let remaining = MAX_HEADER_BYTES.saturating_sub(header_bytes);
        if remaining == 0 {
            return Err(TransportError::BadHeader(format!(
                "header block exceeds {MAX_HEADER_BYTES}-byte cap"
            )));
        }
        let mut line = String::new();
        let n = (&mut *reader)
            .take(remaining as u64)
            .read_line(&mut line)
            .await?;
        if n == 0 {
            if header_bytes == 0 {
                return Err(TransportError::Eof);
            }
            return Err(TransportError::BadHeader(
                "stream closed mid-header".to_string(),
            ));
        }
        if n == remaining && !line.ends_with('\n') {
            // The artificial cap above (not a real newline) is what
            // stopped this read: the line itself is oversized.
            return Err(TransportError::BadHeader(format!(
                "header line exceeds {MAX_HEADER_BYTES}-byte cap"
            )));
        }
        header_bytes += n;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            return Err(TransportError::BadHeader(format!(
                "no colon in '{trimmed}'"
            )));
        };
        let key = key.trim();
        let value = value.trim();
        if key.eq_ignore_ascii_case("Content-Length") {
            content_length = Some(value.parse().map_err(|_| {
                TransportError::BadHeader(format!("non-numeric Content-Length '{value}'"))
            })?);
        }
        // Other headers (`Content-Type`) are accepted and ignored.
    }
    let Some(len) = content_length else {
        return Err(TransportError::BadHeader(
            "missing Content-Length".to_string(),
        ));
    };
    if len > MAX_BODY_BYTES {
        return Err(TransportError::BadHeader(format!(
            "Content-Length {len} exceeds {MAX_BODY_BYTES}-byte cap"
        )));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    let msg: ProtocolMessage = serde_json::from_slice(&buf)?;
    Ok(msg)
}

/// Write one framed DAP message to `writer` and flush.
///
/// # Errors
/// - [`TransportError::Io`] on write failure.
/// - [`TransportError::BadBody`] if serialisation fails — defensive,
///   `serde_json` never fails for our types.
pub async fn write_message<W>(writer: &mut W, msg: &ProtocolMessage) -> Result<(), TransportError>
where
    W: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(msg)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ProtocolEvent, ProtocolRequest, ProtocolResponse};
    use tokio::io::BufReader;

    #[tokio::test]
    async fn round_trips_a_request() {
        let req = ProtocolMessage::Request(ProtocolRequest {
            seq: 1,
            command: "initialize".to_string(),
            arguments: Some(serde_json::json!({"clientID": "nexus"})),
        });
        let mut buf: Vec<u8> = Vec::new();
        write_message(&mut buf, &req).await.unwrap();
        let mut reader = BufReader::new(buf.as_slice());
        let parsed = read_message(&mut reader).await.unwrap();
        let ProtocolMessage::Request(r) = parsed else {
            panic!("expected Request, got {parsed:?}")
        };
        assert_eq!(r.command, "initialize");
        assert_eq!(r.seq, 1);
    }

    #[tokio::test]
    async fn parses_response() {
        let body = br#"{"seq":2,"type":"response","request_seq":1,"success":true,"command":"launch","body":{"ok":true}}"#;
        let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        framed.extend_from_slice(body);
        let mut reader = BufReader::new(framed.as_slice());
        let parsed = read_message(&mut reader).await.unwrap();
        let ProtocolMessage::Response(r) = parsed else {
            panic!("expected Response, got {parsed:?}")
        };
        assert_eq!(r.request_seq, 1);
        assert!(r.success);
        assert_eq!(r.command, "launch");
        assert_eq!(r.body.unwrap()["ok"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn parses_event() {
        let body = br#"{"seq":3,"type":"event","event":"stopped","body":{"reason":"breakpoint","threadId":1}}"#;
        let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        framed.extend_from_slice(body);
        let mut reader = BufReader::new(framed.as_slice());
        let parsed = read_message(&mut reader).await.unwrap();
        let ProtocolMessage::Event(e) = parsed else {
            panic!("expected Event, got {parsed:?}")
        };
        assert_eq!(e.event, "stopped");
        assert_eq!(e.body.unwrap()["reason"], serde_json::json!("breakpoint"));
    }

    #[tokio::test]
    async fn ignores_content_type_header() {
        let body = br#"{"seq":1,"type":"event","event":"output","body":{"category":"stdout","output":"hi\n"}}"#;
        let mut framed = format!(
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n\
             Content-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        framed.extend_from_slice(body);
        let mut reader = BufReader::new(framed.as_slice());
        let parsed = read_message(&mut reader).await.unwrap();
        assert!(matches!(parsed, ProtocolMessage::Event(_)));
    }

    #[tokio::test]
    async fn eof_before_any_byte_returns_eof() {
        let mut reader = BufReader::new(&[][..]);
        let err = read_message(&mut reader).await.unwrap_err();
        assert!(matches!(err, TransportError::Eof));
    }

    #[tokio::test]
    async fn missing_content_length_errors() {
        let buf = b"X-Foo: bar\r\n\r\n{}".to_vec();
        let mut reader = BufReader::new(buf.as_slice());
        let err = read_message(&mut reader).await.unwrap_err();
        assert!(matches!(err, TransportError::BadHeader(_)));
    }

    #[tokio::test]
    async fn oversized_message_rejected() {
        let buf = b"Content-Length: 99999999999\r\n\r\n".to_vec();
        let mut reader = BufReader::new(buf.as_slice());
        let err = read_message(&mut reader).await.unwrap_err();
        match err {
            TransportError::BadHeader(s) => assert!(s.contains("cap")),
            other => panic!("expected BadHeader, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn malformed_body_errors() {
        let body = br#"{"seq":1,"type":"not_a_known_kind"}"#;
        let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        framed.extend_from_slice(body);
        let mut reader = BufReader::new(framed.as_slice());
        let err = read_message(&mut reader).await.unwrap_err();
        assert!(matches!(err, TransportError::BadBody(_)));
    }

    #[tokio::test]
    async fn round_trips_event_payload() {
        let event = ProtocolMessage::Event(ProtocolEvent {
            seq: 7,
            event: "thread".to_string(),
            body: Some(serde_json::json!({"reason": "started", "threadId": 2})),
        });
        let mut buf = Vec::new();
        write_message(&mut buf, &event).await.unwrap();
        let mut reader = BufReader::new(buf.as_slice());
        let parsed = read_message(&mut reader).await.unwrap();
        let ProtocolMessage::Event(e) = parsed else {
            panic!("expected Event")
        };
        assert_eq!(e.event, "thread");
    }

    #[tokio::test]
    async fn round_trips_response_payload() {
        let resp = ProtocolMessage::Response(ProtocolResponse {
            seq: 4,
            request_seq: 2,
            success: false,
            command: "launch".to_string(),
            message: Some("no such file".to_string()),
            body: None,
        });
        let mut buf = Vec::new();
        write_message(&mut buf, &resp).await.unwrap();
        let mut reader = BufReader::new(buf.as_slice());
        let parsed = read_message(&mut reader).await.unwrap();
        let ProtocolMessage::Response(r) = parsed else {
            panic!("expected Response")
        };
        assert!(!r.success);
        assert_eq!(r.message.as_deref(), Some("no such file"));
    }

    /// A reader that never produces a `\n` and never reaches EOF -- the
    /// shape of a hostile/broken peer that would otherwise grow the
    /// header-line buffer without bound.
    struct EndlessLine;

    impl tokio::io::AsyncRead for EndlessLine {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let fill = vec![b'a'; buf.remaining()];
            buf.put_slice(&fill);
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn oversized_unterminated_header_line_is_rejected_not_unbounded() {
        let mut reader = BufReader::new(EndlessLine);
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(5), read_message(&mut reader))
                .await;
        let message =
            result.expect("read_message must not hang reading an unterminated header line");
        assert!(
            matches!(message, Err(TransportError::BadHeader(_))),
            "an unterminated, oversized header line must be rejected"
        );
    }
}
