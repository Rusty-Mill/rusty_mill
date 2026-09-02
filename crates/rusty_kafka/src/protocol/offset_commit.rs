//! `OffsetCommit` (API key 8) v2: commits a consumer group's consumed
//! offsets so a future `OffsetFetch` (or a restarted consumer
//! resuming the group) picks up where this one left off.
//!
//! v2, not v0 -- deliberate. v0 has no `group_generation_id`/
//! `member_id`, so the broker can't tell a commit came from a current
//! group member; since this crate's consumer-group support (via
//! `JoinGroup`/`SyncGroup`) always has a real generation and member ID
//! on hand by the time it commits, sending them is strictly more
//! correct than omitting them, and v1 (which has generation/member ID
//! but also a per-partition `commit_timestamp` deprecated by v2's
//! request-level `retention_time_ms` instead) is a needless
//! intermediate stop. v2's response shape is identical to v0/v1's.

use crate::error::CodecError;
use crate::wire::{
    read_array_len, read_i16, read_i32, read_i64, read_nullable_string, read_string, write_i16,
    write_i32, write_i64, write_nullable_string, write_string,
};
use rusty_wire::{Reader, Writer};

/// One partition's offset to commit within an
/// [`OffsetCommitTopicRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetCommitPartitionRequest {
    pub partition_index: i32,
    /// The offset to commit -- the next offset this consumer will
    /// read, i.e. the last processed offset plus one.
    pub committed_offset: i64,
    /// Consumer-supplied metadata to store alongside the commit
    /// (`meshed` never sets this; always `None` in practice, matching
    /// [`crate::protocol::offset_fetch`]'s own note).
    pub committed_metadata: Option<String>,
}

/// One topic's partitions to commit within an [`OffsetCommitRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetCommitTopicRequest {
    pub name: String,
    pub partitions: Vec<OffsetCommitPartitionRequest>,
}

/// `OffsetCommitRequest` v2.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OffsetCommitRequest {
    pub group_id: String,
    /// The generation this member last synced with (from
    /// `JoinGroupResponse`/`SyncGroupResponse`).
    pub group_generation_id: i32,
    /// This member's ID, assigned by the coordinator in
    /// `JoinGroupResponse`.
    pub member_id: String,
    /// How long the broker should retain this commit after the group
    /// becomes empty, in milliseconds; `-1` uses the broker's own
    /// `offsets.retention.minutes` default.
    pub retention_time_ms: i64,
    pub topics: Vec<OffsetCommitTopicRequest>,
}

impl OffsetCommitRequest {
    /// Encodes the v2 body.
    pub fn encode(&self, writer: &mut Writer) {
        write_string(writer, &self.group_id);
        write_i32(writer, self.group_generation_id);
        write_string(writer, &self.member_id);
        write_i64(writer, self.retention_time_ms);
        write_i32(writer, self.topics.len() as i32);
        for topic in &self.topics {
            write_string(writer, &topic.name);
            write_i32(writer, topic.partitions.len() as i32);
            for partition in &topic.partitions {
                write_i32(writer, partition.partition_index);
                write_i64(writer, partition.committed_offset);
                write_nullable_string(writer, partition.committed_metadata.as_deref());
            }
        }
    }

    /// Decodes a v2 body -- symmetric with [`encode`](Self::encode),
    /// for a fake broker standing in for tests (see
    /// [`crate::testing`]; this crate is client-only).
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let group_id = read_string(reader)?;
        let group_generation_id = read_i32(reader)?;
        let member_id = read_string(reader)?;
        let retention_time_ms = read_i64(reader)?;
        let topic_count = read_array_len(reader)?.max(0);
        let mut topics = Vec::with_capacity(topic_count as usize);
        for _ in 0..topic_count {
            let name = read_string(reader)?;
            let partition_count = read_array_len(reader)?.max(0);
            let mut partitions = Vec::with_capacity(partition_count as usize);
            for _ in 0..partition_count {
                partitions.push(OffsetCommitPartitionRequest {
                    partition_index: read_i32(reader)?,
                    committed_offset: read_i64(reader)?,
                    committed_metadata: read_nullable_string(reader)?,
                });
            }
            topics.push(OffsetCommitTopicRequest { name, partitions });
        }
        Ok(OffsetCommitRequest {
            group_id,
            group_generation_id,
            member_id,
            retention_time_ms,
            topics,
        })
    }
}

/// One partition's commit result within an [`OffsetCommitTopicResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetCommitPartitionResponse {
    pub partition_index: i32,
    /// Kafka error code; `0` means success.
    pub error_code: i16,
}

/// One topic's results within an [`OffsetCommitResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetCommitTopicResponse {
    pub name: String,
    pub partitions: Vec<OffsetCommitPartitionResponse>,
}

/// `OffsetCommitResponse` v2 -- the same shape across v0-v4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetCommitResponse {
    pub topics: Vec<OffsetCommitTopicResponse>,
}

impl OffsetCommitResponse {
    /// Decodes the response body.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let topic_count = read_array_len(reader)?.max(0);
        let mut topics = Vec::with_capacity(topic_count as usize);
        for _ in 0..topic_count {
            let name = read_string(reader)?;
            let partition_count = read_array_len(reader)?.max(0);
            let mut partitions = Vec::with_capacity(partition_count as usize);
            for _ in 0..partition_count {
                partitions.push(OffsetCommitPartitionResponse {
                    partition_index: read_i32(reader)?,
                    error_code: read_i16(reader)?,
                });
            }
            topics.push(OffsetCommitTopicResponse { name, partitions });
        }
        Ok(OffsetCommitResponse { topics })
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
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> OffsetCommitRequest {
        OffsetCommitRequest {
            group_id: "readiness-reporting-personnel-consumer".to_string(),
            group_generation_id: 3,
            member_id: "consumer-1-abc".to_string(),
            retention_time_ms: -1,
            topics: vec![OffsetCommitTopicRequest {
                name: "manpower.personnel-lifecycle.assignments".to_string(),
                partitions: vec![OffsetCommitPartitionRequest {
                    partition_index: 0,
                    committed_offset: 42,
                    committed_metadata: None,
                }],
            }],
        }
    }

    #[test]
    fn request_encode_then_decode_round_trips() {
        let request = sample_request();
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(OffsetCommitRequest::decode(&mut reader).unwrap(), request);
    }

    #[test]
    fn request_encodes_group_generation_and_member_before_topics() {
        let request = sample_request();
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(
            read_string(&mut reader).unwrap(),
            "readiness-reporting-personnel-consumer"
        );
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), 3); // group_generation_id
        assert_eq!(read_string(&mut reader).unwrap(), "consumer-1-abc");
        assert_eq!(crate::wire::read_i64(&mut reader).unwrap(), -1); // retention_time_ms
    }

    #[test]
    fn response_decodes_a_successful_commit() {
        let response = OffsetCommitResponse {
            topics: vec![OffsetCommitTopicResponse {
                name: "t".to_string(),
                partitions: vec![OffsetCommitPartitionResponse {
                    partition_index: 0,
                    error_code: 0,
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(OffsetCommitResponse::decode(&mut reader).unwrap(), response);
    }

    #[test]
    fn response_decodes_an_illegal_generation_error() {
        let response = OffsetCommitResponse {
            topics: vec![OffsetCommitTopicResponse {
                name: "t".to_string(),
                partitions: vec![OffsetCommitPartitionResponse {
                    partition_index: 0,
                    error_code: 22, // ILLEGAL_GENERATION
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = OffsetCommitResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded.topics[0].partitions[0].error_code, 22);
    }
}
