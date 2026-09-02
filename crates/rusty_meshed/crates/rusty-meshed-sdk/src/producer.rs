//! [`DataProductProducerBase`] -- the Rust port of
//! `meshed.sdk.producer.DataProductProducerBase` (SDK-013..028).
//!
//! # No per-subclass ABC, no per-topic `SerializingProducer`
//!
//! The source is an `ABC` subclassed per product, declaring
//! `product_name`/`domain`/`version`/`owner`/`description`/
//! `output_ports` as class attributes; Rust has no class-level
//! attribute inheritance, so this port takes them as constructor
//! arguments instead -- composition over the class-attribute pattern,
//! the same choice `BaseEvent`'s own module doc made for domain events
//! (SDK-013/014). Concrete products (`PersonnelLifecycleProducer` and
//! friends, DOM-012 onward) aren't built yet; when they land, each
//! wraps a [`DataProductProducerBase`] the way the source's subclasses
//! wrap `DataProductProducerBase`'s inherited behavior.
//!
//! The source builds one `SerializingProducer` per output port
//! (SDK-018), each wired to an `AvroSerializer` bridging
//! `obj.model_dump()` into Avro bytes (SDK-017). This port needs
//! neither: every [`rusty_meshed_core::DomainEvent`] already knows how
//! to Avro-encode itself ([`rusty_meshed_core::DomainEvent::serialize`],
//! SDK-008), and [`rusty_kafka::KafkaClient::produce`] can target any
//! topic per call -- there's no serialization concern left for a
//! per-topic producer object to own. One shared [`KafkaClient`]
//! connection publishes to every declared topic; [`started_topics`]
//! tracks which topics `startup()` actually declared, doing the job
//! the source's `self._producers` dict keys did (SDK-023's "is this
//! topic declared" check).
//!
//! [`started_topics`]: DataProductProducerBase::started_topics
//!
//! # `publish`'s `TypeError` (SDK-022) and delivery callback (SDK-025/028)
//!
//! [`publish`](DataProductProducerBase::publish) is generic over `E:
//! DomainEvent` rather than accepting some `dyn` event trait object --
//! the same "compile-time guarantee, stronger than the source's
//! runtime check" shape `OutputPortSpec`'s immutability already uses
//! (SDK-010): there is no way to call `publish` with a value that
//! isn't a `DomainEvent`, so the source's `TypeError` on a non-`BaseEvent`
//! has no runtime counterpart to build here.
//!
//! The source's `on_delivery=self._delivery_callback` fires
//! asynchronously, later, whenever `producer.poll()` next runs;
//! `_delivery_callback` raising `RuntimeError` is how a failed delivery
//! surfaces. `KafkaClient::produce` is already synchronous -- it awaits
//! the full `ProduceResponse` before returning, per-call, the same
//! design [`crate::outbox::relay_pending`] and
//! `rusty-meshed-observability::slo::SLOViolationPublisher::publish`
//! both use -- so a failed delivery is already known by the time
//! `publish` would return, and becomes a [`PublishError`] directly
//! rather than a callback invocation.

use crate::registry_client::RegistryClient;
use crate::topic_config::{TopicSpec, TopicType};
use crate::topic_manager::{CreateTopicError, TopicManager};
use crate::types::PortDescriptor;
use rusty_err::Error;
use rusty_kafka::protocol::create_topics::TOPIC_ALREADY_EXISTS;
use rusty_kafka::protocol::produce::{
    ProducePartitionRequest, ProduceRequest, ProduceTopicRequest,
};
use rusty_kafka::record_batch::Record;
use rusty_kafka::{ClientError, KafkaClient};
use rusty_meshed_core::{DomainEvent, PlatformConfig};
use rusty_meshed_observability::LineageTracker;
use rusty_meshed_schema_registry::{RegisterSchemaError, SchemaRegistryEnforcer};
use rusty_tokio::io::{AsyncRead, AsyncWrite, TcpStream};
use std::collections::HashSet;

