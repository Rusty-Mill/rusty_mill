//! DERP frame codec, mirroring Go `derp/derp.go` (see PROTOCOL.md).
//!
//! Frames are `[1B type][4B big-endian length][payload]`. Reading is
//! panic-free on arbitrary input and bounds every length against a caller
//! supplied maximum.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// 8-byte DERP magic: `"DERP"` + the key emoji (U+1F511).
pub const MAGIC: &[u8; 8] = b"DERP\xf0\x9f\x94\x91";

/// The DERP protocol version this client speaks.
pub const PROTOCOL_VERSION: i64 = 2;

/// Maximum packet size DERP will relay (64 KiB).
pub const MAX_PACKET_SIZE: usize = 64 << 10;

/// Upper bound on any frame we are willing to read: the largest packet plus
/// generous room for framing/key prefixes.
pub const MAX_FRAME_SIZE: u32 = (MAX_PACKET_SIZE as u32) + 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    ServerKey,
    ClientInfo,
    ServerInfo,
    SendPacket,
    RecvPacket,
    KeepAlive,
    NotePreferred,
    PeerGone,
    PeerPresent,
    Ping,
    Pong,
    Health,
    Restarting,
    /// Any frame type we don't model; carried through so the reader can skip it.
    Other(u8),
}

impl FrameType {
    pub fn to_byte(self) -> u8 {
        match self {
            FrameType::ServerKey => 0x01,
            FrameType::ClientInfo => 0x02,
            FrameType::ServerInfo => 0x03,
            FrameType::SendPacket => 0x04,
            FrameType::RecvPacket => 0x05,
            FrameType::KeepAlive => 0x06,
            FrameType::NotePreferred => 0x07,
            FrameType::PeerGone => 0x08,
            FrameType::PeerPresent => 0x09,
            FrameType::Ping => 0x12,
            FrameType::Pong => 0x13,
            FrameType::Health => 0x14,
            FrameType::Restarting => 0x15,
            FrameType::Other(b) => b,
        }
    }

    pub fn from_byte(b: u8) -> Self {
        match b {
            0x01 => FrameType::ServerKey,
            0x02 => FrameType::ClientInfo,
            0x03 => FrameType::ServerInfo,
            0x04 => FrameType::SendPacket,
            0x05 => FrameType::RecvPacket,
            0x06 => FrameType::KeepAlive,
            0x07 => FrameType::NotePreferred,
            0x08 => FrameType::PeerGone,
            0x09 => FrameType::PeerPresent,
            0x12 => FrameType::Ping,
            0x13 => FrameType::Pong,
            0x14 => FrameType::Health,
            0x15 => FrameType::Restarting,
            other => FrameType::Other(other),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame length {len} exceeds maximum {max}")]
    TooLong { len: u32, max: u32 },
}

/// Reads one frame header + payload, rejecting any frame longer than `max`.
/// The returned payload is exactly `length` bytes.
pub async fn read_frame<R: AsyncRead + Unpin>(
    r: &mut R,
    max: u32,
) -> Result<(FrameType, Vec<u8>), FrameError> {
    let type_byte = r.read_u8().await?;
    let len = r.read_u32().await?; // big-endian
    if len > max {
        return Err(FrameError::TooLong { len, max });
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload).await?;
    Ok((FrameType::from_byte(type_byte), payload))
}

/// Writes one frame (header + payload) and flushes.
pub async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    frame_type: FrameType,
    payload: &[u8],
) -> Result<(), FrameError> {
    let mut header = [0u8; 5];
    header[0] = frame_type.to_byte();
    header[1..5].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    w.write_all(&header).await?;
    w.write_all(payload).await?;
    w.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_bytes() {
        assert_eq!(MAGIC, &[0x44, 0x45, 0x52, 0x50, 0xf0, 0x9f, 0x94, 0x91]);
    }

    #[test]
    fn frame_type_round_trip() {
        for b in 0u8..=0x20 {
            assert_eq!(FrameType::from_byte(b).to_byte(), b);
        }
    }

    #[tokio::test]
    async fn write_then_read() {
        let mut buf = Vec::new();
        write_frame(&mut buf, FrameType::SendPacket, b"hello derp")
            .await
            .unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let (t, payload) = read_frame(&mut cursor, MAX_FRAME_SIZE).await.unwrap();
        assert_eq!(t, FrameType::SendPacket);
        assert_eq!(payload, b"hello derp");
    }

    #[tokio::test]
    async fn oversize_frame_rejected_without_allocating() {
        // Header claims 4 GiB; must error on the length check, not OOM.
        let mut bytes = vec![FrameType::RecvPacket.to_byte()];
        bytes.extend_from_slice(&u32::MAX.to_be_bytes());
        let mut cursor = std::io::Cursor::new(bytes);
        let err = read_frame(&mut cursor, MAX_FRAME_SIZE).await.unwrap_err();
        assert!(matches!(err, FrameError::TooLong { .. }));
    }

    #[tokio::test]
    async fn truncated_frame_is_error_not_panic() {
        // Length says 10 but only 3 bytes follow.
        let mut bytes = vec![FrameType::RecvPacket.to_byte()];
        bytes.extend_from_slice(&10u32.to_be_bytes());
        bytes.extend_from_slice(b"abc");
        let mut cursor = std::io::Cursor::new(bytes);
        assert!(read_frame(&mut cursor, MAX_FRAME_SIZE).await.is_err());
    }

    #[tokio::test]
    async fn empty_stream_is_eof_error() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        assert!(read_frame(&mut cursor, MAX_FRAME_SIZE).await.is_err());
    }
}
