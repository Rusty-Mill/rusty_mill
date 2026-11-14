//! Async I/O drivers for the sans-I/O handshake in `handshake.rs`.

use rusty_tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use ts_key::MachinePrivate;

use super::conn::Conn;
use super::handshake::{
    ClientHandshake, HandshakeError, INITIATION_LEN, MSG_TYPE_ERROR, MSG_TYPE_RESPONSE,
    RESPONSE_PAYLOAD_LEN, client_initiation, server_respond,
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
    T: AsyncRead + AsyncWrite + Unpin + Send,
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
    T: AsyncRead + AsyncWrite + Unpin + Send,
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

/// Server side of [`connect`]: reads the client's initiation from `io`,
/// answers it, and returns the secured connection with the client's
/// machine public key.
pub async fn accept<T>(
    mut io: T,
    control_key: &MachinePrivate,
) -> Result<(Conn<T>, [u8; 32]), ConnectError>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    let mut init = [0u8; INITIATION_LEN];
    io.read_exact(&mut init).await?;
    accept_deferred(io, control_key, &init).await
}

/// Server side of [`connect_deferred`]: the initiation already arrived out
/// of band (the controlhttp `X-Tailscale-Handshake` header).
pub async fn accept_deferred<T>(
    mut io: T,
    control_key: &MachinePrivate,
    init: &[u8; INITIATION_LEN],
) -> Result<(Conn<T>, [u8; 32]), ConnectError>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    let accepted = server_respond(control_key, init)?;
    let mut reply = [0u8; 3 + RESPONSE_PAYLOAD_LEN];
    reply[0] = MSG_TYPE_RESPONSE;
    reply[1..3].copy_from_slice(&(RESPONSE_PAYLOAD_LEN as u16).to_be_bytes());
    reply[3..].copy_from_slice(&accepted.response);
    io.write_all(&reply).await?;
    io.flush().await?;
    Ok((Conn::new(io, accepted.keys), accepted.machine_public))
}
