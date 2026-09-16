//! Topic configuration types -- the Rust port of
//! `meshed.infrastructure.topic_config` (`TopicType`, `TopicSpec`). No
//! code outside this module should build a raw Kafka config list; all
//! cleanup-policy/retention logic lives in [`TopicSpec::kafka_config`]
//! (SDK-069).

use rusty_kafka::protocol::create_topics::ConfigEntry;

/// Controls cleanup policy and retention semantics for a Kafka topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TopicType {
    /// Entity-state topics: log compaction, latest record per key
    /// retained forever (personnel records, position state, ...).
    State,
    /// Immutable event log topics: time-based delete retention (audit
    /// trails, domain events).
    Events,
    /// Command/request topics: time-based delete retention, typically a
    /// shorter TTL than `Events`.
    Commands,
    /// Dead-letter queue topics: delete retention, long TTL for
    /// post-mortem debugging.
    Dlq,
}

/// Typed specification for a Kafka topic, passed to
/// [`crate::TopicManager::create_topic`] for every topic creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicSpec {
    /// Full topic name following the `{domain}.{product}.{stream-type}`
    /// convention. Validated by `TopicManager` before creation.
    pub name: String,
    /// Determines cleanup policy and default retention.
    pub topic_type: TopicType,
    /// Number of partitions. Defaults to `3`.
    pub num_partitions: u32,
    /// Replication factor. Must not exceed the broker count.
    pub replication_factor: u16,
    /// Time-based retention in milliseconds for `Events`/`Commands`/`Dlq`
    /// topics; ignored for `State` topics (compacted). Default is 30
    /// days (`2_592_000_000` ms).
    pub retention_ms: u64,
}

impl TopicSpec {
    /// Builds a spec with the same defaults as the Python source's
    /// dataclass field defaults (`num_partitions=3`,
    /// `replication_factor=1`, `retention_ms=2_592_000_000`).
    pub fn new(name: impl Into<String>, topic_type: TopicType) -> Self {
        TopicSpec {
            name: name.into(),
            topic_type,
            num_partitions: 3,
            replication_factor: 1,
            retention_ms: 2_592_000_000,
        }
    }

    /// Produces the Kafka config entries for this topic type: `State`
    /// gets `cleanup.policy=compact` (plus `min.cleanable.dirty.ratio`
    /// and a 24-hour segment rotation); every other type gets
    /// `cleanup.policy=delete` with an explicit `retention.ms`.
    pub fn kafka_config(&self) -> Vec<ConfigEntry> {
        match self.topic_type {
            TopicType::State => vec![
                ConfigEntry {
                    name: "cleanup.policy".to_string(),
                    value: Some("compact".to_string()),
                },
                ConfigEntry {
                    name: "min.cleanable.dirty.ratio".to_string(),
                    value: Some("0.1".to_string()),
                },
                ConfigEntry {
                    name: "segment.ms".to_string(),
                    value: Some((24 * 60 * 60 * 1000).to_string()),
                },
            ],
            TopicType::Events | TopicType::Commands | TopicType::Dlq => vec![
                ConfigEntry {
                    name: "cleanup.policy".to_string(),
                    value: Some("delete".to_string()),
                },
                ConfigEntry {
                    name: "retention.ms".to_string(),
                    value: Some(self.retention_ms.to_string()),
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_applies_the_same_defaults_as_the_python_dataclass() {
        let spec = TopicSpec::new("manpower.personnel.events", TopicType::Events);
        assert_eq!(spec.num_partitions, 3);
        assert_eq!(spec.replication_factor, 1);
        assert_eq!(spec.retention_ms, 2_592_000_000);
    }

    #[test]
    fn state_topic_gets_compact_policy() {
        let spec = TopicSpec::new("manpower.personnel.state", TopicType::State);
        let config = spec.kafka_config();
        assert_eq!(
            config[0],
            ConfigEntry {
                name: "cleanup.policy".to_string(),
                value: Some("compact".to_string())
            }
        );
        assert_eq!(
            config[1],
            ConfigEntry {
                name: "min.cleanable.dirty.ratio".to_string(),
                value: Some("0.1".to_string())
            }
        );
        assert_eq!(config[2].name, "segment.ms");
    }

    #[test]
    fn events_topic_gets_delete_policy_with_retention() {
        let mut spec = TopicSpec::new("manpower.personnel.events", TopicType::Events);
        spec.retention_ms = 2_592_000_000;
        let config = spec.kafka_config();
        assert_eq!(
            config[0],
            ConfigEntry {
                name: "cleanup.policy".to_string(),
                value: Some("delete".to_string())
            }
        );
        assert_eq!(
            config[1],
            ConfigEntry {
                name: "retention.ms".to_string(),
                value: Some("2592000000".to_string())
            }
        );
    }

    #[test]
    fn dlq_topic_gets_delete_policy() {
        let spec = TopicSpec::new("manpower.personnel.dlq", TopicType::Dlq);
        let config = spec.kafka_config();
        assert_eq!(config[0].value, Some("delete".to_string()));
    }

    #[test]
    fn commands_topic_gets_delete_policy() {
        let spec = TopicSpec::new("manpower.personnel.commands", TopicType::Commands);
        let config = spec.kafka_config();
        assert_eq!(config[0].value, Some("delete".to_string()));
    }
}
