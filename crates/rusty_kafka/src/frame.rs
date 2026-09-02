//! Kafka's wire framing: every request/response is a 4-byte big-endian
//! length prefix followed by that many bytes of message (header +
//! body). Generic over [`rusty_tokio`]'s `AsyncRead`/`AsyncWrite` so it
//! works over a real [`rusty_tokio::io::TcpStream`] and, in this
//! crate's own tests, an in-memory `rusty_tokio::io::duplex` pair.

use crate::error::ClientError;
use rusty_tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use rusty_wire::Writer;

/// Writes `payload` (an already-encoded header + body) to `io`, wrapped
/// in the 4-byte length prefix.
pub(crate) async fn write_frame<W: AsyncWrite + Unpin + Send>(
    io: &mut W,
    payload: &[u8],
) -> Result<(), ClientError> {
    let mut writer = Writer::with_capacity(4 + payload.len());
    writer.write_u32_be(payload.len() as u32);
    writer.write_bytes(payload);
    io.write_all(&writer.into_vec()).await?;
    Ok(())
}

/// Reads one length-prefixed message from `io`. Rejects a declared
/// length over `max_frame_len` before allocating a buffer for it, so a
/// corrupt or hostile length prefix can't force an unbounded
/// allocation.
pub(crate) async fn read_frame<R: AsyncRead + Unpin + Send>(
    io: &mut R,
    max_frame_len: usize,
) -> Result<Vec<u8>, ClientError> {
    // rusty_tokio's AsyncReadExt names its big-endian reader with no
    // suffix (`read_u32`) and little-endian as `read_u32_le` -- the
    // opposite convention from rusty_wire's explicit `_be`/`_le` on
    // both, used everywhere else in this crate.
    let len = io.read_u32().await?;
    if (len as usize) > max_frame_len {
        return Err(ClientError::FrameTooLarge(max_frame_len, len));
    }
    let mut buf = vec![0u8; len as usize];
    io.read_exact(&mut buf).await?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_tokio::io::duplex;

    #[rusty_tokio::test]
    async fn write_frame_then_read_frame_round_trips() {
        let (mut a, mut b) = duplex(256);
        write_frame(&mut a, b"hello").await.unwrap();
        let received = read_frame(&mut b, 1024).await.unwrap();
        assert_eq!(received, b"hello");
    }

    #[rusty_tokio::test]
    async fn read_frame_rejects_a_length_over_the_cap() {
        let (mut a, mut b) = duplex(256);
        write_frame(&mut a, b"hello").await.unwrap();
        let err = read_frame(&mut b, 2).await.unwrap_err();
        assert!(matches!(err, ClientError::FrameTooLarge(2, 5)));
    }

    #[rusty_tokio::test]
    async fn read_frame_handles_an_empty_payload() {
        let (mut a, mut b) = duplex(256);
        write_frame(&mut a, b"").await.unwrap();
        let received = read_frame(&mut b, 1024).await.unwrap();
        assert!(received.is_empty());
    }
}
