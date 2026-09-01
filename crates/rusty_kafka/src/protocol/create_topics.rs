//! `CreateTopics` (API key 19) v0: creates one or more topics, each with
//! partition count/replication factor (or an explicit per-partition
//! replica assignment instead) and per-topic config overrides -- what
//! `rusty-meshed-sdk`'s `TopicManager` needs for idempotent topic
//! creation (SDK-070..079 in the capability manifest).

use crate::error::CodecError;
use crate::wire::{
    read_array_len, read_i16, read_string, write_i16, write_i32, write_nullable_string,
    write_string,
};
use rusty_wire::{Reader, Writer};

/// An explicit replica-to-broker assignment for one partition, in place
/// of letting the broker choose (`num_partitions`/`replication_factor`
/// on [`CreatableTopic`] and this are mutually exclusive per the Kafka
/// protocol; this crate doesn't enforce that -- the broker rejects an
/// invalid combination with an error code in the response).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaAssignment {
    /// Which partition this assignment is for.
    pub partition_index: i32,
    /// Broker node IDs to host this partition's replicas, in order
    /// (first is the preferred leader).
    pub broker_ids: Vec<i32>,
}

/// A per-topic config override, e.g. `("cleanup.policy", Some("delete"))`
/// -- the Rust equivalent of `TopicSpec::kafka_config()`'s output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEntry {
    /// Config key, e.g. `"retention.ms"`.
    pub name: String,
    /// Config value; `None` resets the key to its broker default.
    pub value: Option<String>,
}

/// One topic to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatableTopic {
    /// Topic name.
    pub name: String,
    /// Partition count, or `-1` if `assignments` gives an explicit
    /// per-partition placement instead.
    pub num_partitions: i32,
    /// Replication factor, or `-1` if `assignments` gives an explicit
    /// per-partition placement instead.
    pub replication_factor: i16,
    /// Explicit per-partition replica placement; empty to let the
    /// broker choose based on `num_partitions`/`replication_factor`.
    pub assignments: Vec<ReplicaAssignment>,
    /// Per-topic config overrides.
    pub configs: Vec<ConfigEntry>,
}

/// `CreateTopicsRequest` v0.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CreateTopicsRequest {
    /// Topics to create in this one request.
    pub topics: Vec<CreatableTopic>,
    /// How long the broker should wait for the topics to be fully
    /// created before responding, in milliseconds.
    pub timeout_ms: i32,
}

impl CreateTopicsRequest {
    /// Encodes the v0 body.
    pub fn encode(&self, writer: &mut Writer) {
        write_i32(writer, self.topics.len() as i32);
        for topic in &self.topics {
            write_string(writer, &topic.name);
            write_i32(writer, topic.num_partitions);
            write_i16(writer, topic.replication_factor);

            write_i32(writer, topic.assignments.len() as i32);
            for assignment in &topic.assignments {
                write_i32(writer, assignment.partition_index);
                write_i32(writer, assignment.broker_ids.len() as i32);
                for broker_id in &assignment.broker_ids {
                    write_i32(writer, *broker_id);
                }
            }

            write_i32(writer, topic.configs.len() as i32);
            for config in &topic.configs {
                write_string(writer, &config.name);
                write_nullable_string(writer, config.value.as_deref());
            }
        }
        write_i32(writer, self.timeout_ms);
    }
}

/// One topic's creation result within a [`CreateTopicsResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatableTopicResult {
    /// Topic name, echoing the request.
    pub name: String,
    /// Kafka error code; `0` means success. `36`
    /// (`TOPIC_ALREADY_EXISTS`) is the idempotency case
    /// `DataProductProducerBase.startup()` swallows (SDK-015).
    pub error_code: i16,
}

/// `CreateTopicsResponse` v0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTopicsResponse {
    /// One result per topic in the request, in the same order.
    pub topics: Vec<CreatableTopicResult>,
}

