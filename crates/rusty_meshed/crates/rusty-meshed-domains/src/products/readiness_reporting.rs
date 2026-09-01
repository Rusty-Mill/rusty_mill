//! [`ReadinessAssessmentProducer`], [`PersonnelAssignmentConsumer`],
//! [`PositionFillConsumer`], and [`ReadinessReportingProduct`] -- the
//! Rust port of `meshed.domains.products.readiness_reporting`
//! (DOM-022..025).
//!
//! `correlation_id` is always propagated from the triggering event,
//! never freshly generated -- both consumers' [`process`] set it
//! explicitly after construction (`UnitReadinessAssessed::new`, like
//! every domain event's constructor, only ever generates a *fresh*
//! `correlation_id` via `BaseEvent::new`, matching source parity for
//! every other caller of that constructor -- DOM-023/024's propagation
//! is a property of these two `process()` methods, not of the event
//! type itself).
//!
//! # What's built, and what's still blocked
//!
//! [`process`] is fully built and tested: it needs only
//! `DataProductProducerBase::publish`, not `Fetch`. There is no poll
//! loop to drive it from yet, so nothing here calls it automatically
//! -- see `rusty-meshed-sdk::consumer`'s own module doc for the
//! blocker. [`ReadinessReportingProduct::prepare`] builds and runs the
//! *sequential* half of the source's `startup()` (producer schema/
//! product/port registration, then both consumers' output-port
//! resolution and contract validation) since none of that needs
//! `Fetch` either; `run()`'s concurrent `asyncio.gather`-driven polling
//! (DOM-025) is not built, for the same reason `DataProductConsumerBase`
//! has no `run()` yet.
//!
//! [`process`]: PersonnelAssignmentConsumer::process

use crate::events::{PersonnelAssigned, PositionFilled, UnitReadinessAssessed};
use rusty_err::Error;
use rusty_meshed_core::EventType;
use rusty_meshed_core::PlatformConfig;
use rusty_meshed_sdk::{
    ConsumerStartupError, DataProductConsumerBase, DataProductProducerBase, OutputPortSpec,
    PortDescriptor, ProducerError, PublishError,
};
use rusty_tokio::io::{AsyncRead, AsyncWrite, TcpStream};
use rusty_tokio::sync::Mutex;
use std::sync::Arc;

/// The derived measurement's fixed readiness percentage (DOM-023/024:
/// hardcoded `0.75` in the source -- a v1 placeholder, not a real
/// computation).
const DERIVED_READINESS_PCT: f64 = 0.75;

/// Data product producer for unit readiness assessment measurements
/// (DOM-022).
pub struct ReadinessAssessmentProducer;

impl ReadinessAssessmentProducer {
    pub const PRODUCT_NAME: &'static str = "readiness-reporting";
    pub const DOMAIN: &'static str = "manpower";
    pub const VERSION: &'static str = "1.0.0";
    pub const OWNER: &'static str = "manpower-team";
    pub const DESCRIPTION: &'static str =
        "Unit readiness assessments derived from personnel and position events";

    /// One `Measurement`-classified output port (DOM-022).
    pub fn output_ports() -> Vec<PortDescriptor> {
        vec![OutputPortSpec::<UnitReadinessAssessed>::new(
            "assessments",
            "manpower.readiness-reporting.assessments",
            EventType::Measurement,
        )
        .describe()]
    }
}

