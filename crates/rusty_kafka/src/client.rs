//! [`KafkaClient`]: sends one request, awaits its response, over a
//! single connection. See the crate's module doc for what this first
//! pass does and doesn't cover.

use crate::error::ClientError;
use crate::protocol::api_key;
use crate::protocol::api_versions::{ApiVersionsRequest, ApiVersionsResponse};
use crate::protocol::create_topics::{CreateTopicsRequest, CreateTopicsResponse};
use crate::protocol::header::{RequestHeader, ResponseHeader};
use crate::protocol::list_offsets::{ListOffsetsRequest, ListOffsetsResponse};
use crate::protocol::metadata::{MetadataRequest, MetadataResponse};
use crate::protocol::offset_fetch::{OffsetFetchRequest, OffsetFetchResponse};
use rusty_tokio::io::{AsyncRead, AsyncWrite, TcpStream};
use rusty_wire::{Reader, Writer};
use std::sync::atomic::{AtomicI32, Ordering};

/// Default cap on a single response frame's declared size (16 MiB) --
/// see [`crate::frame::read_frame`].
pub const DEFAULT_MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// A minimal Kafka client: one request in flight at a time over one
/// connection. See the crate's module doc for scope.
pub struct KafkaClient<S> {
    io: S,
    client_id: Option<String>,
    next_correlation_id: AtomicI32,
    max_frame_len: usize,
}

