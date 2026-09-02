//! `ListOffsets` (API key 2) v1: asks the broker for the offset (and,
//! at v1+, the timestamp of that offset) closest to a given timestamp
//! per partition -- most commonly `-1` ("latest", the high-watermark)
//! or `-2` ("earliest").
//!
//! Implemented at **v1**, not v0 like every other API in this crate so
//! far -- a deliberate exception, not an inconsistency. v0's response
//! carries no timestamp at all (just an array of offsets per
//! partition); v1's response is `{partition, error_code, timestamp,
//! offset}`, a single pair per partition. `SLOMonitor`'s
//! `_get_latest_timestamp_seconds_ago()` (GOV-043) needs that
//! `timestamp`, so v1 is the only version that can serve both it and
//! `MetricsCollector`'s watermark lookups (GOV-034..036) from one
//! implementation. Still classic/non-flexible encoding, same as every
//! v0 message here -- just one version newer.

use crate::error::CodecError;
use crate::wire::{
    read_array_len, read_i16, read_i32, read_i64, read_string, write_i16, write_i32, write_i64,
    write_string,
};
use rusty_wire::{Reader, Writer};

/// Requests the offset at the latest available position (the
/// high-watermark) -- Kafka's well-known `-1` timestamp sentinel,
/// matching `confluent_kafka.admin.OffsetSpec.latest()`.
pub const LATEST_TIMESTAMP: i64 = -1;
/// Requests the offset at the earliest available (retained) position
/// -- Kafka's well-known `-2` timestamp sentinel, matching
/// `confluent_kafka.admin.OffsetSpec.earliest()`.
pub const EARLIEST_TIMESTAMP: i64 = -2;

/// One partition to query within a [`ListOffsetsTopicRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListOffsetsPartitionRequest {
    /// Partition index (0-based).
    pub partition_index: i32,
    /// The timestamp to query -- [`LATEST_TIMESTAMP`]/[`EARLIEST_TIMESTAMP`]
    /// or a literal milliseconds-since-epoch value.
    pub timestamp: i64,
}

/// One topic's partitions to query within a [`ListOffsetsRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListOffsetsTopicRequest {
    /// Topic name.
    pub name: String,
    /// Partitions to query within this topic.
    pub partitions: Vec<ListOffsetsPartitionRequest>,
}

/// `ListOffsetsRequest` v1.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListOffsetsRequest {
    /// Always `-1` for a normal (non-replica-broker) client -- this
    /// crate never sends anything else, but the field exists on the
    /// wire, so it's modeled rather than hardcoded silently.
    pub replica_id: i32,
    /// Topics/partitions to query.
    pub topics: Vec<ListOffsetsTopicRequest>,
}

impl ListOffsetsRequest {
    /// Encodes the v1 body.
    pub fn encode(&self, writer: &mut Writer) {
        write_i32(writer, self.replica_id);
        write_i32(writer, self.topics.len() as i32);
        for topic in &self.topics {
            write_string(writer, &topic.name);
            write_i32(writer, topic.partitions.len() as i32);
            for partition in &topic.partitions {
                write_i32(writer, partition.partition_index);
                write_i64(writer, partition.timestamp);
            }
        }
    }

    /// Decodes a v1 body -- symmetric with [`encode`](Self::encode),
    /// for a fake broker standing in for tests (see
    /// [`crate::testing`]; this crate is client-only).
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let replica_id = read_i32(reader)?;
        let topic_count = read_array_len(reader)?.max(0);
        let mut topics = Vec::with_capacity(topic_count as usize);
        for _ in 0..topic_count {
            let name = read_string(reader)?;
            let partition_count = read_array_len(reader)?.max(0);
            let mut partitions = Vec::with_capacity(partition_count as usize);
            for _ in 0..partition_count {
                partitions.push(ListOffsetsPartitionRequest {
                    partition_index: read_i32(reader)?,
                    timestamp: read_i64(reader)?,
                });
            }
            topics.push(ListOffsetsTopicRequest { name, partitions });
        }
        Ok(ListOffsetsRequest { replica_id, topics })
    }
}

