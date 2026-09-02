//! [`PersonnelLifecycleProducer`] -- the Rust port of
//! `meshed.domains.products.personnel_lifecycle.PersonnelLifecycleProducer`
//! (DOM-012..019).
//!
//! Unlike [`crate::products::PositionManagementProducer`]/
//! `ReadinessAssessmentProducer`, this producer overrides `publish()`
//! entirely: instead of producing to Kafka directly, it writes to the
//! transactional outbox (`rusty-meshed-sdk::outbox`) inside the same
//! atomic SQLite write the source uses, and relies on a dedicated
//! [`OutboxRelay`] to relay pending entries to Kafka in the background
//! (DOM-014). `startup()`/`shutdown()` wrap the relay's own
//! `start()`/`stop()` around the base producer's normal startup
//! (schema registration, product/port registration, topic creation --
//! all unchanged, DOM-018/019).
//!
//! # The outbox payload is JSON, not Avro -- faithfully reproduced
//!
//! `startup()` still registers this event type's *Avro* schema with
//! the Schema Registry (inherited unchanged from the base producer,
//! SDK-016). But the outbox write itself uses
//! [`rusty_meshed_core::DomainEvent::to_json`] -- matching the
//! source's own `write_outbox_entry(..., payload=event.model_dump())`
//! -- and `OutboxRelay`/`relay_pending` forward that payload to Kafka
//! verbatim. So a message actually produced through this path carries
//! JSON bytes, not Avro, even though the registered schema describes
//! Avro. This is a real mismatch already present in the Python
//! source, not something this port introduces or corrects -- see
//! [`rusty_meshed_core::DomainEvent::to_json`]'s own doc.

use crate::events::{PersonnelAssigned, PersonnelPromoted, PersonnelSeparated, StatusChanged};
use rusty_err::Error;
use rusty_meshed_core::{DomainEvent, EventType, PlatformConfig};
use rusty_meshed_sdk::{
    ensure_outbox_schema, write_outbox_entry, DataProductProducerBase, OutboxRelay, OutputPortSpec,
    PortDescriptor, ProducerError,
};
use rusty_sqlite::rusqlite::Connection;
use rusty_tokio::io::{AsyncRead, AsyncWrite, TcpStream};

/// Errors from [`PersonnelLifecycleProducer::connect`]/
/// [`PersonnelLifecycleProducer::startup`].
#[derive(Debug, Error)]
pub enum PersonnelLifecycleStartupError {
    /// The base producer's own `connect`/`startup` failed.
    #[error("{0}")]
    Producer(#[from] ProducerError),
    /// Opening or preparing the outbox database failed.
    #[error("outbox database error: {0}")]
    Outbox(String),
}

/// Errors from [`PersonnelLifecycleProducer::publish`].
#[derive(Debug, Error)]
pub enum PersonnelLifecyclePublishError {
    /// `topic` wasn't one of the topics `startup()` declared
    /// (matching `DataProductProducerBase::publish`'s own SDK-023
    /// check, reused via `is_declared_topic`).
    #[error("{0:?} is not a declared output port topic. Declared topics: {1:?}")]
    UndeclaredTopic(String, Vec<String>),
    /// The outbox write itself failed.
    #[error("outbox database error: {0}")]
    Outbox(String),
    /// Recording record-level lineage after the outbox write failed.
    #[error("lineage recording failed: {0}")]
    Lineage(String),
}

/// Data product producer for manpower personnel lifecycle events
/// (DOM-012..019). See the module doc for the outbox override. Generic
/// over `S`, the base producer's Kafka stream type -- `TcpStream` for
/// real use via [`connect`](Self::connect), any
/// `AsyncRead + AsyncWrite` for [`with_base`](PersonnelLifecycleProducer::with_base),
/// the seam this crate's own tests use.
pub struct PersonnelLifecycleProducer<S> {
    base: DataProductProducerBase<S>,
    outbox_db_path: String,
    outbox_relay: OutboxRelay,
}

impl<S> PersonnelLifecycleProducer<S> {
    // Metadata and the port declarations don't depend on `S` at all --
    // an unbounded `impl<S>` block, separate from the bounded one
    // below, so a caller who only needs these (e.g. this module's own
    // tests, building port descriptors before any producer exists)
    // never has to satisfy `AsyncRead + AsyncWrite` just to name them.
    // (Referencing them still needs a concrete `S` in the turbofish,
    // e.g. `PersonnelLifecycleProducer::<TcpStream>::output_ports()`
    // -- any valid `S` works, since none of these read `Self`.)
    pub const PRODUCT_NAME: &'static str = "personnel-lifecycle";
    pub const DOMAIN: &'static str = "manpower";
    pub const VERSION: &'static str = "1.0.0";
    pub const OWNER: &'static str = "manpower-team";
    pub const DESCRIPTION: &'static str =
        "Personnel lifecycle: assignments, promotions, separations, status changes";

