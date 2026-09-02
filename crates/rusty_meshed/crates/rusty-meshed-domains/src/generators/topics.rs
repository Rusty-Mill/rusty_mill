//! Shared Kafka topic bootstrapping for the `run_continuous`/
//! `run_scenario` demo binaries (DOM-037/038, DOM-043/044) -- the Rust
//! port of both scripts' identical `_PHASE4_TOPICS`/`_EVENT_TOPIC_MAP`/
//! `_create_topics()`.
//!
//! # Raw `CreateTopics`, not `TopicManager`
//!
//! The source creates topics via `confluent_kafka.admin.AdminClient`
//! directly, not through either producer's own `startup()` (which
//! would only create the ports *that specific producer* declares).
//! `rusty-meshed-sdk::TopicManager::create_topic` isn't used here for
//! the same reason the source's own comment gives: these topic names
//! follow `domain.product.port-name`, where the last segment is a
//! human-readable port name, not `TopicManager`'s own stream-type
//! naming convention (`TopicSpec`'s `TopicType`) -- these fixed topic
//! names would fail that validator. [`create_phase4_topics`] issues
//! `CreateTopics` directly via `rusty_kafka::KafkaClient`, matching the
//! source's own `AdminClient` bypass exactly.
//!
//! # `Metadata` as the connectivity probe
//!
//! The source's `AdminClient.list_topics(timeout=5)` serves two
//! purposes: it raises if the broker is unreachable (DOM-038's "
//! unreachable broker propagates from connectivity probe"), and its
//! result gives the existing-topics set used to skip already-created
//! ones. A `Metadata` request (`topics: None`, every topic) is this
//! crate's equivalent of both: [`rusty_kafka::KafkaClient::metadata`]
//! failing *is* the connectivity-probe failure, and its response's
//! `topics` list gives the same existing-topics set.

use rusty_kafka::protocol::create_topics::{ConfigEntry, CreatableTopic, CreateTopicsRequest};
use rusty_kafka::protocol::metadata::MetadataRequest;
use rusty_kafka::{ClientError, KafkaClient};
use rusty_tokio::io::{AsyncRead, AsyncWrite};
use std::collections::HashSet;

/// All 9 Phase-4 Kafka topics this platform's manpower domain
/// producers publish to, in the source's own declaration order.
pub const PHASE4_TOPICS: [&str; 9] = [
    "manpower.personnel-lifecycle.assignments",
    "manpower.personnel-lifecycle.promotions",
    "manpower.personnel-lifecycle.separations",
    "manpower.personnel-lifecycle.status-changes",
    "manpower.position-management.authorization-changes",
    "manpower.position-management.fills",
    "manpower.position-management.vacancies",
    "manpower.position-management.modifications",
    "manpower.readiness-reporting.assessments",
];

/// Maps a [`crate::generators::ScenarioEvent::event_name`] to the
/// topic it publishes to. Deliberately covers only the 6 event types
/// [`crate::generators::ScenarioBuilder`] can produce -- the readiness
/// topic is created (it's in [`PHASE4_TOPICS`]) but never published to
/// by either demo generator, matching the source exactly (DOM-042).
pub fn event_topic(event_name: &str) -> Option<&'static str> {
    match event_name {
        "StatusChanged" => Some("manpower.personnel-lifecycle.status-changes"),
        "PersonnelAssigned" => Some("manpower.personnel-lifecycle.assignments"),
        "PersonnelPromoted" => Some("manpower.personnel-lifecycle.promotions"),
        "PersonnelSeparated" => Some("manpower.personnel-lifecycle.separations"),
        "PositionAuthorizationChanged" => {
            Some("manpower.position-management.authorization-changes")
        }
        "PositionFilled" => Some("manpower.position-management.fills"),
        _ => None,
    }
}

/// A `CreateTopics` broker error for one topic, surfaced non-fatally
/// (see [`create_phase4_topics`]'s own doc).
pub struct TopicCreationWarning {
    pub topic: String,
    pub error_code: i16,
}

