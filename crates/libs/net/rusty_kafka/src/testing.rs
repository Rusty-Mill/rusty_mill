//! Test-only helpers for standing in as a fake Kafka broker over an
//! in-memory transport (typically `rusty_tokio::io::duplex`). Used by
//! this crate's own tests and by downstream crates
//! (`rusty-meshed-sdk`/`-observability`/...) that build on
//! [`crate::KafkaClient`] and need to fake a broker response without a
//! live Kafka cluster to test against.

use crate::error::ClientError;
use crate::protocol::header::RequestHeader;
use crate::wire::{read_i16, read_i32, read_nullable_string};
use rusty_tokio::io::{AsyncRead, AsyncWrite};
use rusty_wire::{Reader, Writer};

/// Reads one framed request off `peer` and returns its decoded header
/// plus the raw (still request-body-encoded, header already stripped)
/// bytes that followed it -- decode those with the matching request
/// type's own `decode` (e.g.
/// [`crate::protocol::create_topics::CreateTopicsRequest::decode`]).
pub async fn recv_request<S: AsyncRead + Unpin + Send>(
    peer: &mut S,
) -> Result<(RequestHeader, Vec<u8>), ClientError> {
    let frame = crate::frame::read_frame(peer, crate::client::DEFAULT_MAX_FRAME_LEN).await?;
    let mut reader = Reader::new(&frame);
    let api_key = read_i16(&mut reader)?;
    let api_version = read_i16(&mut reader)?;
    let correlation_id = read_i32(&mut reader)?;
    let client_id = read_nullable_string(&mut reader)?;
    Ok((
        RequestHeader {
            api_key,
            api_version,
            correlation_id,
            client_id,
        },
        reader.peek_remaining().to_vec(),
    ))
}

/// Writes a framed response (response header + `body`) to `peer`,
/// echoing `correlation_id` as every "classic" v0 response header does.
pub async fn send_response<S: AsyncWrite + Unpin + Send>(
    peer: &mut S,
    correlation_id: i32,
    body: &[u8],
) -> Result<(), ClientError> {
    let mut writer = Writer::new();
    crate::wire::write_i32(&mut writer, correlation_id);
    writer.write_bytes(body);
    crate::frame::write_frame(peer, writer.as_slice()).await
}