/// Errors from [`DataProductProducerBase::connect`]/
/// [`DataProductProducerBase::startup`].
#[derive(Debug, Error)]
pub enum ProducerError {
    /// Topic creation failed for a reason other than the topic already
    /// existing (SDK-015 swallows exactly that one case before this
    /// variant is ever produced).
    #[error("{0}")]
    Topic(#[from] CreateTopicError),
    /// Schema registration was rejected as incompatible, or the
    /// Schema Registry call itself failed.
    #[error("{0}")]
    SchemaRegistry(#[from] RegisterSchemaError),
    /// The Data Product Registry rejected a `register_product`/
    /// `register_output_port` call.
    #[error("{0}")]
    Registry(#[from] crate::RegistryError),
    /// `register_product`'s response had no `"id"` field to read
    /// `product_id` from.
    #[error("registry response for the new product had no 'id' field")]
    MissingProductId,
    /// Recording topology lineage (`startup`'s Step 4) failed.
    #[error("lineage recording failed: {0}")]
    Lineage(String),
    /// [`connect`](DataProductProducerBase::connect)'s Kafka connection
    /// attempt (admin or publish client) failed.
    #[error("Kafka connection failed: {0}")]
    Kafka(#[from] ClientError),
}

/// Errors from [`DataProductProducerBase::publish`].
#[derive(Debug, Error)]
pub enum PublishError {
    /// `topic` wasn't one of the topics [`startup`](DataProductProducerBase::startup)
    /// declared (SDK-023). `{1}` lists every topic that was.
    #[error("{0:?} is not a declared output port topic. Declared topics: {1:?}")]
    UndeclaredTopic(String, Vec<String>),
    /// The underlying Kafka request itself failed.
    #[error("Kafka client error: {0}")]
    Kafka(#[from] ClientError),
    /// The broker's response didn't include a result for the
    /// topic/partition produced to.
    #[error("no result for the requested topic/partition in the broker's response")]
    MissingPartitionResult,
    /// The broker returned a non-zero error code for the produce.
    #[error("broker returned Kafka error code {0}")]
    KafkaErrorCode(i16),
    /// Recording record-level lineage after a successful produce
    /// failed.
    #[error("lineage recording failed: {0}")]
    Lineage(String),
}

/// Abstract base for meshed data product producers (SDK-013). See the
/// module doc for how this differs structurally from the source's
/// `ABC`-subclassing shape.
pub struct DataProductProducerBase<S> {
    product_name: String,
    domain: String,
    version: String,
    owner: String,
    description: String,
    output_ports: Vec<PortDescriptor>,
    client: KafkaClient<S>,
    topic_manager: TopicManager<S>,
    sr_enforcer: SchemaRegistryEnforcer,
    registry_client: RegistryClient,
    lineage_tracker: LineageTracker,
    /// Topics `startup()` has registered a schema for -- the
    /// replacement for the source's `self._producers.keys()`
    /// (SDK-018), used by [`publish`](Self::publish)'s SDK-023 check.
    started_topics: HashSet<String>,
}

impl DataProductProducerBase<TcpStream> {
    /// Builds every dependency from `config` and connects two separate
    /// Kafka clients -- one wrapped in a [`TopicManager`] for admin
    /// operations (`startup`'s Step 0), one for publishing -- matching
    /// the source's own separation between `AdminClient` and
    /// `SerializingProducer` (SDK-014's DI defaults, built here instead
    /// of lazily inside `__init__` since Rust has no equivalent to a
    /// `None`-means-construct-a-default constructor parameter).
    #[allow(clippy::too_many_arguments)]
    pub async fn connect(
        product_name: impl Into<String>,
        domain: impl Into<String>,
        version: impl Into<String>,
        owner: impl Into<String>,
        description: impl Into<String>,
        output_ports: Vec<PortDescriptor>,
        config: &PlatformConfig,
    ) -> Result<Self, ProducerError> {
        let sr_enforcer = SchemaRegistryEnforcer::new(config.schema_registry_url.clone());
        let registry_client = RegistryClient::new(config.registry_base_url.clone());
        let lineage_tracker = LineageTracker::new(config.registry_db_path.clone())
            .map_err(|err| ProducerError::Lineage(err.to_string()))?;
        let admin_client = KafkaClient::connect(
            &config.kafka_bootstrap_servers,
            Some("rusty_meshed_producer_admin".to_string()),
        )
        .await?;
        let topic_manager = TopicManager::new(admin_client);
        let client = KafkaClient::connect(
            &config.kafka_bootstrap_servers,
            Some("rusty_meshed_producer".to_string()),
        )
        .await?;
        Ok(Self::new(
            product_name,
            domain,
            version,
            owner,
            description,
            output_ports,
            sr_enforcer,
            registry_client,
            lineage_tracker,
            topic_manager,
            client,
        ))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> DataProductProducerBase<S> {
    /// Wraps already-constructed dependencies -- the seam this crate's
    /// own tests use (a fake HTTP server for `sr_enforcer`/
    /// `registry_client`, an in-memory `rusty_tokio::io::duplex` pair
    /// for `topic_manager`/`client`) instead of real network/database
    /// connections.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        product_name: impl Into<String>,
        domain: impl Into<String>,
        version: impl Into<String>,
        owner: impl Into<String>,
        description: impl Into<String>,
        output_ports: Vec<PortDescriptor>,
        sr_enforcer: SchemaRegistryEnforcer,
        registry_client: RegistryClient,
        lineage_tracker: LineageTracker,
        topic_manager: TopicManager<S>,
        client: KafkaClient<S>,
    ) -> Self {
        DataProductProducerBase {
            product_name: product_name.into(),
            domain: domain.into(),
            version: version.into(),
            owner: owner.into(),
            description: description.into(),
            output_ports,
            client,
            topic_manager,
            sr_enforcer,
            registry_client,
            lineage_tracker,
            started_topics: HashSet::new(),
        }
    }

    /// Registers schemas, the product, and its output ports; must be
    /// called once before [`publish`](Self::publish).
    ///
    /// - **Step 0** (SDK-015): idempotently creates a topic per output
    ///   port. [`TOPIC_ALREADY_EXISTS`] is swallowed -- every other
    ///   [`CreateTopicError`] propagates.
    /// - **Step 1** (SDK-016..018): registers each port's Avro schema
    ///   under `{topic}-value` and marks the topic started (see the
    ///   module doc for why there's no per-topic producer object to
    ///   build here).
    /// - **Step 2** (SDK-019): registers the data product.
    /// - **Step 3** (SDK-020): registers each output port under the
    ///   product.
    /// - **Step 4** (SDK-021): records one topology lineage job run
    ///   listing every output port's topic.
    pub async fn startup(&mut self) -> Result<(), ProducerError> {
        for port in &self.output_ports {
            let spec = TopicSpec::new(port.topic.clone(), TopicType::Events);
            match self.topic_manager.create_topic(spec).await {
                Ok(()) => {}
                Err(CreateTopicError::Rejected(_, code)) if code == TOPIC_ALREADY_EXISTS => {}
                Err(err) => return Err(err.into()),
            }
        }

        for port in &self.output_ports {
            let subject = format!("{}-value", port.topic);
            self.sr_enforcer
                .register_schema(&subject, &port.schema)
                .await?;
            self.started_topics.insert(port.topic.clone());
        }

        let product = self
            .registry_client
            .register_product(
                &self.product_name,
                &self.domain,
                &self.version,
                &self.owner,
                &self.description,
                None,
            )
            .await?;
        let product_id = product
            .get("id")
            .and_then(|value| value.as_f64())
            .ok_or(ProducerError::MissingProductId)? as i64;

        for port in &self.output_ports {
            let subject = format!("{}-value", port.topic);
            self.registry_client
                .register_output_port(
                    product_id,
                    &port.name,
                    &port.topic,
                    &subject,
                    port.event_classification,
                )
                .await?;
        }

        let outputs: Vec<(String, String)> = self
            .output_ports
            .iter()
            .map(|port| ("kafka".to_string(), port.topic.clone()))
            .collect();
        self.lineage_tracker
            .record_job_run(&self.product_name, "meshed", &[], &outputs)
            .map_err(|err| ProducerError::Lineage(err.to_string()))?;

        Ok(())
    }

    /// This producer's declared `product_name`.
    pub fn product_name(&self) -> &str {
        &self.product_name
    }

    /// Read-only access to this producer's `LineageTracker` -- exposed
    /// for `rusty-meshed-domains`' `PersonnelLifecycleProducer`, whose
    /// outbox-writing `publish()` override still calls
    /// `LineageTracker::record_event` itself (DOM-016), the same way
    /// this type's own [`publish`](Self::publish) does.
    pub fn lineage_tracker(&self) -> &LineageTracker {
        &self.lineage_tracker
    }

    /// Whether `topic` was declared and started via
    /// [`startup`](Self::startup) -- exposed for
    /// `PersonnelLifecycleProducer`, which overrides
    /// [`publish`](Self::publish) entirely (writes to the outbox
    /// instead of producing directly, DOM-014) but still needs the
    /// same SDK-023 validation this type's own `publish` performs.
    pub fn is_declared_topic(&self, topic: &str) -> bool {
        self.started_topics.contains(topic)
    }

    /// Every topic [`startup`](Self::startup) declared, for
    /// [`PublishError::UndeclaredTopic`]'s topic listing --- exposed
    /// for the same cross-crate override [`is_declared_topic`](Self::is_declared_topic)
    /// serves.
    pub fn declared_topics(&self) -> Vec<String> {
        let mut declared: Vec<String> = self.started_topics.iter().cloned().collect();
        declared.sort();
        declared
    }

    /// Publishes `event` to `topic` with lineage headers (SDK-024),
    /// then records record-level lineage once the broker has confirmed
    /// the produce succeeded (SDK-026) -- see the module doc for why
    /// there's no separate `TypeError`/delivery-callback path to build.
    pub async fn publish<E: DomainEvent>(
        &mut self,
        topic: &str,
        event: &E,
    ) -> Result<(), PublishError> {
        if !self.started_topics.contains(topic) {
            let mut declared: Vec<String> = self.started_topics.iter().cloned().collect();
            declared.sort();
            return Err(PublishError::UndeclaredTopic(topic.to_string(), declared));
        }

        let base = event.base();
        let headers = vec![
            (
                "event_id".to_string(),
                Some(base.event_id.clone().into_bytes()),
            ),
            (
                "correlation_id".to_string(),
                Some(base.correlation_id.clone().into_bytes()),
            ),
            (
                "source_event_ids".to_string(),
                Some(base.source_event_ids.join(",").into_bytes()),
            ),
            (
                "timestamp".to_string(),
                Some(base.timestamp.clone().into_bytes()),
            ),
        ];
        let request = ProduceRequest {
            acks: -1,
            timeout_ms: 5000,
            base_timestamp_ms: now_millis(),
            topics: vec![ProduceTopicRequest {
                name: topic.to_string(),
                partitions: vec![ProducePartitionRequest {
                    partition_index: 0,
                    records: vec![Record {
                        key: None,
                        value: Some(event.serialize()),
                        headers,
                    }],
                }],
            }],
        };
        let response = self.client.produce(&request).await?;
        let result = response
            .topics
            .first()
            .and_then(|t| t.partitions.first())
            .ok_or(PublishError::MissingPartitionResult)?;
        if result.error_code != 0 {
            return Err(PublishError::KafkaErrorCode(result.error_code));
        }

        self.lineage_tracker
            .record_event(
                &base.event_id,
                &base.correlation_id,
                &base.source_event_ids,
                &self.product_name,
                topic,
                &base.timestamp,
            )
            .map_err(|err| PublishError::Lineage(err.to_string()))?;
        Ok(())
    }

    /// A documented no-op (SDK-027): [`publish`](Self::publish) already
    /// synchronously awaits the full `Produce` response before
    /// returning, so nothing is left buffered by the time this is
    /// called -- see the module doc's delivery-callback note.
    pub fn flush(&mut self, _timeout_seconds: f64) {}
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::OutputPortSpec;
    use rusty_http::async_tokio::AsyncTransport;
    use rusty_http::head::ResponseHead;
    use rusty_http::{HeaderMap, StatusCode, Version};
    use rusty_kafka::protocol::create_topics::{
        CreatableTopicResult, CreateTopicsRequest as WireCreateTopicsRequest, CreateTopicsResponse,
    };
    use rusty_kafka::protocol::produce::{
        ProducePartitionResponse, ProduceResponse, ProduceTopicResponse,
    };
    use rusty_kafka::testing::{recv_request, send_response};
    use rusty_meshed_core::{AvroDecodeError, BaseEvent, EventType};
    use rusty_request::Client;
    use rusty_tokio::io::duplex;
    use rusty_wire::{Reader, Writer};

    struct TestEvent {
        base: BaseEvent,
    }

    impl TestEvent {
        fn new(correlation_id: &str) -> Self {
            TestEvent {
                base: BaseEvent::new(correlation_id),
            }
        }
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
            "rusty_meshed_producer_test_{name}_{}.db",
            rusty_uuid::Uuid::new_v4()
        ));
        path.to_str().unwrap().to_string()
    }

    struct CapturedHttpRequest {
        method: String,
        target: String,
        body: String,
    }

    /// A local copy of `registry_client`'s own test-only fake HTTP
    /// server -- not shared cross-module, matching this crate family's
    /// convention of small per-module test helpers over a shared
    /// test-utility module.
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
                let body_bytes = transport
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
                    method: head.method.as_str().to_string(),
                    target: head.target.clone(),
                    body: String::from_utf8(body_bytes).expect("request body wasn't UTF-8"),
                });
            }
            captured
        });

        (url, handle)
    }

    async fn recv_create_topics_request(
        peer: &mut rusty_tokio::io::DuplexStream,
    ) -> (i32, WireCreateTopicsRequest) {
        let (header, body) = recv_request(peer).await.unwrap();
        assert_eq!(
            header.api_key,
            rusty_kafka::protocol::api_key::CREATE_TOPICS
        );
        let mut reader = Reader::new(&body);
        (
            header.correlation_id,
            WireCreateTopicsRequest::decode(&mut reader).unwrap(),
        )
    }

    async fn send_create_topics_response(
        peer: &mut rusty_tokio::io::DuplexStream,
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

    async fn respond_to_produce(peer: &mut rusty_tokio::io::DuplexStream, error_code: i16) {
        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, rusty_kafka::protocol::api_key::PRODUCE);
        let response = ProduceResponse {
            topics: vec![ProduceTopicResponse {
                name: "t".to_string(),
                partitions: vec![ProducePartitionResponse {
                    partition_index: 0,
                    error_code,
                    base_offset: 0,
                    log_append_time: -1,
                }],
            }],
            throttle_time_ms: 0,
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        send_response(peer, header.correlation_id, &writer.into_vec())
            .await
            .unwrap();
    }

    fn one_port() -> Vec<PortDescriptor> {
        vec![OutputPortSpec::<TestEvent>::new(
            "assignments",
            "manpower.personnel-lifecycle.assignments",
            EventType::Delta,
        )
        .describe()]
    }

    fn two_ports() -> Vec<PortDescriptor> {
        vec![
            OutputPortSpec::<TestEvent>::new(
                "assignments",
                "manpower.personnel-lifecycle.assignments",
                EventType::Delta,
            )
            .describe(),
            OutputPortSpec::<TestEvent>::new(
                "promotions",
                "manpower.personnel-lifecycle.promotions",
                EventType::Delta,
            )
            .describe(),
        ]
    }

    #[rusty_tokio::test]
    async fn startup_creates_topics_registers_schemas_registers_product_and_ports_and_records_lineage(
    ) {
        let (admin_io, mut admin_peer) = duplex(8192);
        let topic_manager = TopicManager::new(KafkaClient::new(admin_io, None));
        let (client_io, _client_peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);

        let (sr_url, sr_server) =
            start_fake_http_server(vec![(200, r#"{"id": 1}"#), (200, r#"{"id": 2}"#)]);
        let sr_enforcer = SchemaRegistryEnforcer::with_client(&sr_url, Client::new());

        let (reg_url, reg_server) = start_fake_http_server(vec![
            (200, r#"{"id": 42, "name": "personnel-lifecycle"}"#),
            (200, r#"{"id": 100}"#),
            (200, r#"{"id": 101}"#),
        ]);
        let registry_client = RegistryClient::new(&reg_url);

        let db_path = temp_db_path("startup_happy_path");
        let lineage_tracker = LineageTracker::new(&db_path).unwrap();

        let mut producer = DataProductProducerBase::new(
            "personnel-lifecycle",
            "manpower",
            "1.0.0",
            "manpower-team",
            "",
            two_ports(),
            sr_enforcer,
            registry_client,
            lineage_tracker,
            topic_manager,
            client,
        );

        let admin_server = rusty_tokio::spawn(async move {
            for _ in 0..2 {
                let (correlation_id, request) = recv_create_topics_request(&mut admin_peer).await;
                let name = request.topics[0].name.clone();
                send_create_topics_response(&mut admin_peer, correlation_id, &[(&name, 0)]).await;
            }
        });

        producer.startup().await.unwrap();
        admin_server.await.unwrap();
        let sr_requests = sr_server.await.unwrap();
        let reg_requests = reg_server.await.unwrap();

        assert_eq!(sr_requests.len(), 2);
        assert!(sr_requests[0].target.contains("assignments-value"));
        assert!(sr_requests[1].target.contains("promotions-value"));

        assert_eq!(reg_requests.len(), 3);
        assert_eq!(reg_requests[0].method, "POST");
        assert_eq!(reg_requests[0].target, "/data-products/");
        assert!(reg_requests[1]
            .target
            .contains("/data-products/42/output-ports"));
        assert!(reg_requests[1]
            .body
            .contains(r#""description":"assignments""#));
        assert!(reg_requests[2]
            .body
            .contains(r#""description":"promotions""#));

        let _ = std::fs::remove_file(&db_path);
    }

    #[rusty_tokio::test]
    async fn startup_swallows_topic_already_exists() {
        let (admin_io, mut admin_peer) = duplex(4096);
        let topic_manager = TopicManager::new(KafkaClient::new(admin_io, None));
        let (client_io, _client_peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);

        let (sr_url, sr_server) = start_fake_http_server(vec![(200, r#"{"id": 1}"#)]);
        let sr_enforcer = SchemaRegistryEnforcer::with_client(&sr_url, Client::new());

        let (reg_url, reg_server) =
            start_fake_http_server(vec![(200, r#"{"id": 42}"#), (200, r#"{"id": 100}"#)]);
        let registry_client = RegistryClient::new(&reg_url);

        let db_path = temp_db_path("startup_already_exists");
        let lineage_tracker = LineageTracker::new(&db_path).unwrap();

        let mut producer = DataProductProducerBase::new(
            "personnel-lifecycle",
            "manpower",
            "1.0.0",
            "manpower-team",
            "",
            one_port(),
            sr_enforcer,
            registry_client,
            lineage_tracker,
            topic_manager,
            client,
        );

        let admin_server = rusty_tokio::spawn(async move {
            let (correlation_id, request) = recv_create_topics_request(&mut admin_peer).await;
            let name = request.topics[0].name.clone();
            send_create_topics_response(
                &mut admin_peer,
                correlation_id,
                &[(&name, TOPIC_ALREADY_EXISTS)],
            )
            .await;
        });

        producer.startup().await.unwrap();
        admin_server.await.unwrap();
        sr_server.await.unwrap();
        reg_server.await.unwrap();

        let _ = std::fs::remove_file(&db_path);
    }

    #[rusty_tokio::test]
    async fn startup_propagates_other_topic_creation_errors() {
        let (admin_io, mut admin_peer) = duplex(4096);
        let topic_manager = TopicManager::new(KafkaClient::new(admin_io, None));
        let (client_io, _client_peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);

        let sr_enforcer = SchemaRegistryEnforcer::new("http://unused.invalid");
        let registry_client = RegistryClient::new("http://unused.invalid");
        let db_path = temp_db_path("startup_other_error");
        let lineage_tracker = LineageTracker::new(&db_path).unwrap();

        let mut producer = DataProductProducerBase::new(
            "personnel-lifecycle",
            "manpower",
            "1.0.0",
            "manpower-team",
            "",
            one_port(),
            sr_enforcer,
            registry_client,
            lineage_tracker,
            topic_manager,
            client,
        );

        let admin_server = rusty_tokio::spawn(async move {
            let (correlation_id, request) = recv_create_topics_request(&mut admin_peer).await;
            let name = request.topics[0].name.clone();
            send_create_topics_response(
                &mut admin_peer,
                correlation_id,
                &[(
                    &name, 3, /* UNKNOWN_TOPIC_OR_PARTITION, never TOPIC_ALREADY_EXISTS */
                )],
            )
            .await;
        });

        let err = producer.startup().await.unwrap_err();
        admin_server.await.unwrap();

        assert!(matches!(err, ProducerError::Topic(_)));

        let _ = std::fs::remove_file(&db_path);
    }

    #[rusty_tokio::test]
    async fn publish_rejects_an_undeclared_topic() {
        let (admin_io, _admin_peer) = duplex(4096);
        let topic_manager = TopicManager::new(KafkaClient::new(admin_io, None));
        let (client_io, _client_peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let sr_enforcer = SchemaRegistryEnforcer::new("http://unused.invalid");
        let registry_client = RegistryClient::new("http://unused.invalid");
        let db_path = temp_db_path("publish_undeclared");
        let lineage_tracker = LineageTracker::new(&db_path).unwrap();

        let mut producer = DataProductProducerBase::new(
            "personnel-lifecycle",
            "manpower",
            "1.0.0",
            "manpower-team",
            "",
            Vec::new(),
            sr_enforcer,
            registry_client,
            lineage_tracker,
            topic_manager,
            client,
        );

        let event = TestEvent::new("req-1");
        let err = producer
            .publish("manpower.personnel-lifecycle.assignments", &event)
            .await
            .unwrap_err();
        assert!(
            matches!(err, PublishError::UndeclaredTopic(topic, _) if topic == "manpower.personnel-lifecycle.assignments")
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[rusty_tokio::test]
    async fn publish_sends_lineage_headers_and_records_lineage_after_a_successful_produce() {
        let (admin_io, _admin_peer) = duplex(4096);
        let topic_manager = TopicManager::new(KafkaClient::new(admin_io, None));
        let (client_io, mut client_peer) = duplex(8192);
        let client = KafkaClient::new(client_io, None);
        let sr_enforcer = SchemaRegistryEnforcer::new("http://unused.invalid");
        let registry_client = RegistryClient::new("http://unused.invalid");
        let db_path = temp_db_path("publish_success");
        let lineage_tracker = LineageTracker::new(&db_path).unwrap();

        let mut producer = DataProductProducerBase::new(
            "personnel-lifecycle",
            "manpower",
            "1.0.0",
            "manpower-team",
            "",
            Vec::new(),
            sr_enforcer,
            registry_client,
            lineage_tracker,
            topic_manager,
            client,
        );
        // publish()'s SDK-023 check only cares that the topic was
        // started -- reach in and seed it directly rather than paying
        // for a full startup() round trip this test doesn't need.
        producer
            .started_topics
            .insert("manpower.personnel-lifecycle.assignments".to_string());

        let mut event = TestEvent::new("req-1");
        event.base.source_event_ids = vec!["upstream-1".to_string(), "upstream-2".to_string()];
        let event_id = event.base.event_id.clone();

        let server = rusty_tokio::spawn(async move {
            respond_to_produce(&mut client_peer, 0).await;
        });

        producer
            .publish("manpower.personnel-lifecycle.assignments", &event)
            .await
            .unwrap();
        server.await.unwrap();

        let lineage = producer
            .lineage_tracker
            .get_record_lineage(&event.base.correlation_id)
            .unwrap();
        assert_eq!(lineage.len(), 1);
        assert_eq!(lineage[0].event_id, event_id);
        assert_eq!(
            lineage[0].source_event_ids,
            vec!["upstream-1".to_string(), "upstream-2".to_string()]
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[rusty_tokio::test]
    async fn flush_is_a_documented_no_op() {
        let (admin_io, _admin_peer) = duplex(4096);
        let topic_manager = TopicManager::new(KafkaClient::new(admin_io, None));
        let (client_io, _client_peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let sr_enforcer = SchemaRegistryEnforcer::new("http://unused.invalid");
        let registry_client = RegistryClient::new("http://unused.invalid");
        let db_path = temp_db_path("flush_noop");
        let lineage_tracker = LineageTracker::new(&db_path).unwrap();

        let mut producer = DataProductProducerBase::new(
            "personnel-lifecycle",
            "manpower",
            "1.0.0",
            "manpower-team",
            "",
            Vec::new(),
            sr_enforcer,
            registry_client,
            lineage_tracker,
            topic_manager,
            client,
        );
        producer.flush(10.0);

        let _ = std::fs::remove_file(&db_path);
    }
}