/// Idempotently creates every [`PHASE4_TOPICS`] topic not already
/// present (3 partitions, replication factor 1, `cleanup.policy =
/// delete`, 30-day retention -- DOM-038/044).
///
/// The `Metadata` call itself failing (broker unreachable) propagates
/// as `Err` -- the connectivity-probe failure the module doc describes.
/// A per-topic `CreateTopics` error (including, though not limited to,
/// the topic already existing) is *not* fatal: it comes back as a
/// [`TopicCreationWarning`] in the returned list rather than aborting
/// the whole call, matching the source's own per-topic
/// `log.warning(...)` inside a swallowing `try`/`except`.
pub async fn create_phase4_topics<S: AsyncRead + AsyncWrite + Unpin + Send>(
    client: &mut KafkaClient<S>,
) -> Result<Vec<TopicCreationWarning>, ClientError> {
    let metadata = client.metadata(&MetadataRequest { topics: None }).await?;
    let existing: HashSet<&str> = metadata
        .topics
        .iter()
        .map(|topic| topic.name.as_str())
        .collect();

    let missing: Vec<&str> = PHASE4_TOPICS
        .iter()
        .copied()
        .filter(|name| !existing.contains(name))
        .collect();
    if missing.is_empty() {
        return Ok(Vec::new());
    }

    let request = CreateTopicsRequest {
        topics: missing
            .iter()
            .map(|&name| CreatableTopic {
                name: name.to_string(),
                num_partitions: 3,
                replication_factor: 1,
                assignments: vec![],
                configs: vec![
                    ConfigEntry {
                        name: "cleanup.policy".to_string(),
                        value: Some("delete".to_string()),
                    },
                    ConfigEntry {
                        name: "retention.ms".to_string(),
                        value: Some("2592000000".to_string()),
                    },
                ],
            })
            .collect(),
        timeout_ms: 5000,
    };
    let response = client.create_topics(&request).await?;

    let mut warnings = Vec::new();
    for result in response.topics {
        if result.error_code != 0 {
            warnings.push(TopicCreationWarning {
                topic: result.name,
                error_code: result.error_code,
            });
        }
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_topic_covers_exactly_the_six_scenario_builder_event_types() {
        assert_eq!(
            event_topic("StatusChanged"),
            Some("manpower.personnel-lifecycle.status-changes")
        );
        assert_eq!(
            event_topic("PersonnelAssigned"),
            Some("manpower.personnel-lifecycle.assignments")
        );
        assert_eq!(
            event_topic("PersonnelPromoted"),
            Some("manpower.personnel-lifecycle.promotions")
        );
        assert_eq!(
            event_topic("PersonnelSeparated"),
            Some("manpower.personnel-lifecycle.separations")
        );
        assert_eq!(
            event_topic("PositionAuthorizationChanged"),
            Some("manpower.position-management.authorization-changes")
        );
        assert_eq!(
            event_topic("PositionFilled"),
            Some("manpower.position-management.fills")
        );
    }

    #[test]
    fn event_topic_has_no_entry_for_readiness_or_unknown_types() {
        assert_eq!(event_topic("UnitReadinessAssessed"), None);
        assert_eq!(event_topic("SomethingElse"), None);
    }

    #[test]
    fn phase4_topics_includes_the_readiness_topic_even_though_nothing_publishes_to_it() {
        assert!(PHASE4_TOPICS.contains(&"manpower.readiness-reporting.assessments"));
    }

    #[rusty_tokio::test]
    async fn create_phase4_topics_creates_every_missing_topic_in_one_request() {
        use rusty_kafka::protocol::api_key;
        use rusty_kafka::protocol::create_topics::CreatableTopicResult;
        use rusty_kafka::protocol::create_topics::CreateTopicsResponse;
        use rusty_kafka::protocol::metadata::MetadataResponse;
        use rusty_kafka::testing::{recv_request, send_response};
        use rusty_tokio::io::duplex;
        use rusty_wire::{Reader, Writer};

        let (client_io, mut peer) = duplex(16384);
        let mut client = KafkaClient::new(client_io, None);

        let server = rusty_tokio::spawn(async move {
            let (header, _body) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::METADATA);
            // No topics exist yet.
            let response = MetadataResponse {
                brokers: vec![],
                topics: vec![],
            };
            let mut writer = Writer::new();
            write_i32(&mut writer, response.brokers.len() as i32);
            write_i32(&mut writer, response.topics.len() as i32);
            send_response(&mut peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();

            let (header, body) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::CREATE_TOPICS);
            let mut reader = Reader::new(&body);
            let decoded = CreateTopicsRequest::decode(&mut reader).unwrap();
            assert_eq!(decoded.topics.len(), PHASE4_TOPICS.len());

            let response = CreateTopicsResponse {
                topics: decoded
                    .topics
                    .iter()
                    .map(|t| CreatableTopicResult {
                        name: t.name.clone(),
                        error_code: 0,
                    })
                    .collect(),
            };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();
        });

        let warnings = create_phase4_topics(&mut client).await.unwrap();
        server.await.unwrap();
        assert!(warnings.is_empty());

        fn write_i32(writer: &mut Writer, v: i32) {
            writer.write_u32_be(v as u32);
        }
    }

    #[rusty_tokio::test]
    async fn create_phase4_topics_skips_already_existing_topics() {
        use rusty_kafka::protocol::api_key;
        use rusty_kafka::protocol::create_topics::{CreatableTopicResult, CreateTopicsResponse};
        use rusty_kafka::testing::{recv_request, send_response};
        use rusty_tokio::io::duplex;
        use rusty_wire::Writer;

        let (client_io, mut peer) = duplex(16384);
        let mut client = KafkaClient::new(client_io, None);

        let server = rusty_tokio::spawn(async move {
            let (header, _body) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::METADATA);
            // Every topic already exists.
            let mut writer = Writer::new();
            write_i32(&mut writer, 0); // brokers
            write_i32(&mut writer, PHASE4_TOPICS.len() as i32);
            for name in PHASE4_TOPICS {
                write_i16(&mut writer, 0); // error_code
                write_string(&mut writer, name);
                write_i32(&mut writer, 0); // partitions
            }
            send_response(&mut peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();
            // No CreateTopics request expected -- nothing is missing.
            let _ = CreateTopicsResponse {
                topics: Vec::<CreatableTopicResult>::new(),
            };
        });

        let warnings = create_phase4_topics(&mut client).await.unwrap();
        server.await.unwrap();
        assert!(warnings.is_empty());

        fn write_i32(writer: &mut Writer, v: i32) {
            writer.write_u32_be(v as u32);
        }
        fn write_i16(writer: &mut Writer, v: i16) {
            writer.write_u16_be(v as u16);
        }
        fn write_string(writer: &mut Writer, v: &str) {
            write_i16(writer, v.len() as i16);
            writer.write_bytes(v.as_bytes());
        }
    }

    #[rusty_tokio::test]
    async fn create_phase4_topics_reports_a_per_topic_broker_error_as_a_warning_not_a_failure() {
        use rusty_kafka::protocol::api_key;
        use rusty_kafka::protocol::create_topics::{CreatableTopicResult, CreateTopicsResponse};
        use rusty_kafka::protocol::metadata::MetadataResponse;
        use rusty_kafka::testing::{recv_request, send_response};
        use rusty_tokio::io::duplex;
        use rusty_wire::{Reader, Writer};

        let (client_io, mut peer) = duplex(16384);
        let mut client = KafkaClient::new(client_io, None);

        let server = rusty_tokio::spawn(async move {
            let (header, _body) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::METADATA);
            let response = MetadataResponse {
                brokers: vec![],
                topics: vec![],
            };
            let mut writer = Writer::new();
            write_i32(&mut writer, response.brokers.len() as i32);
            write_i32(&mut writer, response.topics.len() as i32);
            send_response(&mut peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();

            let (header, body) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::CREATE_TOPICS);
            let mut reader = Reader::new(&body);
            let decoded = CreateTopicsRequest::decode(&mut reader).unwrap();

            let response = CreateTopicsResponse {
                topics: decoded
                    .topics
                    .iter()
                    .enumerate()
                    .map(|(i, t)| CreatableTopicResult {
                        name: t.name.clone(),
                        error_code: if i == 0 { 37 } else { 0 }, // INVALID_PARTITIONS on the first, success elsewhere
                    })
                    .collect(),
            };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();
        });

        let warnings = create_phase4_topics(&mut client).await.unwrap();
        server.await.unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].error_code, 37);

        fn write_i32(writer: &mut Writer, v: i32) {
            writer.write_u32_be(v as u32);
        }
    }
}
