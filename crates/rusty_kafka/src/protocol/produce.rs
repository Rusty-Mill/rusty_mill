//! `Produce` (API key 0) v3: publishes one record batch (v2/KIP-98
//! format, see [`crate::record_batch`]) per partition.
//!
//! v3, not v0 like most of this crate's other classic-encoded APIs --
//! deliberate, same reasoning [`crate::protocol::list_offsets`]'s own
//! module doc gives for its own version choice. Message format v2
//! (record batch `magic = 2`) requires `ProduceRequest` v3+; v0-v2 only
//! accept the older v0/v1 message set formats, which this crate
//! doesn't implement. v3 itself is still classic (non-flexible)
//! encoding -- flexible versions don't start until v9 -- so it fits
//! this crate's existing request/response header/array conventions
//! unchanged; only the `records` bytes inside each partition use the
//! newer v2 record batch format.
//!
//! No live broker to validate this against in this environment (see
//! the crate's own module doc) -- every field here is taken directly
//! from the published Kafka protocol spec and hand-verified via this
//! module's own round-trip tests, the same rigor
//! [`crate::record_batch`] gives its CRC-32C implementation.

use crate::error::CodecError;
use crate::record_batch::{self, Record};
use crate::wire::{
    read_array_len, read_i16, read_i32, read_i64, read_nullable_string, read_string, write_i16,
    write_i32, write_i64, write_nullable_string, write_string,
};
use rusty_wire::{Reader, Writer};

/// One partition's records to produce within a [`ProduceTopicRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProducePartitionRequest {
    pub partition_index: i32,
    /// Encoded together as one record batch (KIP-98 v2) -- never split
    /// across multiple batches by this client.
    pub records: Vec<Record>,
}

/// One topic's partitions to produce within a [`ProduceRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProduceTopicRequest {
    pub name: String,
    pub partitions: Vec<ProducePartitionRequest>,
}

/// `ProduceRequest` v3.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProduceRequest {
    /// `-1` = "all in-sync replicas", `1` = "leader only", `0` = "no
    /// acknowledgment" -- standard Kafka acks semantics, passed through
    /// unchanged.
    pub acks: i16,
    /// Broker-side timeout for satisfying `acks`, in milliseconds.
    pub timeout_ms: i32,
    /// Shared by every record in every batch this request encodes --
    /// one produce call means one "now".
    pub base_timestamp_ms: i64,
    pub topics: Vec<ProduceTopicRequest>,
}

impl ProduceRequest {
    /// Encodes the v3 body. `transactional_id` is always sent as null
    /// -- this client has no transaction support.
    pub fn encode(&self, writer: &mut Writer) {
        write_nullable_string(writer, None);
        write_i16(writer, self.acks);
        write_i32(writer, self.timeout_ms);
        write_i32(writer, self.topics.len() as i32);
        for topic in &self.topics {
            write_string(writer, &topic.name);
            write_i32(writer, topic.partitions.len() as i32);
            for partition in &topic.partitions {
                write_i32(writer, partition.partition_index);
                let batch = record_batch::encode_batch(&partition.records, self.base_timestamp_ms);
                write_i32(writer, batch.len() as i32);
                writer.write_bytes(&batch);
            }
        }
    }

    /// Decodes a v3 body -- symmetric with [`encode`](Self::encode),
    /// for a fake broker standing in for tests (see [`crate::testing`];
    /// this crate is client-only). `base_timestamp_ms` isn't recovered
    /// (it's baked into each record batch's bytes, not read back out
    /// since nothing needs it after decoding) -- always `0` on a
    /// decoded value; compare `topics`/`acks`/`timeout_ms` in
    /// round-trip tests instead.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let _transactional_id = read_nullable_string(reader)?;
        let acks = read_i16(reader)?;
        let timeout_ms = read_i32(reader)?;
        let topic_count = read_array_len(reader)?.max(0);
        let mut topics = Vec::with_capacity(topic_count as usize);
        for _ in 0..topic_count {
            let name = read_string(reader)?;
            let partition_count = read_array_len(reader)?.max(0);
            let mut partitions = Vec::with_capacity(partition_count as usize);
            for _ in 0..partition_count {
                let partition_index = read_i32(reader)?;
                let records_len = read_i32(reader)?;
                let records = if records_len < 0 {
                    Vec::new()
                } else {
                    let batch_bytes = reader.read_bytes(records_len as usize)?;
                    record_batch::decode_batch(batch_bytes)?
                };
                partitions.push(ProducePartitionRequest {
                    partition_index,
                    records,
                });
            }
            topics.push(ProduceTopicRequest { name, partitions });
        }
        Ok(ProduceRequest {
            acks,
            timeout_ms,
            base_timestamp_ms: 0,
            topics,
        })
    }
}

/// One partition's result within a [`ProduceTopicResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducePartitionResponse {
    /// Partition index, echoing the request.
    pub partition_index: i32,
    /// Kafka error code; `0` means success.
    pub error_code: i16,
    /// Offset assigned to the first record in the batch.
    pub base_offset: i64,
    /// Broker-assigned append timestamp, or `-1` if the topic uses
    /// `CreateTime` rather than `LogAppendTime`.
    pub log_append_time: i64,
}

