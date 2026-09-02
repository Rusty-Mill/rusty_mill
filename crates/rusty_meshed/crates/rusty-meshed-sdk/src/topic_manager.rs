//! [`TopicManager`]: the single interface for Kafka topic lifecycle
//! management -- the Rust port of `meshed.infrastructure.topic_manager`.
//! No code outside this module should call
//! `rusty_kafka::KafkaClient::create_topics` directly; all topic
//! creation, configuration, and deprecation goes through
//! [`TopicManager`] so naming convention and compaction policy are
//! enforced unconditionally.

use crate::topic_config::{TopicSpec, TopicType};
use rusty_err::Error;
use rusty_kafka::protocol::create_topics::{CreatableTopic, CreateTopicsRequest};
use rusty_kafka::{ClientError, KafkaClient};
use rusty_tokio::io::{AsyncRead, AsyncWrite};
use std::collections::HashMap;
use std::time::SystemTime;

/// Well-known stream-type suffixes, listed for documentation purposes
/// only -- not enforced by [`validate_topic_name`] (a port-name suffix
/// like `assignments` or `status-changes` is also a valid stream type).
/// Matches the Python source's own `_WELL_KNOWN_STREAM_TYPES`, which is
/// itself unreferenced anywhere in that codebase.
pub const WELL_KNOWN_STREAM_TYPES: &[&str] = &["events", "state", "commands", "dlq"];

/// Raised when a topic name violates the platform naming convention:
/// `{domain}.{product}.{stream-type}`, each segment lowercase
/// alphanumeric-plus-hyphens starting with a letter.
///
/// A one-variant enum rather than a tuple struct: `rusty_err`'s
/// `#[derive(Error)]` only supports enums today.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TopicNameError {
    /// `{0}` is the offending topic name.
    #[error(
        "Topic name '{0}' violates convention {{domain}}.{{product}}.{{stream-type}}. Rules: all three segments must be lowercase alphanumeric with optional hyphens and must start with a letter (e.g. manpower.personnel-lifecycle.assignments)."
    )]
    InvalidName(String),
}

/// Errors from [`TopicManager::create_topic`]: either the name failed
/// validation, the connection/framing failed, or the broker rejected
/// the topic (a non-zero Kafka error code in the response -- including
/// `TOPIC_ALREADY_EXISTS`; this layer does not swallow that, see
/// SDK-075, `DataProductProducerBase::startup` is where that
/// idempotency handling belongs).
#[derive(Debug, Error)]
pub enum CreateTopicError {
    /// `spec.name` failed [`validate_topic_name`].
    #[error("{0}")]
    InvalidName(#[from] TopicNameError),
    /// The connection to the broker failed, or its response didn't
    /// decode.
    #[error("{0}")]
    Client(#[from] ClientError),
    /// The broker's `CreateTopics` response reported a non-zero error
    /// code for this topic.
    #[error("broker rejected topic '{0}' with error code {1}")]
    Rejected(String, i16),
}

/// One topic's status, as reported by [`TopicManager::list_topics`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicStatus {
    /// Full topic name.
    pub name: String,
    /// The topic's configured type.
    pub topic_type: TopicType,
    /// Whether [`TopicManager::deprecate_topic`] has been called for
    /// this topic.
    pub deprecated: bool,
    /// When the topic was deprecated, if it has been.
    pub deprecated_at: Option<SystemTime>,
}

/// Validates `name` against the `{domain}.{product}.{stream-type}`
/// convention: exactly three dot-separated segments, each lowercase
/// alphanumeric-plus-hyphens starting with a lowercase letter.
pub fn validate_topic_name(name: &str) -> Result<(), TopicNameError> {
    let mut parts = name.split('.');
    let valid = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(a), Some(b), Some(c), None) => {
            is_valid_segment(a) && is_valid_segment(b) && is_valid_segment(c)
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(TopicNameError::InvalidName(name.to_string()))
    }
}