impl CreateTopicsResponse {
    /// Decodes the response body.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let count = read_array_len(reader)?.max(0);
        let mut topics = Vec::with_capacity(count as usize);
        for _ in 0..count {
            topics.push(CreatableTopicResult {
                name: read_string(reader)?,
                error_code: read_i16(reader)?,
            });
        }
        Ok(CreateTopicsResponse { topics })
    }
}

/// Kafka's `TOPIC_ALREADY_EXISTS` error code -- what a `CreateTopics`
/// response reports for a topic that already exists, the idempotency
/// signal `DataProductProducerBase.startup()` (SDK-015) and
/// `TopicManager` need to treat as success rather than an error.
pub const TOPIC_ALREADY_EXISTS: i16 = 36;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_encodes_a_simple_topic() {
        let request = CreateTopicsRequest {
            topics: vec![CreatableTopic {
                name: "manpower.readiness-reporting.assessments".to_string(),
                num_partitions: 3,
                replication_factor: 1,
                assignments: vec![],
                configs: vec![ConfigEntry {
                    name: "cleanup.policy".to_string(),
                    value: Some("delete".to_string()),
                }],
            }],
            timeout_ms: 5000,
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(read_array_len(&mut reader).unwrap(), 1);
        assert_eq!(
            crate::wire::read_string(&mut reader).unwrap(),
            "manpower.readiness-reporting.assessments"
        );
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), 3);
        assert_eq!(read_i16(&mut reader).unwrap(), 1);
        assert_eq!(read_array_len(&mut reader).unwrap(), 0); // assignments
        assert_eq!(read_array_len(&mut reader).unwrap(), 1); // configs
        assert_eq!(
            crate::wire::read_string(&mut reader).unwrap(),
            "cleanup.policy"
        );
        assert_eq!(
            crate::wire::read_nullable_string(&mut reader).unwrap(),
            Some("delete".to_string())
        );
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), 5000); // timeout_ms
        assert!(reader.is_empty());
    }

    #[test]
    fn request_encodes_explicit_replica_assignments() {
        let request = CreateTopicsRequest {
            topics: vec![CreatableTopic {
                name: "t".to_string(),
                num_partitions: -1,
                replication_factor: -1,
                assignments: vec![ReplicaAssignment {
                    partition_index: 0,
                    broker_ids: vec![1, 2],
                }],
                configs: vec![],
            }],
            timeout_ms: 1000,
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        read_array_len(&mut reader).unwrap(); // topics len
        crate::wire::read_string(&mut reader).unwrap(); // name
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), -1); // num_partitions
        assert_eq!(read_i16(&mut reader).unwrap(), -1); // replication_factor
        assert_eq!(read_array_len(&mut reader).unwrap(), 1); // assignments len
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), 0); // partition_index
        assert_eq!(read_array_len(&mut reader).unwrap(), 2); // broker_ids len
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), 1);
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), 2);
    }

    fn encode_response(results: &[(&str, i16)]) -> Vec<u8> {
        let mut writer = Writer::new();
        write_i32(&mut writer, results.len() as i32);
        for (name, error_code) in results {
            write_string(&mut writer, name);
            write_i16(&mut writer, *error_code);
        }
        writer.into_vec()
    }

    #[test]
    fn decodes_success_response() {
        let bytes = encode_response(&[("my-topic", 0)]);
        let mut reader = Reader::new(&bytes);
        let response = CreateTopicsResponse::decode(&mut reader).unwrap();
        assert_eq!(
            response.topics,
            vec![CreatableTopicResult {
                name: "my-topic".to_string(),
                error_code: 0
            }]
        );
    }

    #[test]
    fn decodes_topic_already_exists_error() {
        let bytes = encode_response(&[("my-topic", TOPIC_ALREADY_EXISTS)]);
        let mut reader = Reader::new(&bytes);
        let response = CreateTopicsResponse::decode(&mut reader).unwrap();
        assert_eq!(response.topics[0].error_code, TOPIC_ALREADY_EXISTS);
    }
}