/// One topic's results within a [`ProduceResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProduceTopicResponse {
    /// Topic name, echoing the request.
    pub name: String,
    /// One result per partition produced to for this topic.
    pub partitions: Vec<ProducePartitionResponse>,
}

/// `ProduceResponse` v3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProduceResponse {
    /// One entry per topic in the request, in the same order.
    pub topics: Vec<ProduceTopicResponse>,
    pub throttle_time_ms: i32,
}

impl ProduceResponse {
    /// Decodes the response body.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let topic_count = read_array_len(reader)?.max(0);
        let mut topics = Vec::with_capacity(topic_count as usize);
        for _ in 0..topic_count {
            let name = read_string(reader)?;
            let partition_count = read_array_len(reader)?.max(0);
            let mut partitions = Vec::with_capacity(partition_count as usize);
            for _ in 0..partition_count {
                partitions.push(ProducePartitionResponse {
                    partition_index: read_i32(reader)?,
                    error_code: read_i16(reader)?,
                    base_offset: read_i64(reader)?,
                    log_append_time: read_i64(reader)?,
                });
            }
            topics.push(ProduceTopicResponse { name, partitions });
        }
        let throttle_time_ms = read_i32(reader)?;
        Ok(ProduceResponse {
            topics,
            throttle_time_ms,
        })
    }

    /// Encodes the response body -- symmetric with
    /// [`decode`](Self::decode), for a fake broker standing in for
    /// tests.
    pub fn encode(&self, writer: &mut Writer) {
        write_i32(writer, self.topics.len() as i32);
        for topic in &self.topics {
            write_string(writer, &topic.name);
            write_i32(writer, topic.partitions.len() as i32);
            for partition in &topic.partitions {
                write_i32(writer, partition.partition_index);
                write_i16(writer, partition.error_code);
                write_i64(writer, partition.base_offset);
                write_i64(writer, partition.log_append_time);
            }
        }
        write_i32(writer, self.throttle_time_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> ProduceRequest {
        ProduceRequest {
            acks: -1,
            timeout_ms: 5000,
            base_timestamp_ms: 1_735_689_600_000,
            topics: vec![ProduceTopicRequest {
                name: "mesh.governance.slo-violations".to_string(),
                partitions: vec![ProducePartitionRequest {
                    partition_index: 0,
                    records: vec![Record {
                        key: Some(b"orders".to_vec()),
                        value: Some(b"{\"slo_type\":\"freshness\"}".to_vec()),
                        headers: vec![
                            ("event_id".to_string(), Some(b"e-1".to_vec())),
                            ("correlation_id".to_string(), Some(b"c-1".to_vec())),
                        ],
                    }],
                }],
            }],
        }
    }

    #[test]
    fn request_encodes_a_null_transactional_id_and_the_given_acks_timeout() {
        let request = sample_request();
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(read_nullable_string(&mut reader).unwrap(), None); // transactional_id
        assert_eq!(crate::wire::read_i16(&mut reader).unwrap(), -1); // acks
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), 5000); // timeout_ms
        assert_eq!(read_array_len(&mut reader).unwrap(), 1); // topics
        assert_eq!(
            read_string(&mut reader).unwrap(),
            "mesh.governance.slo-violations"
        );
    }

    #[test]
    fn request_encode_then_decode_round_trips_topics_and_records() {
        let request = sample_request();
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = ProduceRequest::decode(&mut reader).unwrap();
        assert_eq!(decoded.acks, request.acks);
        assert_eq!(decoded.timeout_ms, request.timeout_ms);
        assert_eq!(decoded.topics, request.topics);
    }

    #[test]
    fn request_with_no_records_encodes_an_empty_batch_and_round_trips() {
        let request = ProduceRequest {
            acks: 1,
            timeout_ms: 1000,
            base_timestamp_ms: 0,
            topics: vec![ProduceTopicRequest {
                name: "t".to_string(),
                partitions: vec![ProducePartitionRequest {
                    partition_index: 0,
                    records: vec![],
                }],
            }],
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = ProduceRequest::decode(&mut reader).unwrap();
        assert_eq!(decoded.topics, request.topics);
    }

    #[test]
    fn response_decodes_a_successful_produce() {
        let response = ProduceResponse {
            topics: vec![ProduceTopicResponse {
                name: "t".to_string(),
                partitions: vec![ProducePartitionResponse {
                    partition_index: 0,
                    error_code: 0,
                    base_offset: 42,
                    log_append_time: -1,
                }],
            }],
            throttle_time_ms: 0,
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = ProduceResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded, response);
        assert_eq!(decoded.topics[0].partitions[0].base_offset, 42);
    }

    #[test]
    fn response_decodes_a_broker_error() {
        let response = ProduceResponse {
            topics: vec![ProduceTopicResponse {
                name: "no-such-topic".to_string(),
                partitions: vec![ProducePartitionResponse {
                    partition_index: 0,
                    error_code: 3, // UNKNOWN_TOPIC_OR_PARTITION
                    base_offset: -1,
                    log_append_time: -1,
                }],
            }],
            throttle_time_ms: 0,
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = ProduceResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded.topics[0].partitions[0].error_code, 3);
    }
}