fn is_valid_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {
            chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        }
        _ => false,
    }
}

/// The default `CreateTopics` request timeout this manager uses. Not
/// itself ported from the Python source -- `TopicManager` there never
/// sets a timeout explicitly, delegating to `confluent_kafka`'s own
/// internal default (also 30s), which isn't a value the source exposes
/// or tests.
const CREATE_TOPICS_TIMEOUT_MS: i32 = 30_000;

/// Single interface for topic lifecycle management. Enforces the
/// `{domain}.{product}.{stream-type}` naming convention before
/// delegating to a [`rusty_kafka::KafkaClient`]. Tracks created and
/// deprecated topics in an in-memory registry for the lifetime of this
/// instance.
pub struct TopicManager<S> {
    client: KafkaClient<S>,
    registry: HashMap<String, TopicSpec>,
    deprecated: HashMap<String, SystemTime>,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> TopicManager<S> {
    /// Wraps an already-constructed [`KafkaClient`], injected at
    /// construction time -- matching the Python source's
    /// constructor-kwarg DI pattern (no direct client instantiation
    /// here).
    pub fn new(client: KafkaClient<S>) -> Self {
        TopicManager {
            client,
            registry: HashMap::new(),
            deprecated: HashMap::new(),
        }
    }

    /// Creates a Kafka topic, enforcing naming convention and config
    /// policy. Validates the name *before* any Kafka call. Does not
    /// swallow a `TOPIC_ALREADY_EXISTS` response -- see this module's
    /// doc comment.
    pub async fn create_topic(&mut self, spec: TopicSpec) -> Result<(), CreateTopicError> {
        validate_topic_name(&spec.name)?;

        let request = CreateTopicsRequest {
            topics: vec![CreatableTopic {
                name: spec.name.clone(),
                num_partitions: spec.num_partitions as i32,
                replication_factor: spec.replication_factor as i16,
                assignments: vec![],
                configs: spec.kafka_config(),
            }],
            timeout_ms: CREATE_TOPICS_TIMEOUT_MS,
        };

        let response = self.client.create_topics(&request).await?;
        for result in &response.topics {
            if result.error_code != 0 {
                return Err(CreateTopicError::Rejected(
                    result.name.clone(),
                    result.error_code,
                ));
            }
        }

        self.registry.insert(spec.name.clone(), spec);
        Ok(())
    }

    /// Marks a topic as deprecated without removing it from Kafka. The
    /// topic remains in Kafka and continues to serve existing
    /// consumers. Performs no membership check against the registry --
    /// calling this with a name never created via this instance
    /// silently records a deprecation that [`list_topics`](Self::list_topics)
    /// will never surface, matching the Python source's own behavior
    /// (flagged as a possible design gap in `capability-manifest.md`,
    /// not something this port unilaterally changes).
    pub fn deprecate_topic(&mut self, name: &str) {
        self.deprecated.insert(name.to_string(), SystemTime::now());
    }

    /// Returns every topic created via this instance, with its
    /// deprecation status. A topic created through a different
    /// `TopicManager` instance never appears here.
    pub fn list_topics(&self) -> Vec<TopicStatus> {
        self.registry
            .iter()
            .map(|(name, spec)| {
                let deprecated_at = self.deprecated.get(name).copied();
                TopicStatus {
                    name: name.clone(),
                    topic_type: spec.topic_type,
                    deprecated: deprecated_at.is_some(),
                    deprecated_at,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_kafka::protocol::create_topics::{
        CreatableTopicResult, CreateTopicsRequest as WireCreateTopicsRequest, CreateTopicsResponse,
        TOPIC_ALREADY_EXISTS,
    };
    use rusty_kafka::testing::{recv_request, send_response};
    use rusty_tokio::io::duplex;
    use rusty_wire::{Reader, Writer};

    /// Reads one `CreateTopics` request off `peer` and returns the
    /// correlation_id (for replying) plus the decoded request body.
    async fn recv_create_topics_request<T: AsyncRead + Unpin + Send>(
        peer: &mut T,
    ) -> (i32, WireCreateTopicsRequest) {
        let (header, body) = recv_request(peer).await.unwrap();
        let mut reader = Reader::new(&body);
        (
            header.correlation_id,
            WireCreateTopicsRequest::decode(&mut reader).unwrap(),
        )
    }

    /// Sends a `CreateTopics` v0 response with one result per name in
    /// `results` (name, error_code).
    async fn send_create_topics_response<T: AsyncWrite + Unpin + Send>(
        peer: &mut T,
        correlation_id: i32,
        results: &[(&str, i16)],
    ) {
        let response = CreateTopicsResponse {
            topics: results
                .iter()
                .map(|(name, error_code)| CreatableTopicResult {
                    name: name.to_string(),
                    error_code: *error_code,
                })
                .collect(),
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        send_response(peer, correlation_id, writer.as_slice())
            .await
            .unwrap();
    }

    #[test]
    fn valid_topic_name_accepted() {
        assert!(validate_topic_name("manpower.personnel.events").is_ok());
    }

    #[test]
    fn valid_topic_name_with_hyphens_accepted() {
        assert!(validate_topic_name("manpower.personnel-lifecycle.events").is_ok());
    }

    #[test]
    fn port_name_suffix_accepted() {
        assert!(validate_topic_name("manpower.personnel-lifecycle.assignments").is_ok());
    }

    #[test]
    fn hyphenated_port_name_suffix_accepted() {
        assert!(validate_topic_name("manpower.personnel-lifecycle.status-changes").is_ok());
    }

    #[test]
    fn missing_domain_prefix_rejected() {
        let err = validate_topic_name("personnel-events").unwrap_err();
        assert!(err.to_string().contains("violates convention"));
    }

    #[test]
    fn two_segment_name_rejected() {
        assert!(validate_topic_name("manpower.personnel").is_err());
    }

    #[test]
    fn numeric_leading_segment_rejected() {
        assert!(validate_topic_name("1manpower.personnel.events").is_err());
    }

    #[test]
    fn uppercase_name_rejected() {
        assert!(validate_topic_name("Manpower.Personnel.events").is_err());
    }

    #[test]
    fn uppercase_stream_type_rejected() {
        let err = validate_topic_name("manpower.personnel.Events").unwrap_err();
        assert!(err.to_string().contains("violates convention"));
    }

    #[rusty_tokio::test]
    async fn create_state_topic_sends_compact_config() {
        let (client_io, mut peer) = duplex(4096);
        let mut manager = TopicManager::new(KafkaClient::new(client_io, None));

        let server = rusty_tokio::spawn(async move {
            let (correlation_id, request) = recv_create_topics_request(&mut peer).await;
            let name = request.topics[0].name.clone();
            assert_eq!(name, "manpower.personnel.state");
            send_create_topics_response(&mut peer, correlation_id, &[(&name, 0)]).await;
        });

        let spec = TopicSpec::new("manpower.personnel.state", TopicType::State);
        manager.create_topic(spec).await.unwrap();
        server.await.unwrap();

        assert_eq!(manager.list_topics().len(), 1);
    }

    #[rusty_tokio::test]
    async fn create_topic_sets_partitions_and_replication() {
        let (client_io, mut peer) = duplex(4096);
        let mut manager = TopicManager::new(KafkaClient::new(client_io, None));

        let server = rusty_tokio::spawn(async move {
            let (correlation_id, request) = recv_create_topics_request(&mut peer).await;
            let name = request.topics[0].name.clone();
            assert_eq!(request.topics[0].num_partitions, 6);
            assert_eq!(request.topics[0].replication_factor, 2);
            send_create_topics_response(&mut peer, correlation_id, &[(&name, 0)]).await;
        });

        let mut spec = TopicSpec::new("manpower.personnel.events", TopicType::Events);
        spec.num_partitions = 6;
        spec.replication_factor = 2;
        manager.create_topic(spec).await.unwrap();
        server.await.unwrap();
    }

    #[rusty_tokio::test]
    async fn create_topic_propagates_broker_error() {
        let (client_io, mut peer) = duplex(4096);
        let mut manager = TopicManager::new(KafkaClient::new(client_io, None));

        let server = rusty_tokio::spawn(async move {
            let (correlation_id, request) = recv_create_topics_request(&mut peer).await;
            let name = request.topics[0].name.clone();
            send_create_topics_response(
                &mut peer,
                correlation_id,
                &[(&name, TOPIC_ALREADY_EXISTS)],
            )
            .await;
        });

        let spec = TopicSpec::new("manpower.personnel.events", TopicType::Events);
        let err = manager.create_topic(spec).await.unwrap_err();
        server.await.unwrap();

        assert!(matches!(
            err,
            CreateTopicError::Rejected(name, code)
                if name == "manpower.personnel.events" && code == TOPIC_ALREADY_EXISTS
        ));
        // A rejected topic is never recorded as managed.
        assert!(manager.list_topics().is_empty());
    }

    #[rusty_tokio::test]
    async fn create_topic_rejects_invalid_name_before_any_kafka_call() {
        let (client_io, _peer) = duplex(4096);
        let mut manager = TopicManager::new(KafkaClient::new(client_io, None));

        let spec = TopicSpec::new("personnel-events", TopicType::Events);
        let err = manager.create_topic(spec).await.unwrap_err();
        assert!(matches!(err, CreateTopicError::InvalidName(_)));
    }

    #[rusty_tokio::test]
    async fn deprecate_topic_marks_it_deprecated() {
        let (client_io, mut peer) = duplex(4096);
        let mut manager = TopicManager::new(KafkaClient::new(client_io, None));

        let server = rusty_tokio::spawn(async move {
            let (correlation_id, request) = recv_create_topics_request(&mut peer).await;
            let name = request.topics[0].name.clone();
            send_create_topics_response(&mut peer, correlation_id, &[(&name, 0)]).await;
        });
        manager
            .create_topic(TopicSpec::new(
                "manpower.personnel.events",
                TopicType::Events,
            ))
            .await
            .unwrap();
        server.await.unwrap();

        manager.deprecate_topic("manpower.personnel.events");
        let topics = manager.list_topics();
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].name, "manpower.personnel.events");
        assert!(topics[0].deprecated);
        assert!(topics[0].deprecated_at.is_some());
    }

    #[rusty_tokio::test]
    async fn list_topics_returns_all_managed_topics() {
        let (client_io, mut peer) = duplex(4096);
        let mut manager = TopicManager::new(KafkaClient::new(client_io, None));

        let server = rusty_tokio::spawn(async move {
            for _ in 0..2 {
                let (correlation_id, request) = recv_create_topics_request(&mut peer).await;
                let name = request.topics[0].name.clone();
                send_create_topics_response(&mut peer, correlation_id, &[(&name, 0)]).await;
            }
        });

        manager
            .create_topic(TopicSpec::new(
                "manpower.personnel.events",
                TopicType::Events,
            ))
            .await
            .unwrap();
        manager
            .create_topic(TopicSpec::new("manpower.personnel.state", TopicType::State))
            .await
            .unwrap();
        server.await.unwrap();

        let topics = manager.list_topics();
        assert_eq!(topics.len(), 2);
        let names: std::collections::HashSet<_> = topics.iter().map(|t| t.name.clone()).collect();
        assert!(names.contains("manpower.personnel.events"));
        assert!(names.contains("manpower.personnel.state"));
        assert!(topics.iter().all(|t| !t.deprecated));
    }
}