    /// The four output ports (DOM-013), all `EventType::Delta`.
    pub fn output_ports() -> Vec<PortDescriptor> {
        vec![
            OutputPortSpec::<PersonnelAssigned>::new(
                "assignments",
                "manpower.personnel-lifecycle.assignments",
                EventType::Delta,
            )
            .describe(),
            OutputPortSpec::<PersonnelPromoted>::new(
                "promotions",
                "manpower.personnel-lifecycle.promotions",
                EventType::Delta,
            )
            .describe(),
            OutputPortSpec::<PersonnelSeparated>::new(
                "separations",
                "manpower.personnel-lifecycle.separations",
                EventType::Delta,
            )
            .describe(),
            OutputPortSpec::<StatusChanged>::new(
                "status-changes",
                "manpower.personnel-lifecycle.status-changes",
                EventType::Delta,
            )
            .describe(),
        ]
    }
}

impl PersonnelLifecycleProducer<TcpStream> {
    /// Connects the base producer from `config`, then opens (creating
    /// if absent) an outbox database at `db_path` -- defaulting to
    /// `config.registry_db_path` when `None`, the same plain-path
    /// translation of the source's `db_url or
    /// f"sqlite:///{config.registry_db_path}"` this crate family uses
    /// everywhere (DOM-017; see `rusty-meshed-sdk::outbox`'s own doc)
    /// -- and builds the [`OutboxRelay`] bound to that same path but a
    /// separate connection, matching the source's own cross-thread
    /// SQLite avoidance.
    pub async fn connect(
        config: &PlatformConfig,
        db_path: Option<&str>,
    ) -> Result<Self, PersonnelLifecycleStartupError> {
        let base = DataProductProducerBase::connect(
            Self::PRODUCT_NAME,
            Self::DOMAIN,
            Self::VERSION,
            Self::OWNER,
            Self::DESCRIPTION,
            Self::output_ports(),
            config,
        )
        .await?;

        let outbox_db_path = db_path
            .map(str::to_string)
            .unwrap_or_else(|| config.registry_db_path.clone());
        {
            let conn = Connection::open(&outbox_db_path)
                .map_err(|err| PersonnelLifecycleStartupError::Outbox(err.to_string()))?;
            ensure_outbox_schema(&conn)
                .map_err(|err| PersonnelLifecycleStartupError::Outbox(err.to_string()))?;
        }
        let outbox_relay =
            OutboxRelay::new(outbox_db_path.clone(), &config.kafka_bootstrap_servers);

        Ok(PersonnelLifecycleProducer {
            base,
            outbox_db_path,
            outbox_relay,
        })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> PersonnelLifecycleProducer<S> {
    /// Wraps an already-connected base producer and outbox path --
    /// the seam this crate's own tests use.
    pub fn with_base(
        base: DataProductProducerBase<S>,
        outbox_db_path: impl Into<String>,
        bootstrap_servers: &str,
    ) -> Self {
        let outbox_db_path = outbox_db_path.into();
        let outbox_relay = OutboxRelay::new(outbox_db_path.clone(), bootstrap_servers);
        PersonnelLifecycleProducer {
            base,
            outbox_db_path,
            outbox_relay,
        }
    }

    /// Registers schemas/product/ports via the base producer, then
    /// starts the outbox relay (DOM-018).
    pub async fn startup(&mut self) -> Result<(), PersonnelLifecycleStartupError> {
        self.base.startup().await?;
        self.outbox_relay.start();
        Ok(())
    }

    /// Stops the outbox relay, waiting up to 5 seconds (DOM-019).
    pub fn shutdown(&mut self) {
        self.outbox_relay.stop();
    }

    /// Writes `event` to the transactional outbox instead of producing
    /// to Kafka directly (DOM-014/015), then records record-level
    /// lineage (DOM-016) -- regardless of whether the relay has
    /// actually relayed the entry to Kafka yet, matching the source
    /// (its own lineage call happens right after the outbox commit,
    /// not after a confirmed Kafka delivery -- there is none to wait
    /// for on this path).
    pub fn publish<E: DomainEvent>(
        &mut self,
        topic: &str,
        event: &E,
    ) -> Result<(), PersonnelLifecyclePublishError> {
        if !self.base.is_declared_topic(topic) {
            return Err(PersonnelLifecyclePublishError::UndeclaredTopic(
                topic.to_string(),
                self.base.declared_topics(),
            ));
        }

        let base_event = event.base();
        let mut headers = rusty_json::Map::new();
        headers.insert(
            "event_id".to_string(),
            rusty_json::Value::from(base_event.event_id.as_str()),
        );
        headers.insert(
            "correlation_id".to_string(),
            rusty_json::Value::from(base_event.correlation_id.as_str()),
        );
        headers.insert(
            "source_event_ids".to_string(),
            rusty_json::Value::from(base_event.source_event_ids.join(",").as_str()),
        );
        headers.insert(
            "timestamp".to_string(),
            rusty_json::Value::from(base_event.timestamp.as_str()),
        );
        let headers = rusty_json::Value::Object(headers);
        let payload = event.to_json();

        let conn = Connection::open(&self.outbox_db_path)
            .map_err(|err| PersonnelLifecyclePublishError::Outbox(err.to_string()))?;
        write_outbox_entry(&conn, E::EVENT_NAME, topic, &payload, Some(&headers))
            .map_err(|err| PersonnelLifecyclePublishError::Outbox(err.to_string()))?;

        self.base
            .lineage_tracker()
            .record_event(
                &base_event.event_id,
                &base_event.correlation_id,
                &base_event.source_event_ids,
                self.base.product_name(),
                topic,
                &base_event.timestamp,
            )
            .map_err(|err| PersonnelLifecyclePublishError::Lineage(err.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_http::async_tokio::AsyncTransport;
    use rusty_http::head::ResponseHead;
    use rusty_http::{HeaderMap, StatusCode, Version};
    use rusty_kafka::protocol::create_topics::{
        CreatableTopicResult, CreateTopicsRequest, CreateTopicsResponse,
    };
    use rusty_kafka::testing::{recv_request, send_response};
    use rusty_kafka::KafkaClient;
    use rusty_meshed_observability::LineageTracker;
    use rusty_meshed_schema_registry::SchemaRegistryEnforcer;
    use rusty_meshed_sdk::outbox::fetch_all;
    use rusty_meshed_sdk::registry_client::RegistryClient;
    use rusty_meshed_sdk::TopicManager;
    use rusty_tokio::io::duplex;
    use rusty_wire::{Reader, Writer};

    fn temp_db_path(name: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "rusty_meshed_personnel_lifecycle_test_{name}_{}.db",
            rusty_uuid::Uuid::new_v4()
        ));
        path.to_str().unwrap().to_string()
    }

    /// A local copy of the fake-HTTP-server helper
    /// `rusty-meshed-sdk::producer`'s own tests use, trimmed to skip
    /// capturing request details -- this module's tests don't assert
    /// on them, unlike `producer`'s own -- see that module's test doc
    /// comment for why it's duplicated rather than shared.
    fn start_fake_http_server(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, rusty_tokio::JoinHandle<()>) {
        let listener = rusty_tokio::io::TcpListener::bind("127.0.0.1:0".parse().unwrap())
            .expect("failed to bind");
        let addr = listener.local_addr().expect("failed to read local_addr");
        let url = format!("http://{addr}");

        let handle = rusty_tokio::spawn(async move {
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
            }
        });

        (url, handle)
    }

    type Dup = rusty_tokio::io::DuplexStream;

    fn unstarted_producer(db_path: &str) -> PersonnelLifecycleProducer<Dup> {
        let (admin_io, _admin_peer) = duplex(4096);
        let topic_manager = TopicManager::new(KafkaClient::new(admin_io, None));
        let (client_io, _client_peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let sr_enforcer = SchemaRegistryEnforcer::new("http://unused.invalid");
        let registry_client = RegistryClient::new("http://unused.invalid");
        let lineage_db = temp_db_path("lineage");
        let lineage_tracker = LineageTracker::new(&lineage_db).unwrap();

        let base = DataProductProducerBase::new(
            PersonnelLifecycleProducer::<Dup>::PRODUCT_NAME,
            PersonnelLifecycleProducer::<Dup>::DOMAIN,
            PersonnelLifecycleProducer::<Dup>::VERSION,
            PersonnelLifecycleProducer::<Dup>::OWNER,
            PersonnelLifecycleProducer::<Dup>::DESCRIPTION,
            Vec::new(),
            sr_enforcer,
            registry_client,
            lineage_tracker,
            topic_manager,
            client,
        );
        PersonnelLifecycleProducer::with_base(base, db_path, "unused:9092")
    }

    #[test]
    fn output_ports_declares_four_delta_ports() {
        let ports = PersonnelLifecycleProducer::<Dup>::output_ports();
        assert_eq!(ports.len(), 4);
        let names: Vec<&str> = ports.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["assignments", "promotions", "separations", "status-changes"]
        );
    }

    #[test]
    fn publish_rejects_an_undeclared_topic() {
        let db_path = temp_db_path("undeclared");
        let mut producer = unstarted_producer(&db_path);
        let event = PersonnelAssigned::new(
            "req-1",
            "p-1",
            "pos-1",
            "UIC-1",
            "Rifleman",
            "E4",
            "2026-01-01",
            "2026-01-02",
        );
        let err = producer
            .publish("manpower.personnel-lifecycle.assignments", &event)
            .unwrap_err();
        assert!(matches!(
            err,
            PersonnelLifecyclePublishError::UndeclaredTopic(topic, _)
                if topic == "manpower.personnel-lifecycle.assignments"
        ));
        let _ = std::fs::remove_file(&db_path);
    }

    #[rusty_tokio::test]
    async fn startup_then_publish_writes_a_json_outbox_entry_and_records_lineage() {
        let ports = vec![OutputPortSpec::<PersonnelAssigned>::new(
            "assignments",
            "manpower.personnel-lifecycle.assignments",
            EventType::Delta,
        )
        .describe()];

        let (admin_io, mut admin_peer) = duplex(8192);
        let topic_manager = TopicManager::new(KafkaClient::new(admin_io, None));
        let (client_io, _client_peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let (sr_url, sr_server) = start_fake_http_server(vec![(200, r#"{"id": 1}"#)]);
        let sr_enforcer =
            SchemaRegistryEnforcer::with_client(&sr_url, rusty_request::Client::new());
        let (reg_url, reg_server) =
            start_fake_http_server(vec![(200, r#"{"id": 42}"#), (200, r#"{"id": 100}"#)]);
        let registry_client = RegistryClient::new(&reg_url);
        let lineage_db = temp_db_path("lineage_ok");
        let lineage_tracker = LineageTracker::new(&lineage_db).unwrap();
        let base = DataProductProducerBase::new(
            PersonnelLifecycleProducer::<Dup>::PRODUCT_NAME,
            PersonnelLifecycleProducer::<Dup>::DOMAIN,
            PersonnelLifecycleProducer::<Dup>::VERSION,
            PersonnelLifecycleProducer::<Dup>::OWNER,
            PersonnelLifecycleProducer::<Dup>::DESCRIPTION,
            ports,
            sr_enforcer,
            registry_client,
            lineage_tracker,
            topic_manager,
            client,
        );
        let outbox_db = temp_db_path("outbox_ok");
        let mut producer = PersonnelLifecycleProducer::with_base(base, &outbox_db, "unused:9092");

        let admin_server = rusty_tokio::spawn(async move {
            let (header, body) = recv_request(&mut admin_peer).await.unwrap();
            let mut reader = Reader::new(&body);
            let request = CreateTopicsRequest::decode(&mut reader).unwrap();
            let name = request.topics[0].name.clone();
            let response = CreateTopicsResponse {
                topics: vec![CreatableTopicResult {
                    name,
                    error_code: 0,
                }],
            };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut admin_peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();
        });

        // `with_base` is the raw DI seam (mirrors
        // `DataProductProducerBase::new`) -- it doesn't touch the
        // outbox schema, matching `connect()`'s own doc that schema
        // setup happens there for real callers; this test does it
        // explicitly instead.
        {
            let conn = Connection::open(&outbox_db).unwrap();
            ensure_outbox_schema(&conn).unwrap();
        }

        producer.startup().await.unwrap();
        admin_server.await.unwrap();
        sr_server.await.unwrap();
        reg_server.await.unwrap();
        producer.shutdown();

        let event = PersonnelAssigned::new(
            "req-1",
            "p-1",
            "pos-1",
            "UIC-1",
            "Rifleman",
            "E4",
            "2026-01-01",
            "2026-01-02",
        );
        producer
            .publish("manpower.personnel-lifecycle.assignments", &event)
            .unwrap();

        let conn = Connection::open(&outbox_db).unwrap();
        let entries = fetch_all(&conn).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type, "PersonnelAssigned");
        let payload: rusty_json::Value = rusty_json::from_str(&entries[0].payload).unwrap();
        assert_eq!(payload.get("person_id").unwrap().as_str(), Some("p-1"));

        let lineage = producer
            .base
            .lineage_tracker()
            .get_record_lineage("req-1")
            .unwrap();
        assert_eq!(lineage.len(), 1);

        let _ = std::fs::remove_file(&outbox_db);
        let _ = std::fs::remove_file(&lineage_db);
    }
}