/// Errors from [`ReadinessReportingProduct::connect`]/
/// [`ReadinessReportingProduct::prepare`].
#[derive(Debug, Error)]
pub enum ReadinessReportingError {
    #[error("{0}")]
    Producer(#[from] ProducerError),
    #[error("{0}")]
    Consumer(#[from] ConsumerStartupError),
}

/// Consumer for personnel assignment events that emits readiness
/// assessments (DOM-023). Generic over `S`, the shared producer's
/// Kafka stream type -- same reasoning as
/// [`crate::products::PersonnelLifecycleProducer`]'s own module doc.
pub struct PersonnelAssignmentConsumer<S> {
    base: DataProductConsumerBase<PersonnelAssigned, S>,
    producer: Arc<Mutex<DataProductProducerBase<S>>>,
}

impl<S> PersonnelAssignmentConsumer<S> {
    pub const PRODUCT_NAME: &'static str = "personnel-lifecycle";
    pub const PORT_NAME: &'static str = "assignments";
    pub const GROUP_ID: &'static str = "readiness-reporting-personnel-consumer";
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> PersonnelAssignmentConsumer<S> {
    /// Wraps an already-connected consumer base and the shared
    /// [`ReadinessAssessmentProducer`] this consumer's derived
    /// assessments publish through.
    pub fn new(
        base: DataProductConsumerBase<PersonnelAssigned, S>,
        producer: Arc<Mutex<DataProductProducerBase<S>>>,
    ) -> Self {
        PersonnelAssignmentConsumer { base, producer }
    }

    /// Read-only access to this consumer's base, for
    /// [`ReadinessReportingProduct::prepare`]'s output-port resolution
    /// step.
    pub fn base(&self) -> &DataProductConsumerBase<PersonnelAssigned, S> {
        &self.base
    }

    /// Derives a [`UnitReadinessAssessed`] measurement from `event` and
    /// publishes it via the shared producer (DOM-023) -- the testable
    /// core of the source's `process()`, not yet drivable by a real
    /// poll loop (see the module doc).
    pub async fn process(&self, event: &PersonnelAssigned) -> Result<(), PublishError> {
        let assessment = derive_assessment(
            &event.base.correlation_id,
            &event.base.event_id,
            &event.unit_uic,
            &event.effective_date,
            &event.transaction_date,
        );
        let mut producer = self.producer.lock().await;
        producer
            .publish("manpower.readiness-reporting.assessments", &assessment)
            .await?;
        producer.flush(10.0);
        Ok(())
    }
}

/// Consumer for position fill events that emits readiness assessments
/// (DOM-024).
pub struct PositionFillConsumer<S> {
    base: DataProductConsumerBase<PositionFilled, S>,
    producer: Arc<Mutex<DataProductProducerBase<S>>>,
}

impl<S> PositionFillConsumer<S> {
    pub const PRODUCT_NAME: &'static str = "position-management";
    pub const PORT_NAME: &'static str = "fills";
    pub const GROUP_ID: &'static str = "readiness-reporting-position-consumer";
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> PositionFillConsumer<S> {
    pub fn new(
        base: DataProductConsumerBase<PositionFilled, S>,
        producer: Arc<Mutex<DataProductProducerBase<S>>>,
    ) -> Self {
        PositionFillConsumer { base, producer }
    }

    pub fn base(&self) -> &DataProductConsumerBase<PositionFilled, S> {
        &self.base
    }

    /// Derives a [`UnitReadinessAssessed`] measurement from `event` and
    /// publishes it via the shared producer (DOM-024).
    pub async fn process(&self, event: &PositionFilled) -> Result<(), PublishError> {
        let assessment = derive_assessment(
            &event.base.correlation_id,
            &event.base.event_id,
            &event.unit_uic,
            &event.effective_date,
            &event.transaction_date,
        );
        let mut producer = self.producer.lock().await;
        producer
            .publish("manpower.readiness-reporting.assessments", &assessment)
            .await?;
        producer.flush(10.0);
        Ok(())
    }
}

/// Shared derivation logic for both consumers' `process()`: same
/// `correlation_id` as the triggering event (never a fresh one),
/// `source_event_ids = [event.event_id]`, a hardcoded
/// [`DERIVED_READINESS_PCT`], `assessed_at` set to now.
fn derive_assessment(
    correlation_id: &str,
    source_event_id: &str,
    unit_uic: &str,
    effective_date: &str,
    transaction_date: &str,
) -> UnitReadinessAssessed {
    let mut assessment = UnitReadinessAssessed::new(
        correlation_id.to_string(),
        unit_uic.to_string(),
        DERIVED_READINESS_PCT,
        now_iso(),
        effective_date.to_string(),
        transaction_date.to_string(),
    );
    assessment.base.source_event_ids = vec![source_event_id.to_string()];
    assessment
}

/// A minimal RFC 3339 UTC "now" formatter -- same hand-rolled
/// civil-from-days algorithm duplicated elsewhere in this crate family.
fn now_iso() -> String {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = since_epoch.as_secs();
    let mut days = (total_secs / 86_400) as i64;
    let secs_of_day = total_secs % 86_400;
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );

    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = (days - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Composition wrapper combining two consumers and one producer for
/// readiness reporting (DOM-025). Generic over `S`, same reasoning as
/// [`PersonnelAssignmentConsumer`]/[`PositionFillConsumer`].
pub struct ReadinessReportingProduct<S> {
    producer: Arc<Mutex<DataProductProducerBase<S>>>,
    personnel_consumer: PersonnelAssignmentConsumer<S>,
    position_consumer: PositionFillConsumer<S>,
}

impl ReadinessReportingProduct<TcpStream> {
    /// Connects the shared producer and both consumers from `config`.
    pub async fn connect(config: &PlatformConfig) -> Result<Self, ReadinessReportingError> {
        let producer = DataProductProducerBase::connect(
            ReadinessAssessmentProducer::PRODUCT_NAME,
            ReadinessAssessmentProducer::DOMAIN,
            ReadinessAssessmentProducer::VERSION,
            ReadinessAssessmentProducer::OWNER,
            ReadinessAssessmentProducer::DESCRIPTION,
            ReadinessAssessmentProducer::output_ports(),
            config,
        )
        .await?;
        let producer = Arc::new(Mutex::new(producer));

        let personnel_base = DataProductConsumerBase::<PersonnelAssigned>::connect(
            PersonnelAssignmentConsumer::<TcpStream>::PRODUCT_NAME,
            PersonnelAssignmentConsumer::<TcpStream>::PORT_NAME,
            PersonnelAssignmentConsumer::<TcpStream>::GROUP_ID,
            config,
        )
        .await?;
        let position_base = DataProductConsumerBase::<PositionFilled>::connect(
            PositionFillConsumer::<TcpStream>::PRODUCT_NAME,
            PositionFillConsumer::<TcpStream>::PORT_NAME,
            PositionFillConsumer::<TcpStream>::GROUP_ID,
            config,
        )
        .await?;

        Ok(ReadinessReportingProduct {
            personnel_consumer: PersonnelAssignmentConsumer::new(personnel_base, producer.clone()),
            position_consumer: PositionFillConsumer::new(position_base, producer.clone()),
            producer,
        })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> ReadinessReportingProduct<S> {
    /// Wraps already-connected components -- the seam this crate's own
    /// tests use.
    pub fn new(
        producer: Arc<Mutex<DataProductProducerBase<S>>>,
        personnel_consumer: PersonnelAssignmentConsumer<S>,
        position_consumer: PositionFillConsumer<S>,
    ) -> Self {
        ReadinessReportingProduct {
            producer,
            personnel_consumer,
            position_consumer,
        }
    }

    /// The sequential half of the source's `startup()` (DOM-025):
    /// registers the producer's schemas/product/ports, then resolves
    /// and validates each consumer's output port. Returns the two
    /// resolved topics `(personnel, position)` -- what a `Fetch`-
    /// capable `run()` would go on to subscribe to (not built, see the
    /// module doc).
    pub async fn prepare(&mut self) -> Result<(String, String), ReadinessReportingError> {
        self.producer.lock().await.startup().await?;
        let personnel_topic = self.personnel_consumer.base().resolve_output_port().await?;
        let position_topic = self.position_consumer.base().resolve_output_port().await?;
        Ok((personnel_topic, position_topic))
    }

    /// Read-only access to the shared producer, for tests and callers
    /// driving `process()` directly.
    pub fn producer(&self) -> &Arc<Mutex<DataProductProducerBase<S>>> {
        &self.producer
    }

    /// Read-only access to the personnel-assignment consumer.
    pub fn personnel_consumer(&self) -> &PersonnelAssignmentConsumer<S> {
        &self.personnel_consumer
    }

    /// Read-only access to the position-fill consumer.
    pub fn position_consumer(&self) -> &PositionFillConsumer<S> {
        &self.position_consumer
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
    use rusty_kafka::protocol::produce::{
        ProducePartitionResponse, ProduceResponse, ProduceTopicResponse,
    };
    use rusty_kafka::testing::{recv_request, send_response};
    use rusty_kafka::KafkaClient;
    use rusty_meshed_observability::LineageTracker;
    use rusty_meshed_schema_registry::SchemaRegistryEnforcer;
    use rusty_meshed_sdk::registry_client::RegistryClient;
    use rusty_meshed_sdk::TopicManager;
    use rusty_tokio::io::duplex;
    use rusty_wire::{Reader, Writer};

    fn temp_db_path(name: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "rusty_meshed_readiness_test_{name}_{}.db",
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

    /// Builds a real `ReadinessAssessmentProducer`-shaped
    /// `DataProductProducerBase` and drives its `startup()` to
    /// completion against fake servers (a fake broker for
    /// `CreateTopics`, fake HTTP servers for the Schema Registry and
    /// Data Product Registry), so its one output port is genuinely
    /// declared -- exactly what `process()`'s `publish()` call needs.
    async fn started_producer() -> (
        Arc<Mutex<DataProductProducerBase<rusty_tokio::io::DuplexStream>>>,
        rusty_tokio::io::DuplexStream,
    ) {
        let (admin_io, mut admin_peer) = duplex(4096);
        let topic_manager = TopicManager::new(KafkaClient::new(admin_io, None));
        let (client_io, client_peer) = duplex(8192);
        let client = KafkaClient::new(client_io, None);
        let (sr_url, sr_server) = start_fake_http_server(vec![(200, r#"{"id": 1}"#)]);
        let sr_enforcer =
            SchemaRegistryEnforcer::with_client(&sr_url, rusty_request::Client::new());
        let (reg_url, reg_server) =
            start_fake_http_server(vec![(200, r#"{"id": 7}"#), (200, r#"{"id": 70}"#)]);
        let registry_client = RegistryClient::new(&reg_url);
        let db_path = temp_db_path("producer_lineage");
        let lineage_tracker = LineageTracker::new(&db_path).unwrap();
        let mut producer = DataProductProducerBase::new(
            ReadinessAssessmentProducer::PRODUCT_NAME,
            ReadinessAssessmentProducer::DOMAIN,
            ReadinessAssessmentProducer::VERSION,
            ReadinessAssessmentProducer::OWNER,
            ReadinessAssessmentProducer::DESCRIPTION,
            ReadinessAssessmentProducer::output_ports(),
            sr_enforcer,
            registry_client,
            lineage_tracker,
            topic_manager,
            client,
        );

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

        producer.startup().await.unwrap();
        admin_server.await.unwrap();
        sr_server.await.unwrap();
        reg_server.await.unwrap();

        (Arc::new(Mutex::new(producer)), client_peer)
    }

    async fn respond_to_produce(peer: &mut rusty_tokio::io::DuplexStream, error_code: i16) {
        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, rusty_kafka::protocol::api_key::PRODUCE);
        let response = ProduceResponse {
            topics: vec![ProduceTopicResponse {
                name: "manpower.readiness-reporting.assessments".to_string(),
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

    #[test]
    fn derive_assessment_propagates_correlation_id_and_links_source_event_id() {
        let assessment = derive_assessment(
            "req-1",
            "evt-personnel-1",
            "UIC-1",
            "2026-01-01",
            "2026-01-02",
        );
        assert_eq!(assessment.base.correlation_id, "req-1");
        assert_eq!(assessment.base.source_event_ids, vec!["evt-personnel-1"]);
        assert_eq!(assessment.readiness_pct, DERIVED_READINESS_PCT);
        assert_eq!(assessment.unit_uic, "UIC-1");
        assert_eq!(assessment.effective_date, "2026-01-01");
        assert_eq!(assessment.transaction_date, "2026-01-02");
    }

    #[test]
    fn two_derivations_never_share_a_fresh_event_id() {
        let a = derive_assessment("req-1", "evt-1", "UIC-1", "2026-01-01", "2026-01-02");
        let b = derive_assessment("req-1", "evt-1", "UIC-1", "2026-01-01", "2026-01-02");
        assert_ne!(a.base.event_id, b.base.event_id);
    }

    type Dup = rusty_tokio::io::DuplexStream;

    #[rusty_tokio::test]
    async fn personnel_assignment_consumer_process_publishes_the_derived_assessment() {
        let (producer, mut peer) = started_producer().await;
        let consumer_db = temp_db_path("personnel_consumer");
        let (consumer_client_io, _consumer_client_peer) = duplex(4096);
        let consumer_base = DataProductConsumerBase::<PersonnelAssigned, Dup>::new(
            PersonnelAssignmentConsumer::<Dup>::PRODUCT_NAME,
            PersonnelAssignmentConsumer::<Dup>::PORT_NAME,
            PersonnelAssignmentConsumer::<Dup>::GROUP_ID,
            RegistryClient::new("http://unused.invalid"),
            LineageTracker::new(&consumer_db).unwrap(),
            KafkaClient::new(consumer_client_io, None),
        );
        let consumer = PersonnelAssignmentConsumer::new(consumer_base, producer);

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

        let server = rusty_tokio::spawn(async move {
            respond_to_produce(&mut peer, 0).await;
        });

        consumer.process(&event).await.unwrap();
        server.await.unwrap();
    }

    #[rusty_tokio::test]
    async fn position_fill_consumer_process_publishes_the_derived_assessment() {
        let (producer, mut peer) = started_producer().await;
        let consumer_db = temp_db_path("position_consumer");
        let (consumer_client_io, _consumer_client_peer) = duplex(4096);
        let consumer_base = DataProductConsumerBase::<PositionFilled, Dup>::new(
            PositionFillConsumer::<Dup>::PRODUCT_NAME,
            PositionFillConsumer::<Dup>::PORT_NAME,
            PositionFillConsumer::<Dup>::GROUP_ID,
            RegistryClient::new("http://unused.invalid"),
            LineageTracker::new(&consumer_db).unwrap(),
            KafkaClient::new(consumer_client_io, None),
        );
        let consumer = PositionFillConsumer::new(consumer_base, producer);

        let event =
            PositionFilled::new("req-2", "pos-1", "p-1", "UIC-1", "2026-01-01", "2026-01-02");

        let server = rusty_tokio::spawn(async move {
            respond_to_produce(&mut peer, 0).await;
        });

        consumer.process(&event).await.unwrap();
        server.await.unwrap();
    }
}
