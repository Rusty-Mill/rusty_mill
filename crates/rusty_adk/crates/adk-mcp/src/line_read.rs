//! Bounded-line reading shared by MCP's newline-delimited JSON-RPC
//! transports (the stdio server and the stdio client).
//!
//! `tokio::io::AsyncBufReadExt::lines()` (and `.next_line()`) has no
//! maximum line length: an unterminated line from a misbehaving peer or
//! subprocess grows the internal buffer without bound. Wrapping the
//! reader in `.take(MAX_LINE_BYTES)` per read attempt caps that growth
//! at the source, the same way this workspace's Content-Length-framed
//! transports (`nexus_lsp`/`nexus_dap`) cap their header and body reads.

use adk_core::{AdkError, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};

/// 16 MiB ceiling per line. A single JSON-RPC message larger than this
/// is almost certainly a protocol bug or a misbehaving peer; we'd
/// rather fail the read than grow the buffer without bound.
pub(crate) const MAX_LINE_BYTES: u64 = 16 * 1024 * 1024;

/// Reads one newline-terminated line from `reader`, capped at
/// [`MAX_LINE_BYTES`].
///
/// Returns `Ok(None)` on a clean EOF before any bytes are read, mirroring
/// `Lines::next_line`'s end-of-stream behavior.
///
/// # Errors
/// Returns [`AdkError::Other`] if [`MAX_LINE_BYTES`] is reached before a
/// newline appears, and propagates any underlying I/O error.
pub(crate) async fn read_capped_line<R>(reader: &mut BufReader<R>) -> Result<Option<String>>
where
    R: AsyncRead + Unpin,
{
    let mut line = String::new();
    // `.take()` bounds the underlying read itself, not just a check after
    // the fact: `read_line` can never pull more than `MAX_LINE_BYTES` out
    // of `reader` for this call, no matter how long the peer's line is.
    let n = (&mut *reader)
        .take(MAX_LINE_BYTES)
        .read_line(&mut line)
        .await?;
    if n == 0 {
        return Ok(None);
    }
    if n as u64 == MAX_LINE_BYTES && !line.ends_with('\n') {
        return Err(AdkError::Other(format!(
            "MCP line exceeds {MAX_LINE_BYTES}-byte cap"
        )));
    }
    Ok(Some(line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn a_normal_line_is_read_in_full() {
        let (mut writer, reader_half) = tokio::io::duplex(4096);
        writer.write_all(b"hello\n").await.unwrap();
        drop(writer);
        let mut reader = BufReader::new(reader_half);
        let line = read_capped_line(&mut reader).await.unwrap();
        assert_eq!(line.as_deref(), Some("hello\n"));
    }

    #[tokio::test]
    async fn eof_before_any_bytes_reports_no_line() {
        let (writer, reader_half) = tokio::io::duplex(4096);
        drop(writer);
        let mut reader = BufReader::new(reader_half);
        let line = read_capped_line(&mut reader).await.unwrap();
        assert!(line.is_none());
    }

    #[tokio::test]
    async fn an_unterminated_line_past_the_cap_is_rejected_not_buffered_forever() {
        // A misbehaving peer sends more than the cap with no newline in
        // sight. Pre-fix, `AsyncBufReadExt::lines()` would keep growing
        // its internal buffer forever chasing that newline; the capped
        // reader must fail once the limit is hit instead.
        let payload_len = MAX_LINE_BYTES as usize + 1;
        let (mut writer, reader_half) = tokio::io::duplex(payload_len + 4096);
        writer.write_all(&vec![b'a'; payload_len]).await.unwrap();
        drop(writer);

        let mut reader = BufReader::new(reader_half);
        let result = read_capped_line(&mut reader).await;
        assert!(
            result.is_err(),
            "expected the oversized, unterminated line to be rejected"
        );
    }
}
