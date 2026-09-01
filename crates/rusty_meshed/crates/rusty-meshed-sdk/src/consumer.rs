//! [`DataProductConsumerBase`] -- the Rust port of
//! `meshed.sdk.consumer.DataProductConsumerBase`, **partially** ported
//! (SDK-029..033, SDK-037..038 of SDK-029..039).
//!
//! # What's blocked, and why
//!
//! `startup()`'s Steps 3-5 (SDK-034..036: build a
//! `DeserializingConsumer`, subscribe it to the resolved topic, record
//! post-subscribe lineage) and the actual poll loop (SDK-039) all need
//! `rusty_kafka` to fetch messages and manage consumer-group offsets --
//! `FindCoordinator`/`JoinGroup`/`SyncGroup`/`Heartbeat`/`OffsetCommit`/
//! `Fetch`. `rusty_kafka`'s own module doc lists every one of those
//! under "Not yet implemented" (the producer side, `Produce`, is what
//! landed; a matching consumer side is explicitly called out there as
//! "deferred to a follow-up pass"). Nothing here works around that --
//! see GitHub issue #87's pattern for the two Kafka gaps this crate
//! family has already worked through (`Produce`, then `CreateTopics`
//! before it): this is the next one, filed the same way rather than
//! faked.
//!
//! What SDK-034..036/039 need is deferred as a whole -- there's no
//! meaningful partial "subscribe" or "poll" to build without `Fetch`,
//! the same reasoning `rusty-meshed-sdk::outbox`'s own module doc gave
//! for deferring `OutboxRelay` as one cluster before `Produce` existed.
//! Concretely: this crate exposes
//! [`resolve_output_port`](DataProductConsumerBase::resolve_output_port)
//! (SDK-031..033 -- topic resolution plus contract validation, which
//! need only the Registry HTTP API, not Kafka at all) and
//! [`is_duplicate`](DataProductConsumerBase::is_duplicate) (SDK-037/038
//! -- pure in-memory dedup, needing no Kafka either) as genuinely
//! standalone, independently useful, independently tested capabilities
//! -- not a `startup()` method that only pretends to finish. There is
//! no `startup()`/`run()`/`stop()`/`process()` here yet; add them
//! alongside a `Fetch`-based `rusty_kafka` consumer.
//!
//! # `event_type` becomes a type parameter
//!
//! The source's `event_type: type[BaseEvent]` class attribute names,
//! at the class level, the event type this consumer's topic carries.
//! Since a `DataProductConsumerBase` subclass declares exactly one
//! `event_type` (unlike a producer's `output_ports`, which can span
//! several event types per instance -- see `producer`'s own module
//! doc), this port makes it a single generic parameter, `E:
//! DomainEvent`, on [`DataProductConsumerBase`] itself rather than a
//! type-erased descriptor: `E::EVENT_NAME` is exactly
//! `event_type.__name__` (SDK-032's `ContractVersionMismatch` check).

use crate::registry_client::RegistryClient;
use crate::{ContractVersionMismatch, RegistryError};
use rusty_err::Error;
use rusty_meshed_core::{DomainEvent, PlatformConfig};
use rusty_meshed_observability::LineageTracker;
use std::collections::HashSet;
use std::marker::PhantomData;

