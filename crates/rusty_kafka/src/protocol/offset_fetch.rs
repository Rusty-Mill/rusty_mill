//! `OffsetFetch` (API key 9) v0: asks the broker for a consumer
//! group's last-committed offset per partition -- what
//! `MetricsCollector.compute_lag` needs (GOV-034/035) alongside
//! [`crate::protocol::list_offsets`]'s high-watermark lookup; the two
//! together are `max(0, high_watermark - committed_offset)`.
//!
//! **Scope caveat, not a wire-format concern:** on a real multi-broker
//! cluster this request must go to the consumer group's *coordinator*
//! broker, found via `FindCoordinator` (API key 10, not implemented by
//! this crate). `KafkaClient` has no controller/coordinator discovery
//! at all yet (see the crate's module doc) -- sending `OffsetFetch` to
//! whichever broker it's connected to is only correct when that broker
//! also happens to be the coordinator, true for meshed's single
//! all-in-one dev broker, not guaranteed in general. Fine for
//! `MetricsCollector`'s scratch consumer today; revisit once real
//! consumer-group coordination lands.

use crate::error::CodecError;
use crate::wire::{
    read_array_len, read_i16, read_i32, read_i64, read_nullable_string, read_string, write_i16,
    write_i32, write_i64, write_nullable_string, write_string,
};
use rusty_wire::{Reader, Writer};

/// The wire value for "this partition has no committed offset" --
/// distinct from `confluent_kafka`'s own `OFFSET_INVALID` Python
/// constant (`-1001`), which `MetricsCollector.compute_lag` (GOV-035)
/// treats identically to this one: any committed offset `< 0` counts
/// as `0`.
pub const NO_COMMITTED_OFFSET: i64 = -1;

/// One topic's partitions to fetch committed offsets for within an
/// [`OffsetFetchRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetFetchTopicRequest {
    /// Topic name.
    pub name: String,
    /// Partition indexes to fetch within this topic.
    pub partitions: Vec<i32>,
}

/// `OffsetFetchRequest` v0.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OffsetFetchRequest {
    /// Consumer group ID to fetch committed offsets for.
    pub group_id: String,
    /// Topics/partitions to fetch.
    pub topics: Vec<OffsetFetchTopicRequest>,
}

impl OffsetFetchRequest {
    /// Encodes the v0 body.
    pub fn encode(&self, writer: &mut Writer) {
        write_string(writer, &self.group_id);
        write_i32(writer, self.topics.len() as i32);
        for topic in &self.topics {
            write_string(writer, &topic.name);
            write_i32(writer, topic.partitions.len() as i32);
            for partition_index in &topic.partitions {
                write_i32(writer, *partition_index);
            }
        }
    }

    /// Decodes a v0 body -- symmetric with [`encode`](Self::encode),
    /// for a fake broker standing in for tests (see
    /// [`crate::testing`]; this crate is client-only).
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let group_id = read_string(reader)?;
        let topic_count = read_array_len(reader)?.max(0);
        let mut topics = Vec::with_capacity(topic_count as usize);
        for _ in 0..topic_count {
            let name = read_string(reader)?;
            let partition_count = read_array_len(reader)?.max(0);
            let mut partitions = Vec::with_capacity(partition_count as usize);
            for _ in 0..partition_count {
                partitions.push(read_i32(reader)?);
            }
            topics.push(OffsetFetchTopicRequest { name, partitions });
        }
        Ok(OffsetFetchRequest { group_id, topics })
    }
}

/// One partition's committed-offset result within an
/// [`OffsetFetchTopicResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetFetchPartitionResponse {
    /// Partition index, echoing the request.
    pub partition_index: i32,
    /// The group's committed offset for this partition, or
    /// [`NO_COMMITTED_OFFSET`] if the group has never committed one.
    pub committed_offset: i64,
    /// Consumer-supplied metadata stored alongside the commit, if any
    /// (`meshed` never sets this; always `None` in practice).
    pub metadata: Option<String>,
    /// Kafka error code; `0` means success.
    pub error_code: i16,
}

/// One topic's results within an [`OffsetFetchResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetFetchTopicResponse {
    /// Topic name, echoing the request.
    pub name: String,
    /// One result per partition requested for this topic.
    pub partitions: Vec<OffsetFetchPartitionResponse>,
}

/// `OffsetFetchResponse` v0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetFetchResponse {
    /// One entry per topic in the request, in the same order.
    pub topics: Vec<OffsetFetchTopicResponse>,
}

impl OffsetFetchResponse {
    /// Decodes the response body.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let topic_count = read_array_len(reader)?.max(0);
        let mut topics = Vec::with_capacity(topic_count as usize);
        for _ in 0..topic_count {
            let name = read_string(reader)?;
            let partition_count = read_array_len(reader)?.max(0);
            let mut partitions = Vec::with_capacity(partition_count as usize);
            for _ in 0..partition_count {
                partitions.push(OffsetFetchPartitionResponse {
                    partition_index: read_i32(reader)?,
                    committed_offset: read_i64(reader)?,
                    metadata: read_nullable_string(reader)?,
                    error_code: read_i16(reader)?,
                });
            }
            topics.push(OffsetFetchTopicResponse { name, partitions });
        }
        Ok(OffsetFetchResponse { topics })
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
                write_i64(writer, partition.committed_offset);
                write_nullable_string(writer, partition.metadata.as_deref());
                write_i16(writer, partition.error_code);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_encodes_a_group_and_partitions() {
        let request = OffsetFetchRequest {
            group_id: "_meshed_metrics_readiness-reporting".to_string(),
            topics: vec![OffsetFetchTopicRequest {
                name: "manpower.readiness-reporting.assessments".to_string(),
                partitions: vec![0, 1],
            }],
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(
            crate::wire::read_string(&mut reader).unwrap(),
            "_meshed_metrics_readiness-reporting"
        );
        assert_eq!(read_array_len(&mut reader).unwrap(), 1); // topics
        assert_eq!(
            crate::wire::read_string(&mut reader).unwrap(),
            "manpower.readiness-reporting.assessments"
        );
        assert_eq!(read_array_len(&mut reader).unwrap(), 2); // partitions
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), 0);
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), 1);
        assert!(reader.is_empty());
    }

    #[test]
    fn request_encode_then_decode_round_trips() {
        let request = OffsetFetchRequest {
            group_id: "g".to_string(),
            topics: vec![OffsetFetchTopicRequest {
                name: "t".to_string(),
                partitions: vec![0, 1, 2],
            }],
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(OffsetFetchRequest::decode(&mut reader).unwrap(), request);
    }

    #[test]
    fn response_decodes_a_committed_offset() {
        let response = OffsetFetchResponse {
            topics: vec![OffsetFetchTopicResponse {
                name: "t".to_string(),
                partitions: vec![OffsetFetchPartitionResponse {
                    partition_index: 0,
                    committed_offset: 17,
                    metadata: None,
                    error_code: 0,
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = OffsetFetchResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded, response);
        assert_eq!(decoded.topics[0].partitions[0].committed_offset, 17);
    }

    #[test]
    fn response_decodes_no_committed_offset_sentinel() {
        let response = OffsetFetchResponse {
            topics: vec![OffsetFetchTopicResponse {
                name: "t".to_string(),
                partitions: vec![OffsetFetchPartitionResponse {
                    partition_index: 0,
                    committed_offset: NO_COMMITTED_OFFSET,
                    metadata: None,
                    error_code: 0,
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = OffsetFetchResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded.topics[0].partitions[0].committed_offset, -1);
    }
}
