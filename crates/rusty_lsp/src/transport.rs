//! Asynchronous LSP message framing.
//!
//! LSP frames each JSON-RPC payload with HTTP-style headers:
//!
//! ```text
//! Content-Length: 123\r\n
//! \r\n
//! {"jsonrpc":"2.0", ...}
//! ```
//!
//! These functions read and write that framing over any async byte stream, so
//! the same plumbing serves stdio, sockets, or in-memory pipes (used by the
//! integration tests). They are public so applications can build custom
//! transports — proxies, multiplexers, test harnesses — on the same wire
//! format the [`crate::Server`] uses.

use crate::error::{Error, Result};
use crate::jsonrpc::Message;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const CONTENT_LENGTH: &str = "content-length";
/// Guards against a malformed/hostile `Content-Length` exhausting memory.
const MAX_CONTENT_LENGTH: usize = 256 * 1024 * 1024;

/// Read a single framed [`Message`] from `reader`.
///
/// Returns `Ok(None)` on a clean end-of-stream (the peer closed the
/// connection at a frame boundary), and an error if the stream ends
/// mid-frame or the framing is malformed.
pub async fn read_message<R>(reader: &mut R) -> Result<Option<Message>>
where
    R: AsyncBufRead + Unpin,
{
    let Some(content_length) = read_headers(reader).await? else {
        return Ok(None);
    };

    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            Error::protocol("connection closed mid-message")
        } else {
            Error::Io(e)
        }
    })?;

    let message = serde_json::from_slice(&body)?;
    Ok(Some(message))
}

/// Read the header block, returning the parsed `Content-Length`.
///
/// `Ok(None)` signals a clean EOF before any header bytes were seen.
async fn read_headers<R>(reader: &mut R) -> Result<Option<usize>>
where
    R: AsyncBufRead + Unpin,
{
    let mut content_length: Option<usize> = None;
    let mut line = String::new();
    let mut saw_any_byte = false;

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await?;
        if bytes_read == 0 {
            // EOF. Clean only if it lands exactly on a frame boundary.
            return if saw_any_byte {
                Err(Error::protocol("connection closed inside header block"))
            } else {
                Ok(None)
            };
        }
        saw_any_byte = true;

        // A bare CRLF (or LF) terminates the header block.
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case(CONTENT_LENGTH) {
                let value = value.trim();
                let parsed = value
                    .parse::<usize>()
                    .map_err(|_| Error::protocol(format!("invalid Content-Length: {value:?}")))?;
                content_length = Some(parsed);
            }
            // Other headers (e.g. Content-Type) are accepted and ignored.
        } else {
            return Err(Error::protocol(format!(
                "malformed header line: {trimmed:?}"
            )));
        }
    }

    match content_length {
        Some(len) if len > MAX_CONTENT_LENGTH => Err(Error::protocol(format!(
            "Content-Length {len} exceeds limit"
        ))),
        Some(len) => Ok(Some(len)),
        None => Err(Error::protocol("missing Content-Length header")),
    }
}

/// Write a single framed [`Message`] to `writer` and flush it.
pub async fn write_message<W>(writer: &mut W, message: &Message) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut buf = Vec::new();
    encode_message(&mut buf, message)?;
    writer.write_all(&buf).await?;
    writer.flush().await?;
    Ok(())
}

/// Append a single framed [`Message`] to `buf` without writing it anywhere.
///
/// Lets a caller batch several messages into one buffer and issue a single
/// `write_all` + `flush` for the lot — one syscall pair instead of one per
/// message, which matters when many responses become ready back-to-back
/// under concurrent load.
pub fn encode_message(buf: &mut Vec<u8>, message: &Message) -> Result<()> {
    let body = serde_json::to_vec(message)?;
    buf.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
    buf.extend_from_slice(&body);
    Ok(())
}

/// Convenience: wrap a reader so [`read_message`] can be called repeatedly.
///
/// Equivalent to `tokio::io::BufReader::new(reader)`, re-exported so callers do
/// not need to remember that [`read_message`] requires a buffered source.
pub fn buffered<R: tokio::io::AsyncRead + Unpin>(reader: R) -> tokio::io::BufReader<R> {
    tokio::io::BufReader::new(reader)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonrpc::{Notification, RequestId};

    /// Read one message from raw bytes.
    async fn parse(bytes: &[u8]) -> Result<Option<Message>> {
        let mut reader = buffered(bytes);
        read_message(&mut reader).await
    }

    fn frame(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[tokio::test]
    async fn round_trips_a_large_multibyte_message() {
        let note = Message::Notification(Notification {
            method: "window/logMessage".to_owned(),
            params: Some(serde_json::json!({"message": "héllo 😀 ".repeat(10_000)})),
        });
        let (mut client, server) = tokio::io::duplex(1 << 20);
        write_message(&mut client, &note).await.expect("write");
        drop(client);
        let mut reader = buffered(server);
        let read = read_message(&mut reader)
            .await
            .expect("read")
            .expect("one message");
        assert_eq!(read, note);
        // ... and the stream ends cleanly at the frame boundary.
        assert!(read_message(&mut reader).await.expect("eof").is_none());
    }

    #[tokio::test]
    async fn extra_headers_and_header_case_are_tolerated() {
        let body = r#"{"jsonrpc":"2.0","method":"m"}"#;
        let bytes = format!(
            "content-type: application/vscode-jsonrpc; charset=utf-8\r\n\
             CONTENT-LENGTH: {}\r\n\r\n{body}",
            body.len()
        );
        let message = parse(bytes.as_bytes()).await.expect("ok").expect("message");
        assert!(matches!(message, Message::Notification(n) if n.method == "m"));
    }

    #[tokio::test]
    async fn lf_only_header_termination_is_accepted() {
        let body = r#"{"jsonrpc":"2.0","method":"m"}"#;
        let bytes = format!("Content-Length: {}\n\n{body}", body.len());
        assert!(parse(bytes.as_bytes()).await.expect("ok").is_some());
    }

    #[tokio::test]
    async fn missing_invalid_and_oversized_content_length_are_errors() {
        assert!(parse(b"\r\n").await.is_err()); // no Content-Length at all
        assert!(parse(b"Content-Length: nope\r\n\r\n").await.is_err());
        // Over the limit fails before any body is read.
        let huge = format!("Content-Length: {}\r\n\r\n", MAX_CONTENT_LENGTH + 1);
        assert!(parse(huge.as_bytes()).await.is_err());
    }

    #[tokio::test]
    async fn malformed_header_line_is_an_error() {
        assert!(parse(b"not a header\r\n\r\n").await.is_err());
    }

    #[tokio::test]
    async fn eof_semantics_distinguish_boundary_from_mid_frame() {
        // Clean EOF before any bytes: Ok(None).
        assert!(parse(b"").await.expect("clean").is_none());
        // EOF inside the header block: error.
        assert!(parse(b"Content-Length: 10\r\n").await.is_err());
        // EOF inside the body: error.
        let mut bytes = frame(r#"{"jsonrpc":"2.0","method":"m"}"#);
        bytes.truncate(bytes.len() - 5);
        assert!(parse(&bytes).await.is_err());
    }

    #[tokio::test]
    async fn request_ids_survive_the_wire() {
        let body = r#"{"jsonrpc":"2.0","id":"abc","method":"m"}"#;
        let message = parse(&frame(body)).await.expect("ok").expect("message");
        let Message::Request(request) = message else {
            panic!("expected request");
        };
        assert_eq!(request.id, RequestId::String("abc".to_owned()));
    }
}
