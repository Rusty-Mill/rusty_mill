//! Async I/O drivers for the sans-I/O handshake in `handshake.rs`.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use ts_key::MachinePrivate;

use super::conn::Conn;
use super::handshake::{
    ClientHandshake, HandshakeError, MSG_TYPE_ERROR, MSG_TYPE_RESPONSE, RESPONSE_PAYLOAD_LEN,
    client_initiation,
};

/// Cap on server error-frame text we are willing to read.
const MAX_ERROR_LEN: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("handshake I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Handshake(#[from] HandshakeError),
    /// The server refused the handshake. The text is unauthenticated —
    /// treat as a debugging hint only.
    #[error("server refused handshake: {0:?}")]
    ServerRefused(String),
    #[error("unexpected handshake message type {0}")]
    UnexpectedMessageType(u8),
    #[error("wrong handshake response length {0}")]
    WrongResponseLength(usize),
}

/// Performs a full client handshake over `io`: sends the initiation, reads
/// the response, returns the secured connection.
pub async fn connect<T>(
    mut io: T,
    machine_key: &MachinePrivate,
    control_key: &[u8; 32],
    protocol_version: u16,
) -> Result<Conn<T>, ConnectError>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let (init, hs) = client_initiation(machine_key, control_key, protocol_version);
    io.write_all(&init).await?;
    io.flush().await?;
    connect_deferred(io, hs).await
}

/// Finishes a client handshake whose initiation was already delivered out
/// of band (the controlhttp `X-Tailscale-Handshake` header).
pub async fn connect_deferred<T>(mut io: T, hs: ClientHandshake) -> Result<Conn<T>, ConnectError>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let mut header = [0u8; 3];
    io.read_exact(&mut header).await?;
    let msg_type = header[0];
    let len = u16::from_be_bytes([header[1], header[2]]) as usize;

    match msg_type {
        MSG_TYPE_RESPONSE => {
            if len != RESPONSE_PAYLOAD_LEN {
                return Err(ConnectError::WrongResponseLength(len));
            }
            let mut payload = [0u8; RESPONSE_PAYLOAD_LEN];
            io.read_exact(&mut payload).await?;
            let keys = hs.finish(&payload)?;
            Ok(Conn::new(io, keys))
        }
        MSG_TYPE_ERROR => {
            let mut msg = vec![0u8; len.min(MAX_ERROR_LEN)];
            io.read_exact(&mut msg).await?;
            Err(ConnectError::ServerRefused(
                String::from_utf8_lossy(&msg).into_owned(),
            ))
        }
        other => Err(ConnectError::UnexpectedMessageType(other)),
    }
}