/// One partition's result within a [`ListOffsetsTopicResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListOffsetsPartitionResponse {
    /// Partition index, echoing the request.
    pub partition_index: i32,
    /// Kafka error code; `0` means success.
    pub error_code: i16,
    /// The timestamp of the returned `offset` -- what
    /// `SLOMonitor._get_latest_timestamp_seconds_ago()` (GOV-043)
    /// needs; `-1` if unknown.
    pub timestamp: i64,
    /// The offset closest to the requested timestamp -- the
    /// high-watermark when the request asked for [`LATEST_TIMESTAMP`].
    pub offset: i64,
}

/// One topic's results within a [`ListOffsetsResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListOffsetsTopicResponse {
    /// Topic name, echoing the request.
    pub name: String,
    /// One result per partition queried for this topic.
    pub partitions: Vec<ListOffsetsPartitionResponse>,
}

/// `ListOffsetsResponse` v1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListOffsetsResponse {
    /// One entry per topic in the request, in the same order.
    pub topics: Vec<ListOffsetsTopicResponse>,
}

impl ListOffsetsResponse {
    /// Decodes the response body.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let topic_count = read_array_len(reader)?.max(0);
        let mut topics = Vec::with_capacity(topic_count as usize);
        for _ in 0..topic_count {
            let name = read_string(reader)?;
            let partition_count = read_array_len(reader)?.max(0);
            let mut partitions = Vec::with_capacity(partition_count as usize);
            for _ in 0..partition_count {
                partitions.push(ListOffsetsPartitionResponse {
                    partition_index: read_i32(reader)?,
                    error_code: read_i16(reader)?,
                    timestamp: read_i64(reader)?,
                    offset: read_i64(reader)?,
                });
            }
            topics.push(ListOffsetsTopicResponse { name, partitions });
        }
        Ok(ListOffsetsResponse { topics })
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
                write_i64(writer, partition.timestamp);
                write_i64(writer, partition.offset);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_encodes_a_latest_timestamp_query() {
        let request = ListOffsetsRequest {
            replica_id: -1,
            topics: vec![ListOffsetsTopicRequest {
                name: "manpower.readiness-reporting.assessments".to_string(),
                partitions: vec![ListOffsetsPartitionRequest {
                    partition_index: 0,
                    timestamp: LATEST_TIMESTAMP,
                }],
            }],
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), -1); // replica_id
        assert_eq!(read_array_len(&mut reader).unwrap(), 1); // topics
        assert_eq!(
            crate::wire::read_string(&mut reader).unwrap(),
            "manpower.readiness-reporting.assessments"
        );
        assert_eq!(read_array_len(&mut reader).unwrap(), 1); // partitions
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), 0); // partition_index
        assert_eq!(read_i64(&mut reader).unwrap(), -1); // timestamp
        assert!(reader.is_empty());
    }

    #[test]
    fn request_encode_then_decode_round_trips() {
        let request = ListOffsetsRequest {
            replica_id: -1,
            topics: vec![ListOffsetsTopicRequest {
                name: "t".to_string(),
                partitions: vec![
                    ListOffsetsPartitionRequest {
                        partition_index: 0,
                        timestamp: LATEST_TIMESTAMP,
                    },
                    ListOffsetsPartitionRequest {
                        partition_index: 1,
                        timestamp: EARLIEST_TIMESTAMP,
                    },
                ],
            }],
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(ListOffsetsRequest::decode(&mut reader).unwrap(), request);
    }

    #[test]
    fn response_decodes_a_high_watermark() {
        let response = ListOffsetsResponse {
            topics: vec![ListOffsetsTopicResponse {
                name: "t".to_string(),
                partitions: vec![ListOffsetsPartitionResponse {
                    partition_index: 0,
                    error_code: 0,
                    timestamp: 1_735_689_600_000,
                    offset: 42,
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = ListOffsetsResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded, response);
        assert_eq!(decoded.topics[0].partitions[0].offset, 42);
    }

    #[test]
    fn response_decodes_an_unknown_topic_error() {
        let response = ListOffsetsResponse {
            topics: vec![ListOffsetsTopicResponse {
                name: "no-such-topic".to_string(),
                partitions: vec![ListOffsetsPartitionResponse {
                    partition_index: 0,
                    error_code: 3, // UNKNOWN_TOPIC_OR_PARTITION
                    timestamp: -1,
                    offset: -1,
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = ListOffsetsResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded.topics[0].partitions[0].error_code, 3);
    }
}