/// Errors from [`DataProductConsumerBase::connect`]/
/// [`DataProductConsumerBase::resolve_output_port`].
#[derive(Debug, Error)]
pub enum ConsumerStartupError {
    /// A Data Product Registry HTTP call failed.
    #[error("{0}")]
    Registry(#[from] RegistryError),
    /// The resolved output port's JSON was missing a field this method
    /// needs (`topic_name`, `id`, or `data_product_id`).
    #[error("output port response missing '{0}' field")]
    MissingField(&'static str),
    /// The registry's published contract's `schema_ref` didn't contain
    /// `E::EVENT_NAME` (SDK-032).
    #[error("{0}")]
    ContractMismatch(#[from] ContractVersionMismatch),
    /// [`connect`](DataProductConsumerBase::connect)'s
    /// `LineageTracker::new` call failed.
    #[error("lineage tracker initialization failed: {0}")]
    Lineage(String),
}

/// Abstract base for meshed data product consumers (SDK-029), ported
/// so far as [`resolve_output_port`](Self::resolve_output_port)
/// (SDK-031..033) and [`is_duplicate`](Self::is_duplicate)
/// (SDK-037/038) -- see the module doc for what's still blocked and
/// why.
pub struct DataProductConsumerBase<E> {
    product_name: String,
    port_name: String,
    group_id: String,
    registry_client: RegistryClient,
    lineage_tracker: LineageTracker,
    /// Unbounded, in-memory, never evicted -- deduplication is lost on
    /// restart and not shared across processes, matching the source's
    /// own documented limitation (SDK-038) rather than a gap
    /// introduced here.
    seen_event_ids: HashSet<String>,
    _event_type: PhantomData<E>,
}

impl<E: DomainEvent> DataProductConsumerBase<E> {
    /// Builds a `LineageTracker` from `config` and a `RegistryClient`
    /// pointed at `config.registry_base_url` (SDK-030's DI defaults,
    /// built eagerly here rather than lazily -- see `producer`'s own
    /// module doc for why).
    pub async fn connect(
        product_name: impl Into<String>,
        port_name: impl Into<String>,
        group_id: impl Into<String>,
        config: &PlatformConfig,
    ) -> Result<Self, ConsumerStartupError> {
        let registry_client = RegistryClient::new(config.registry_base_url.clone());
        let lineage_tracker = LineageTracker::new(config.registry_db_path.clone())
            .map_err(|err| ConsumerStartupError::Lineage(err.to_string()))?;
        Ok(Self::new(
            product_name,
            port_name,
            group_id,
            registry_client,
            lineage_tracker,
        ))
    }

    /// Wraps already-constructed dependencies -- the seam this crate's
    /// own tests use (a fake HTTP server for `registry_client`, an
    /// in-memory `LineageTracker` at a temp path).
    pub fn new(
        product_name: impl Into<String>,
        port_name: impl Into<String>,
        group_id: impl Into<String>,
        registry_client: RegistryClient,
        lineage_tracker: LineageTracker,
    ) -> Self {
        DataProductConsumerBase {
            product_name: product_name.into(),
            port_name: port_name.into(),
            group_id: group_id.into(),
            registry_client,
            lineage_tracker,
            seen_event_ids: HashSet::new(),
            _event_type: PhantomData,
        }
    }

    /// This consumer's configured Kafka consumer group ID.
    pub fn group_id(&self) -> &str {
        &self.group_id
    }

    /// Read-only access to this consumer's `LineageTracker` -- what a
    /// future `startup()` would use for SDK-036's post-subscribe
    /// job-run recording, once it exists.
    pub fn lineage_tracker(&self) -> &LineageTracker {
        &self.lineage_tracker
    }

    /// Resolves this consumer's output port and validates its
    /// published contract against `E::EVENT_NAME` (SDK-031..033).
    /// Returns the resolved Kafka topic -- what a `Fetch`-capable
    /// `startup()` would go on to subscribe to (SDK-034/035, not built
    /// yet, see the module doc).
    ///
    /// 1. Resolves the output port via
    ///    `RegistryClient::get_output_port` (SDK-031).
    /// 2. Fetches the port's contract via `RegistryClient::get_contract`;
    ///    skips validation entirely when `None` (no contract published
    ///    yet, SDK-033); when a contract exists and carries a
    ///    `schema_ref` not containing `E::EVENT_NAME`, returns
    ///    [`ConsumerStartupError::ContractMismatch`] (SDK-032).
    pub async fn resolve_output_port(&self) -> Result<String, ConsumerStartupError> {
        let port = self
            .registry_client
            .get_output_port(&self.product_name, &self.port_name)
            .await?;
        let topic = port
            .get("topic_name")
            .and_then(|value| value.as_str())
            .ok_or(ConsumerStartupError::MissingField("topic_name"))?
            .to_string();
        let port_id = port
            .get("id")
            .and_then(|value| value.as_f64())
            .ok_or(ConsumerStartupError::MissingField("id"))? as i64;
        let product_id =
            port.get("data_product_id")
                .and_then(|value| value.as_f64())
                .ok_or(ConsumerStartupError::MissingField("data_product_id"))? as i64;

        if let Some(contract) = self
            .registry_client
            .get_contract(product_id, port_id)
            .await?
        {
            if let Some(schema_ref) = contract.get("schema_ref").and_then(|value| value.as_str()) {
                if !schema_ref.contains(E::EVENT_NAME) {
                    return Err(ContractVersionMismatch::new(E::EVENT_NAME, schema_ref).into());
                }
            }
        }

        Ok(topic)
    }

    /// Returns `false` the first time `event_id` is seen (and records
    /// it), `true` on every subsequent call (SDK-037) -- what a future
    /// poll loop would use to run `process()` at most once per unique
    /// event ID.
    pub fn is_duplicate(&mut self, event_id: &str) -> bool {
        !self.seen_event_ids.insert(event_id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_http::async_tokio::AsyncTransport;
    use rusty_http::head::ResponseHead;
    use rusty_http::{HeaderMap, StatusCode, Version};
    use rusty_meshed_core::{AvroDecodeError, BaseEvent};

    struct TestEvent {
        base: BaseEvent,
    }

    impl DomainEvent for TestEvent {
        const EVENT_NAME: &'static str = "TestEvent";

        fn base(&self) -> &BaseEvent {
            &self.base
        }

        fn avro_schema() -> String {
            BaseEvent::avro_record_schema("TestEvent", "test", rusty_json::json!([]))
        }

        fn serialize(&self) -> Vec<u8> {
            self.base.serialize()
        }

        fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
            Ok(TestEvent {
                base: BaseEvent::deserialize(bytes)?,
            })
        }

        fn to_json(&self) -> rusty_json::Value {
            rusty_json::Value::Object(self.base.to_json_fields())
        }
    }

    fn temp_db_path(name: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "rusty_meshed_consumer_test_{name}_{}.db",
            rusty_uuid::Uuid::new_v4()
        ));
        path.to_str().unwrap().to_string()
    }

    struct CapturedHttpRequest {
        target: String,
    }

    /// A local copy of the same fake-HTTP-server helper `producer`'s
    /// own tests use -- see that module's test doc comment for why
    /// it's duplicated rather than shared.
    fn start_fake_http_server(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, rusty_tokio::JoinHandle<Vec<CapturedHttpRequest>>) {
        let listener = rusty_tokio::io::TcpListener::bind("127.0.0.1:0".parse().unwrap())
            .expect("failed to bind");
        let addr = listener.local_addr().expect("failed to read local_addr");
        let url = format!("http://{addr}");

        let handle = rusty_tokio::spawn(async move {
            let mut captured = Vec::new();
            for (status, response_body) in responses {
                let (stream, _peer) = listener.accept().await.expect("failed to accept");
                let mut transport = AsyncTransport::new(stream);
                let head = transport
                    .read_request_head(8192)
                    .await
                    .expect("failed to read request head");
                let framing = rusty_http::body::request_framing(&head.headers)
                    .expect("failed to determine framing");
                let _body_bytes = transport
                    .read_body(framing)
                    .await
                    .expect("failed to read request body");

                let mut headers = HeaderMap::new();
                headers
                    .insert("Content-Length", &response_body.len().to_string())
                    .unwrap();
                headers.insert("Content-Type", "application/json").unwrap();
                let response_head = ResponseHead {
                    status: StatusCode::from_u16(status),
                    reason: String::new(),
                    version: Version::Http11,
                    headers,
                };
                transport
                    .write_response_head(&response_head)
                    .await
                    .expect("failed to write response head");
                transport
                    .write_body(response_body.as_bytes())
                    .await
                    .expect("failed to write response body");

                captured.push(CapturedHttpRequest {
                    target: head.target.clone(),
                });
            }
            captured
        });

        (url, handle)
    }

    fn consumer_with(registry_client: RegistryClient) -> DataProductConsumerBase<TestEvent> {
        let db_path = temp_db_path("resolve");
        let lineage_tracker = LineageTracker::new(&db_path).unwrap();
        DataProductConsumerBase::new(
            "personnel-lifecycle",
            "assignments",
            "readiness-reporting-personnel-consumer",
            registry_client,
            lineage_tracker,
        )
    }

    #[rusty_tokio::test]
    async fn resolve_output_port_returns_the_topic_with_no_contract_published() {
        let (url, server) = start_fake_http_server(vec![
            (200, r#"[{"id": 1, "name": "personnel-lifecycle"}]"#),
            (
                200,
                r#"[{"id": 10, "data_product_id": 1, "description": "assignments", "topic_name": "manpower.personnel-lifecycle.assignments"}]"#,
            ),
            (404, "{}"),
        ]);
        let consumer = consumer_with(RegistryClient::new(&url));

        let topic = consumer.resolve_output_port().await.unwrap();
        assert_eq!(topic, "manpower.personnel-lifecycle.assignments");

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].target.contains("/contract"));
    }

    #[rusty_tokio::test]
    async fn resolve_output_port_passes_when_the_contract_schema_ref_contains_the_event_name() {
        let (url, _server) = start_fake_http_server(vec![
            (200, r#"[{"id": 1, "name": "personnel-lifecycle"}]"#),
            (
                200,
                r#"[{"id": 10, "data_product_id": 1, "description": "assignments", "topic_name": "manpower.personnel-lifecycle.assignments"}]"#,
            ),
            (200, r#"{"schema_ref": "TestEvent-value-1"}"#),
        ]);
        let consumer = consumer_with(RegistryClient::new(&url));

        let topic = consumer.resolve_output_port().await.unwrap();
        assert_eq!(topic, "manpower.personnel-lifecycle.assignments");
    }

    #[rusty_tokio::test]
    async fn resolve_output_port_fails_when_the_contract_schema_ref_omits_the_event_name() {
        let (url, _server) = start_fake_http_server(vec![
            (200, r#"[{"id": 1, "name": "personnel-lifecycle"}]"#),
            (
                200,
                r#"[{"id": 10, "data_product_id": 1, "description": "assignments", "topic_name": "manpower.personnel-lifecycle.assignments"}]"#,
            ),
            (200, r#"{"schema_ref": "SomeOtherEvent-value-1"}"#),
        ]);
        let consumer = consumer_with(RegistryClient::new(&url));

        let err = consumer.resolve_output_port().await.unwrap_err();
        assert!(matches!(err, ConsumerStartupError::ContractMismatch(_)));
    }

    #[test]
    fn is_duplicate_returns_false_once_then_true_on_repeats() {
        let mut consumer = consumer_with(RegistryClient::new("http://unused.invalid"));
        assert!(!consumer.is_duplicate("evt-1"));
        assert!(consumer.is_duplicate("evt-1"));
        assert!(!consumer.is_duplicate("evt-2"));
    }
}