impl KafkaClient<TcpStream> {
    /// Connects to a single broker at `addr` (e.g. `"localhost:9092"`,
    /// matching `PlatformConfig::kafka_bootstrap_servers`). Talks to
    /// exactly this broker for every request -- no controller/leader
    /// discovery yet, see the crate's module doc.
    pub async fn connect(addr: &str, client_id: Option<String>) -> Result<Self, ClientError> {
        let io = TcpStream::connect(addr).await?;
        Ok(KafkaClient::new(io, client_id))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> KafkaClient<S> {
    /// Wraps an already-connected transport -- the seam this crate's own
    /// tests use (an in-memory [`rusty_tokio::io::duplex`] pair) instead
    /// of a real TCP connection.
    pub fn new(io: S, client_id: Option<String>) -> Self {
        KafkaClient {
            io,
            client_id,
            next_correlation_id: AtomicI32::new(0),
            max_frame_len: DEFAULT_MAX_FRAME_LEN,
        }
    }

    /// Overrides the default response-frame size cap (mostly useful for
    /// tests exercising [`ClientError::FrameTooLarge`]).
    pub fn with_max_frame_len(mut self, max_frame_len: usize) -> Self {
        self.max_frame_len = max_frame_len;
        self
    }

    fn next_correlation_id(&self) -> i32 {
        self.next_correlation_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn call(
        &mut self,
        api_key: i16,
        api_version: i16,
        encode_body: impl FnOnce(&mut Writer),
    ) -> Result<Vec<u8>, ClientError> {
        let correlation_id = self.next_correlation_id();
        let header = RequestHeader {
            api_key,
            api_version,
            correlation_id,
            client_id: self.client_id.clone(),
        };

        let mut writer = Writer::new();
        header.encode(&mut writer);
        encode_body(&mut writer);
        crate::frame::write_frame(&mut self.io, writer.as_slice()).await?;

        let response_bytes = crate::frame::read_frame(&mut self.io, self.max_frame_len).await?;
        let mut reader = Reader::new(&response_bytes);
        let response_header = ResponseHeader::decode(&mut reader)?;
        if response_header.correlation_id != correlation_id {
            return Err(ClientError::CorrelationMismatch(
                response_header.correlation_id,
                correlation_id,
            ));
        }
        Ok(reader.peek_remaining().to_vec())
    }

    /// Sends `ApiVersionsRequest` v0 and returns the broker's supported
    /// API version ranges.
    pub async fn api_versions(&mut self) -> Result<ApiVersionsResponse, ClientError> {
        let request = ApiVersionsRequest;
        let body = self
            .call(api_key::API_VERSIONS, 0, |writer| request.encode(writer))
            .await?;
        let mut reader = Reader::new(&body);
        Ok(ApiVersionsResponse::decode(&mut reader)?)
    }

    /// Sends `MetadataRequest` v0.
    pub async fn metadata(
        &mut self,
        request: &MetadataRequest,
    ) -> Result<MetadataResponse, ClientError> {
        let body = self
            .call(api_key::METADATA, 0, |writer| request.encode(writer))
            .await?;
        let mut reader = Reader::new(&body);
        Ok(MetadataResponse::decode(&mut reader)?)
    }

    /// Sends `CreateTopicsRequest` v0.
    pub async fn create_topics(
        &mut self,
        request: &CreateTopicsRequest,
    ) -> Result<CreateTopicsResponse, ClientError> {
        let body = self
            .call(api_key::CREATE_TOPICS, 0, |writer| request.encode(writer))
            .await?;
        let mut reader = Reader::new(&body);
        Ok(CreateTopicsResponse::decode(&mut reader)?)
    }

    /// Sends `ListOffsetsRequest` v1 -- see
    /// [`crate::protocol::list_offsets`]'s module doc for why v1, not
    /// v0 like everything else here.
    pub async fn list_offsets(
        &mut self,
        request: &ListOffsetsRequest,
    ) -> Result<ListOffsetsResponse, ClientError> {
        let body = self
            .call(api_key::LIST_OFFSETS, 1, |writer| request.encode(writer))
            .await?;
        let mut reader = Reader::new(&body);
        Ok(ListOffsetsResponse::decode(&mut reader)?)
    }

    /// Sends `OffsetFetchRequest` v0. See
    /// [`crate::protocol::offset_fetch`]'s module doc for the
    /// coordinator-routing caveat this call doesn't handle.
    pub async fn offset_fetch(
        &mut self,
        request: &OffsetFetchRequest,
    ) -> Result<OffsetFetchResponse, ClientError> {
        let body = self
            .call(api_key::OFFSET_FETCH, 0, |writer| request.encode(writer))
            .await?;
        let mut reader = Reader::new(&body);
        Ok(OffsetFetchResponse::decode(&mut reader)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::create_topics::{CreatableTopic, CreatableTopicResult};
    use crate::wire::{write_i16, write_i32, write_string};
    use rusty_tokio::io::duplex;

    /// Reads one framed request off `peer` and returns its decoded
    /// header plus the raw (still-encoded) body bytes that followed it
    /// -- a stand-in for "the broker received this request", so tests
    /// can assert what the client actually sent.
    async fn recv_request<S: AsyncRead + Unpin + Send>(peer: &mut S) -> (RequestHeader, Vec<u8>) {
        let frame = crate::frame::read_frame(peer, 1024).await.unwrap();
        let mut reader = Reader::new(&frame);
        let api_key = crate::wire::read_i16(&mut reader).unwrap();
        let api_version = crate::wire::read_i16(&mut reader).unwrap();
        let correlation_id = crate::wire::read_i32(&mut reader).unwrap();
        let client_id = crate::wire::read_nullable_string(&mut reader).unwrap();
        (
            RequestHeader {
                api_key,
                api_version,
                correlation_id,
                client_id,
            },
            reader.peek_remaining().to_vec(),
        )
    }

    async fn send_response<S: AsyncWrite + Unpin + Send>(
        peer: &mut S,
        correlation_id: i32,
        body: &[u8],
    ) {
        let mut writer = Writer::new();
        write_i32(&mut writer, correlation_id);
        writer.write_bytes(body);
        crate::frame::write_frame(peer, writer.as_slice())
            .await
            .unwrap();
    }

    #[rusty_tokio::test]
    async fn api_versions_sends_correct_header_and_decodes_response() {
        let (client_io, mut peer) = duplex(1024);
        let mut client = KafkaClient::new(client_io, Some("rusty_meshed".to_string()));

        let server = rusty_tokio::spawn(async move {
            let (header, body) = recv_request(&mut peer).await;
            assert_eq!(header.api_key, api_key::API_VERSIONS);
            assert_eq!(header.api_version, 0);
            assert_eq!(header.client_id, Some("rusty_meshed".to_string()));
            assert!(body.is_empty());

            let mut response_body = Writer::new();
            write_i16(&mut response_body, 0); // error_code
            write_i32(&mut response_body, 1); // one entry
            write_i16(&mut response_body, api_key::CREATE_TOPICS);
            write_i16(&mut response_body, 0);
            write_i16(&mut response_body, 7);
            send_response(&mut peer, header.correlation_id, response_body.as_slice()).await;
        });

        let response = client.api_versions().await.unwrap();
        server.await.unwrap();

        assert_eq!(response.error_code, 0);
        assert_eq!(response.range_for(api_key::CREATE_TOPICS), Some((0, 7)));
    }

    #[rusty_tokio::test]
    async fn create_topics_round_trips_through_a_fake_peer() {
        let (client_io, mut peer) = duplex(1024);
        let mut client = KafkaClient::new(client_io, None);

        let server = rusty_tokio::spawn(async move {
            let (header, _body) = recv_request(&mut peer).await;
            assert_eq!(header.api_key, api_key::CREATE_TOPICS);

            let mut response_body = Writer::new();
            write_i32(&mut response_body, 1);
            write_string(
                &mut response_body,
                "manpower.readiness-reporting.assessments",
            );
            write_i16(&mut response_body, 0);
            send_response(&mut peer, header.correlation_id, response_body.as_slice()).await;
        });

        let request = CreateTopicsRequest {
            topics: vec![CreatableTopic {
                name: "manpower.readiness-reporting.assessments".to_string(),
                num_partitions: 3,
                replication_factor: 1,
                assignments: vec![],
                configs: vec![],
            }],
            timeout_ms: 5000,
        };
        let response = client.create_topics(&request).await.unwrap();
        server.await.unwrap();

        assert_eq!(
            response.topics,
            vec![CreatableTopicResult {
                name: "manpower.readiness-reporting.assessments".to_string(),
                error_code: 0
            }]
        );
    }

    #[rusty_tokio::test]
    async fn list_offsets_round_trips_through_a_fake_peer() {
        use crate::protocol::list_offsets::{
            ListOffsetsPartitionRequest, ListOffsetsTopicRequest, LATEST_TIMESTAMP,
        };
        use crate::wire::write_i64;

        let (client_io, mut peer) = duplex(1024);
        let mut client = KafkaClient::new(client_io, None);

        let server = rusty_tokio::spawn(async move {
            let (header, _body) = recv_request(&mut peer).await;
            assert_eq!(header.api_key, api_key::LIST_OFFSETS);
            assert_eq!(header.api_version, 1);

            let mut response_body = Writer::new();
            write_i32(&mut response_body, 1); // topics
            write_string(
                &mut response_body,
                "manpower.readiness-reporting.assessments",
            );
            write_i32(&mut response_body, 1); // partitions
            write_i32(&mut response_body, 0); // partition_index
            write_i16(&mut response_body, 0); // error_code
            write_i64(&mut response_body, 1_735_689_600_000); // timestamp
            write_i64(&mut response_body, 42); // offset
            send_response(&mut peer, header.correlation_id, response_body.as_slice()).await;
        });

        let request = crate::protocol::list_offsets::ListOffsetsRequest {
            replica_id: -1,
            topics: vec![ListOffsetsTopicRequest {
                name: "manpower.readiness-reporting.assessments".to_string(),
                partitions: vec![ListOffsetsPartitionRequest {
                    partition_index: 0,
                    timestamp: LATEST_TIMESTAMP,
                }],
            }],
        };
        let response = client.list_offsets(&request).await.unwrap();
        server.await.unwrap();

        assert_eq!(response.topics[0].partitions[0].offset, 42);
        assert_eq!(
            response.topics[0].partitions[0].timestamp,
            1_735_689_600_000
        );
    }

    #[rusty_tokio::test]
    async fn offset_fetch_round_trips_through_a_fake_peer() {
        use crate::protocol::offset_fetch::OffsetFetchTopicRequest;
        use crate::wire::write_i64;

        let (client_io, mut peer) = duplex(1024);
        let mut client = KafkaClient::new(client_io, None);

        let server = rusty_tokio::spawn(async move {
            let (header, _body) = recv_request(&mut peer).await;
            assert_eq!(header.api_key, api_key::OFFSET_FETCH);
            assert_eq!(header.api_version, 0);

            let mut response_body = Writer::new();
            write_i32(&mut response_body, 1); // topics
            write_string(
                &mut response_body,
                "manpower.readiness-reporting.assessments",
            );
            write_i32(&mut response_body, 1); // partitions
            write_i32(&mut response_body, 0); // partition_index
            write_i64(&mut response_body, 17); // committed_offset
            crate::wire::write_nullable_string(&mut response_body, None); // metadata
            write_i16(&mut response_body, 0); // error_code
            send_response(&mut peer, header.correlation_id, response_body.as_slice()).await;
        });

        let request = crate::protocol::offset_fetch::OffsetFetchRequest {
            group_id: "_meshed_metrics_readiness-reporting".to_string(),
            topics: vec![OffsetFetchTopicRequest {
                name: "manpower.readiness-reporting.assessments".to_string(),
                partitions: vec![0],
            }],
        };
        let response = client.offset_fetch(&request).await.unwrap();
        server.await.unwrap();

        assert_eq!(response.topics[0].partitions[0].committed_offset, 17);
    }

    #[rusty_tokio::test]
    async fn mismatched_correlation_id_is_rejected() {
        let (client_io, mut peer) = duplex(1024);
        let mut client = KafkaClient::new(client_io, None);

        let server = rusty_tokio::spawn(async move {
            let (header, _body) = recv_request(&mut peer).await;
            // Respond with the wrong correlation_id on purpose.
            send_response(
                &mut peer,
                header.correlation_id + 1,
                &[0x00, 0x00, 0x00, 0x00],
            )
            .await;
        });

        let err = client.api_versions().await.unwrap_err();
        server.await.unwrap();
        assert!(matches!(err, ClientError::CorrelationMismatch(_, _)));
    }
}
