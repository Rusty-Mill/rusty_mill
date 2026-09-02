//! `Metadata` (API key 3) v0: asks the broker for the cluster's broker
//! list and, optionally, per-topic partition/leader/replica info.

use crate::error::CodecError;
use crate::wire::{read_array_len, read_i16, read_i32, read_string, write_i32, write_string};
use rusty_wire::{Reader, Writer};

/// `MetadataRequest` v0. `topics: None` requests metadata for every
/// topic (encoded as a null, `-1`-length array); `Some(vec![])` requests
/// the broker list only, no topic metadata; `Some(names)` requests
/// exactly those topics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MetadataRequest {
    /// See the type's own doc for what `None` vs. `Some([])` vs.
    /// `Some(names)` each request.
    pub topics: Option<Vec<String>>,
}

impl MetadataRequest {
    /// Encodes the v0 body.
    pub fn encode(&self, writer: &mut Writer) {
        match &self.topics {
            None => write_i32(writer, -1),
            Some(names) => {
                write_i32(writer, names.len() as i32);
                for name in names {
                    write_string(writer, name);
                }
            }
        }
    }
}

/// One broker in the cluster, as reported by [`MetadataResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broker {
    /// The broker's node ID.
    pub node_id: i32,
    /// Hostname or IP the broker advertises for client connections.
    pub host: String,
    /// Port the broker advertises for client connections.
    pub port: i32,
}

/// One partition's metadata within a [`TopicMetadata`] entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionMetadata {
    /// Kafka error code for this partition; `0` means success.
    pub error_code: i16,
    /// Partition index (0-based).
    pub partition_index: i32,
    /// Node ID of the partition's current leader, or `-1` if none.
    pub leader_id: i32,
    /// Node IDs holding a replica of this partition.
    pub replica_nodes: Vec<i32>,
    /// Node IDs currently in the in-sync replica set.
    pub isr_nodes: Vec<i32>,
}

/// One topic's metadata within a [`MetadataResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicMetadata {
    /// Kafka error code for this topic; `0` means success (e.g.
    /// `UNKNOWN_TOPIC_OR_PARTITION` if the topic doesn't exist).
    pub error_code: i16,
    /// Topic name.
    pub name: String,
    /// This topic's partitions.
    pub partitions: Vec<PartitionMetadata>,
}

/// `MetadataResponse` v0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataResponse {
    /// Every broker in the cluster the responding broker knows about.
    pub brokers: Vec<Broker>,
    /// Metadata for the requested topics (or every topic, if the
    /// request's `topics` was `None`).
    pub topics: Vec<TopicMetadata>,
}

impl MetadataResponse {
    /// Decodes the response body.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let broker_count = read_array_len(reader)?.max(0);
        let mut brokers = Vec::with_capacity(broker_count as usize);
        for _ in 0..broker_count {
            brokers.push(Broker {
                node_id: read_i32(reader)?,
                host: read_string(reader)?,
                port: read_i32(reader)?,
            });
        }

        let topic_count = read_array_len(reader)?.max(0);
        let mut topics = Vec::with_capacity(topic_count as usize);
        for _ in 0..topic_count {
            let error_code = read_i16(reader)?;
            let name = read_string(reader)?;

            let partition_count = read_array_len(reader)?.max(0);
            let mut partitions = Vec::with_capacity(partition_count as usize);
            for _ in 0..partition_count {
                let p_error_code = read_i16(reader)?;
                let partition_index = read_i32(reader)?;
                let leader_id = read_i32(reader)?;

                let replica_count = read_array_len(reader)?.max(0);
                let mut replica_nodes = Vec::with_capacity(replica_count as usize);
                for _ in 0..replica_count {
                    replica_nodes.push(read_i32(reader)?);
                }

                let isr_count = read_array_len(reader)?.max(0);
                let mut isr_nodes = Vec::with_capacity(isr_count as usize);
                for _ in 0..isr_count {
                    isr_nodes.push(read_i32(reader)?);
                }

                partitions.push(PartitionMetadata {
                    error_code: p_error_code,
                    partition_index,
                    leader_id,
                    replica_nodes,
                    isr_nodes,
                });
            }

            topics.push(TopicMetadata {
                error_code,
                name,
                partitions,
            });
        }

        Ok(MetadataResponse { brokers, topics })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::write_i16;

    #[test]
    fn request_encodes_none_as_null_array() {
        let request = MetadataRequest { topics: None };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        assert_eq!(writer.into_vec(), [0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn request_encodes_empty_vec_as_zero_length_array() {
        let request = MetadataRequest {
            topics: Some(vec![]),
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        assert_eq!(writer.into_vec(), [0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn request_encodes_topic_names() {
        let request = MetadataRequest {
            topics: Some(vec!["a".to_string(), "bb".to_string()]),
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();
        assert_eq!(&bytes[0..4], [0x00, 0x00, 0x00, 0x02]);
        assert_eq!(&bytes[4..7], [0x00, 0x01, b'a']);
        assert_eq!(&bytes[7..11], [0x00, 0x02, b'b', b'b']);
    }

    /// `(error_code, partition_index, leader_id, replica_nodes, isr_nodes)`.
    type PartitionFixture = (i16, i32, i32, Vec<i32>, Vec<i32>);

    fn encode_response(
        brokers: &[(i32, &str, i32)],
        topics: &[(i16, &str, Vec<PartitionFixture>)],
    ) -> Vec<u8> {
        let mut writer = Writer::new();
        write_i32(&mut writer, brokers.len() as i32);
        for (node_id, host, port) in brokers {
            write_i32(&mut writer, *node_id);
            write_string(&mut writer, host);
            write_i32(&mut writer, *port);
        }
        write_i32(&mut writer, topics.len() as i32);
        for (error_code, name, partitions) in topics {
            write_i16(&mut writer, *error_code);
            write_string(&mut writer, name);
            write_i32(&mut writer, partitions.len() as i32);
            for (p_error_code, partition_index, leader_id, replicas, isr) in partitions {
                write_i16(&mut writer, *p_error_code);
                write_i32(&mut writer, *partition_index);
                write_i32(&mut writer, *leader_id);
                write_i32(&mut writer, replicas.len() as i32);
                for r in replicas {
                    write_i32(&mut writer, *r);
                }
                write_i32(&mut writer, isr.len() as i32);
                for i in isr {
                    write_i32(&mut writer, *i);
                }
            }
        }
        writer.into_vec()
    }

    #[test]
    fn decodes_brokers_and_topics() {
        let bytes = encode_response(
            &[(1, "kafka", 9092)],
            &[(
                0,
                "mesh.governance.slo-violations",
                vec![(0, 0, 1, vec![1], vec![1])],
            )],
        );
        let mut reader = Reader::new(&bytes);
        let response = MetadataResponse::decode(&mut reader).unwrap();

        assert_eq!(
            response.brokers,
            vec![Broker {
                node_id: 1,
                host: "kafka".to_string(),
                port: 9092
            }]
        );
        assert_eq!(response.topics.len(), 1);
        assert_eq!(response.topics[0].name, "mesh.governance.slo-violations");
        assert_eq!(response.topics[0].partitions.len(), 1);
        assert_eq!(response.topics[0].partitions[0].leader_id, 1);
    }

    #[test]
    fn decodes_empty_response() {
        let bytes = encode_response(&[], &[]);
        let mut reader = Reader::new(&bytes);
        let response = MetadataResponse::decode(&mut reader).unwrap();
        assert!(response.brokers.is_empty());
        assert!(response.topics.is_empty());
    }
}
