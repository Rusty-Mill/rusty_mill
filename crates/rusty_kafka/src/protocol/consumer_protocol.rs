//! The embedded "consumer" protocol payload formats carried inside
//! `JoinGroup`'s per-protocol `metadata` BYTES field and `SyncGroup`'s
//! `assignment` BYTES field when `protocol_type = "consumer"` --
//! KIP-35's `ConsumerProtocolSubscription`/`ConsumerProtocolAssignment`.
//! Opaque bytes as far as `JoinGroup`/`SyncGroup`'s own wire format is
//! concerned, but a real broker's other consumer-group members (and
//! any tooling inspecting the group) expect exactly this encoding
//! under the well-known `protocol_name` values (`"range"`,
//! `"roundrobin"`, ...).
//!
//! v0 only: no `owned_partitions` (v1, cooperative-sticky
//! rebalancing) or `generation_id` (v2). This crate always joins with
//! an empty `member_id` and never claims previously-owned partitions,
//! so neither field has anything to carry -- see
//! [`crate::protocol::join_group`]'s own module doc for the eager
//! (not cooperative-incremental) rebalancing this implies.

use crate::error::CodecError;
use crate::wire::{
    read_array_len, read_i16, read_i32, read_nullable_bytes, read_string, write_i16, write_i32,
    write_nullable_bytes, write_string,
};
use rusty_wire::{Reader, Writer};

/// The version this crate always writes and expects.
const VERSION: i16 = 0;

/// Encodes a `ConsumerProtocolSubscription`'s `topics` list -- what a
/// `JoinGroupRequest`'s `metadata` field carries per declared
/// protocol.
pub fn encode_subscription(topics: &[String]) -> Vec<u8> {
    let mut writer = Writer::new();
    write_i16(&mut writer, VERSION);
    write_i32(&mut writer, topics.len() as i32);
    for topic in topics {
        write_string(&mut writer, topic);
    }
    write_nullable_bytes(&mut writer, None); // user_data
    writer.into_vec()
}

/// Decodes a `ConsumerProtocolSubscription`, returning its `topics`
/// list (the only field this crate's own callers need; `user_data` is
/// read past but discarded).
pub fn decode_subscription(bytes: &[u8]) -> Result<Vec<String>, CodecError> {
    let mut reader = Reader::new(bytes);
    let _version = read_i16(&mut reader)?;
    let topic_count = read_array_len(&mut reader)?.max(0);
    let mut topics = Vec::with_capacity(topic_count as usize);
    for _ in 0..topic_count {
        topics.push(read_string(&mut reader)?);
    }
    let _user_data = read_nullable_bytes(&mut reader)?;
    Ok(topics)
}

/// Encodes a `ConsumerProtocolAssignment` -- `SyncGroupRequest`'s
/// per-member `assignment` payload, as `(topic, partitions)` pairs.
pub fn encode_assignment(partitions: &[(String, Vec<i32>)]) -> Vec<u8> {
    let mut writer = Writer::new();
    write_i16(&mut writer, VERSION);
    write_i32(&mut writer, partitions.len() as i32);
    for (topic, partition_indexes) in partitions {
        write_string(&mut writer, topic);
        write_i32(&mut writer, partition_indexes.len() as i32);
        for partition_index in partition_indexes {
            write_i32(&mut writer, *partition_index);
        }
    }
    write_nullable_bytes(&mut writer, None); // user_data
    writer.into_vec()
}

/// Decodes a `ConsumerProtocolAssignment`, returning its
/// `(topic, partitions)` pairs -- what `SyncGroupResponse`'s
/// `assignment` field carries back to this member.
pub fn decode_assignment(bytes: &[u8]) -> Result<Vec<(String, Vec<i32>)>, CodecError> {
    let mut reader = Reader::new(bytes);
    let _version = read_i16(&mut reader)?;
    let topic_count = read_array_len(&mut reader)?.max(0);
    let mut assignments = Vec::with_capacity(topic_count as usize);
    for _ in 0..topic_count {
        let topic = read_string(&mut reader)?;
        let partition_count = read_array_len(&mut reader)?.max(0);
        let mut partition_indexes = Vec::with_capacity(partition_count as usize);
        for _ in 0..partition_count {
            partition_indexes.push(read_i32(&mut reader)?);
        }
        assignments.push((topic, partition_indexes));
    }
    let _user_data = read_nullable_bytes(&mut reader)?;
    Ok(assignments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_round_trips_topics() {
        let topics = vec![
            "manpower.personnel-lifecycle.assignments".to_string(),
            "manpower.personnel-lifecycle.promotions".to_string(),
        ];
        let bytes = encode_subscription(&topics);
        assert_eq!(decode_subscription(&bytes).unwrap(), topics);
    }

    #[test]
    fn subscription_round_trips_an_empty_topic_list() {
        let bytes = encode_subscription(&[]);
        assert!(decode_subscription(&bytes).unwrap().is_empty());
    }

    #[test]
    fn assignment_round_trips_partitions_per_topic() {
        let assignments = vec![
            (
                "manpower.personnel-lifecycle.assignments".to_string(),
                vec![0, 1, 2],
            ),
            (
                "manpower.personnel-lifecycle.promotions".to_string(),
                vec![0],
            ),
        ];
        let bytes = encode_assignment(&assignments);
        assert_eq!(decode_assignment(&bytes).unwrap(), assignments);
    }

    #[test]
    fn assignment_round_trips_no_topics() {
        let bytes = encode_assignment(&[]);
        assert!(decode_assignment(&bytes).unwrap().is_empty());
    }
}
