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
//! # `run()`: `rusty_tokio::try_join!`, not `rusty_tokio::spawn`
//!
//! The source's own module doc explains why `run()` uses
//! `asyncio.gather` rather than two sequential `await`s: sequential
//! awaiting would block the second consumer until the first stopped,
//! so only one consumer would ever be actively polling. This port
//! reaches for [`rusty_tokio::try_join!`] rather than
//! [`rusty_tokio::spawn`]-ing two background tasks for the same reason
//! `DataProductConsumerBase::run` (see that module's own doc) doesn't
//! spawn its own poll loop internally: `try_join!` polls both
//! `PersonnelAssignmentConsumer::run`/`PositionFillConsumer::run`
//! futures within [`ReadinessReportingProduct::run`]'s own task,
//! genuinely concurrently (neither one blocks the other from making
//! progress), without needing `'static` bounds or a runtime handle --
//! `spawn` would need both, since [`ReadinessReportingProduct::run`]
//! borrows `&mut self`. `try_join!`, not the plain `join!` that waits
//! for both regardless of outcome, mirrors `asyncio.gather`'s own
//! default behavior of returning as soon as either task raises.
//!
//! [`process`]: PersonnelAssignmentConsumer::process

use crate::events::{PersonnelAssigned, PositionFilled, UnitReadinessAssessed};
use rusty_err::Error;
use rusty_meshed_core::EventType;
use rusty_meshed_core::PlatformConfig;
use rusty_meshed_sdk::{
    ConsumerRunError, ConsumerStartupError, ConsumerStopHandle, DataProductConsumerBase,
    DataProductProducerBase, OutputPortSpec, PortDescriptor, ProducerError, PublishError,
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
/// [`ReadinessReportingProduct::startup`]/[`ReadinessReportingProduct::run`].
#[derive(Debug, Error)]
pub enum ReadinessReportingError {
    #[error("{0}")]
    Producer(#[from] ProducerError),
    #[error("{0}")]
    Consumer(#[from] ConsumerStartupError),
    #[error("{0}")]
    Run(#[from] ConsumerRunError),
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

    /// Read-only access to this consumer's base -- e.g. for
    /// `resolve_output_port` outside of a full [`startup`](Self::startup)
    /// call.
    pub fn base(&self) -> &DataProductConsumerBase<PersonnelAssigned, S> {
        &self.base
    }

    /// Derives a [`UnitReadinessAssessed`] measurement from `event` and
    /// publishes it via the shared producer (DOM-023) -- the testable
    /// core of the source's `process()`. [`run`](Self::run) duplicates
    /// this logic in its own closure rather than calling this method
    /// directly: `DataProductConsumerBase::run` already holds `&mut
    /// self.base` for the poll loop's duration, and a closure calling
    /// `self.process(...)` would need `&self` (the whole receiver, not
    /// just the disjoint `self.producer` field a method call can't
    /// project out of `self`) -- a genuine borrow conflict, not just an
    /// ergonomics choice.
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

    /// Resolves this consumer's topic, joins its group, and resolves
    /// starting offsets (SDK-034..036, via
    /// [`DataProductConsumerBase::startup`]).
    pub async fn startup(&mut self) -> Result<(), ConsumerStartupError> {
        self.base.startup().await
    }

    /// Runs this consumer's fetch/dedup/process/commit poll loop
    /// (SDK-039..042), deriving and publishing a readiness assessment
    /// for every [`PersonnelAssigned`] event via [`process`](Self::process),
    /// until this consumer's [`stop_handle`](Self::stop_handle) is
    /// signaled.
    pub async fn run(&mut self) -> Result<(), ConsumerRunError> {
        let producer = self.producer.clone();
        self.base
            .run(move |event: PersonnelAssigned| {
                let producer = producer.clone();
                async move {
                    let assessment = derive_assessment(
                        &event.base.correlation_id,
                        &event.base.event_id,
                        &event.unit_uic,
                        &event.effective_date,
                        &event.transaction_date,
                    );
                    let mut producer = producer.lock().await;
                    producer
                        .publish("manpower.readiness-reporting.assessments", &assessment)
                        .await
                        .map_err(|err| err.to_string())?;
                    producer.flush(10.0);
                    Ok(())
                }
            })
            .await
    }

    /// A cloneable, thread-safe handle to stop a future
    /// [`run`](Self::run) call -- see
    /// `DataProductConsumerBase::stop_handle`'s own doc for why this
    /// exists instead of a same-`self` `stop()` method.
    pub fn stop_handle(&self) -> ConsumerStopHandle {
        self.base.stop_handle()
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
    /// publishes it via the shared producer (DOM-024). See
    /// [`PersonnelAssignmentConsumer::process`]'s own doc for why
    /// [`run`](Self::run) duplicates this logic rather than calling
    /// this method directly.
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

    /// Resolves this consumer's topic, joins its group, and resolves
    /// starting offsets (SDK-034..036, via
    /// [`DataProductConsumerBase::startup`]).
    pub async fn startup(&mut self) -> Result<(), ConsumerStartupError> {
        self.base.startup().await
    }

    /// Runs this consumer's fetch/dedup/process/commit poll loop
    /// (SDK-039..042), deriving and publishing a readiness assessment
    /// for every [`PositionFilled`] event via [`process`](Self::process),
    /// until this consumer's [`stop_handle`](Self::stop_handle) is
    /// signaled.
    pub async fn run(&mut self) -> Result<(), ConsumerRunError> {
        let producer = self.producer.clone();
        self.base
            .run(move |event: PositionFilled| {
                let producer = producer.clone();
                async move {
                    let assessment = derive_assessment(
                        &event.base.correlation_id,
                        &event.base.event_id,
                        &event.unit_uic,
                        &event.effective_date,
                        &event.transaction_date,
                    );
                    let mut producer = producer.lock().await;
                    producer
                        .publish("manpower.readiness-reporting.assessments", &assessment)
                        .await
                        .map_err(|err| err.to_string())?;
                    producer.flush(10.0);
                    Ok(())
                }
            })
            .await
    }

    /// A cloneable, thread-safe handle to stop a future
    /// [`run`](Self::run) call -- see
    /// `DataProductConsumerBase::stop_handle`'s own doc for why this
    /// exists instead of a same-`self` `stop()` method.
    pub fn stop_handle(&self) -> ConsumerStopHandle {
        self.base.stop_handle()
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

    /// The source's `startup()` (DOM-025): registers the producer's
    /// schemas/product/ports, then joins both consumers' groups and
    /// resolves their starting offsets. Sequential -- `await`ed one
    /// after another, exactly matching the source; only [`run`](Self::run)
    /// needs concurrency (see the module doc).
    ///
    /// Named `startup`, not `prepare`, matching the source exactly --
    /// an earlier pass here used `prepare` while this method could
    /// only run the output-port-resolution half of the source's
    /// `startup()` (before `DataProductConsumerBase::startup` existed);
    /// now that it does the whole thing, so does this method, so it
    /// takes the source's own name back.
    pub async fn startup(&mut self) -> Result<(), ReadinessReportingError> {
        self.producer.lock().await.startup().await?;
        self.personnel_consumer.startup().await?;
        self.position_consumer.startup().await?;
        Ok(())
    }

    /// Runs both consumers' poll loops concurrently until both stop
    /// (DOM-025) -- see the module doc for why `try_join!`, not
    /// sequential `.await` or `rusty_tokio::spawn`.
    pub async fn run(&mut self) -> Result<(), ReadinessReportingError> {
        rusty_tokio::try_join!(self.personnel_consumer.run(), self.position_consumer.run())?;
        Ok(())
    }

    /// Stop handles for both consumers' [`run`](Self::run) loops --
    /// obtain before calling `run()` (same reasoning as
    /// `DataProductConsumerBase::stop_handle`'s own doc) and signal
    /// either or both to end it.
    pub fn stop_handles(&self) -> (ConsumerStopHandle, ConsumerStopHandle) {
        (
            self.personnel_consumer.stop_handle(),
            self.position_consumer.stop_handle(),
        )
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
    use rusty_kafka::protocol::api_key;
    use rusty_kafka::protocol::consumer_protocol::{encode_assignment, encode_subscription};
    use rusty_kafka::protocol::create_topics::{
        CreatableTopicResult, CreateTopicsRequest, CreateTopicsResponse,
    };
    use rusty_kafka::protocol::fetch::{FetchPartitionResponse, FetchResponse, FetchTopicResponse};
    use rusty_kafka::protocol::find_coordinator::FindCoordinatorResponse;
    use rusty_kafka::protocol::heartbeat::HeartbeatResponse;
    use rusty_kafka::protocol::join_group::{JoinGroupMember, JoinGroupResponse};
    use rusty_kafka::protocol::leave_group::LeaveGroupResponse;
    use rusty_kafka::protocol::list_offsets::{
        ListOffsetsPartitionResponse, ListOffsetsResponse, ListOffsetsTopicResponse,
    };
    use rusty_kafka::protocol::metadata::{MetadataResponse, PartitionMetadata, TopicMetadata};
    use rusty_kafka::protocol::offset_commit::{
        OffsetCommitPartitionResponse, OffsetCommitResponse, OffsetCommitTopicResponse,
    };
    use rusty_kafka::protocol::offset_fetch::{
        OffsetFetchPartitionResponse, OffsetFetchResponse, OffsetFetchTopicResponse,
        NO_COMMITTED_OFFSET,
    };
    use rusty_kafka::protocol::produce::{
        ProducePartitionResponse, ProduceResponse, ProduceTopicResponse,
    };
    use rusty_kafka::protocol::sync_group::SyncGroupResponse;
    use rusty_kafka::record_batch::Record;
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

    /// `MetadataResponse` has no `encode` (`rusty_kafka` is
    /// client-only, and `rusty_kafka::wire` is private) -- hand-encodes
    /// the v0 wire shape a fake broker needs to send back, symmetric
    /// with `MetadataResponse::decode`. A local copy of
    /// `rusty-meshed-sdk::consumer`'s own test-only helper of the same
    /// name -- see that module's test doc comment for why it's
    /// duplicated rather than shared.
    fn write_metadata_response(writer: &mut Writer, response: &MetadataResponse) {
        fn write_i16(writer: &mut Writer, v: i16) {
            writer.write_u16_be(v as u16);
        }
        fn write_i32(writer: &mut Writer, v: i32) {
            writer.write_u32_be(v as u32);
        }
        fn write_string(writer: &mut Writer, v: &str) {
            write_i16(writer, v.len() as i16);
            writer.write_bytes(v.as_bytes());
        }

        write_i32(writer, response.brokers.len() as i32);
        for broker in &response.brokers {
            write_i32(writer, broker.node_id);
            write_string(writer, &broker.host);
            write_i32(writer, broker.port);
        }
        write_i32(writer, response.topics.len() as i32);
        for topic in &response.topics {
            write_i16(writer, topic.error_code);
            write_string(writer, &topic.name);
            write_i32(writer, topic.partitions.len() as i32);
            for partition in &topic.partitions {
                write_i16(writer, partition.error_code);
                write_i32(writer, partition.partition_index);
                write_i32(writer, partition.leader_id);
                write_i32(writer, partition.replica_nodes.len() as i32);
                for node in &partition.replica_nodes {
                    write_i32(writer, *node);
                }
                write_i32(writer, partition.isr_nodes.len() as i32);
                for node in &partition.isr_nodes {
                    write_i32(writer, *node);
                }
            }
        }
    }

    /// Drives one full `DataProductConsumerBase::startup`
    /// group-coordination round trip as the sole (and therefore
    /// leader) group member, subscribed to a single-partition `topic`:
    /// `FindCoordinator` -> `JoinGroup` -> `Metadata` (1 partition) ->
    /// `SyncGroup` (that partition assigned back) -> `OffsetFetch`
    /// (nothing committed) -> `ListOffsets` (earliest = 0). Does not
    /// answer `resolve_output_port`'s Data Product Registry HTTP calls
    /// -- callers still need their own fake HTTP server for those (see
    /// `rusty-meshed-sdk::consumer`'s own test of the same shape for
    /// why the two are separate concerns).
    async fn respond_to_startup_single_partition(peer: &mut Dup, topic: &str) {
        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, api_key::FIND_COORDINATOR);
        let response = FindCoordinatorResponse {
            error_code: 0,
            node_id: 1,
            host: "localhost".to_string(),
            port: 9092,
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        send_response(peer, header.correlation_id, writer.as_slice())
            .await
            .unwrap();

        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, api_key::JOIN_GROUP);
        let response = JoinGroupResponse {
            error_code: 0,
            generation_id: 1,
            group_protocol: "range".to_string(),
            leader_id: "consumer-1".to_string(),
            member_id: "consumer-1".to_string(),
            members: vec![JoinGroupMember {
                member_id: "consumer-1".to_string(),
                metadata: encode_subscription(&[topic.to_string()]),
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        send_response(peer, header.correlation_id, writer.as_slice())
            .await
            .unwrap();

        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, api_key::METADATA);
        let response = MetadataResponse {
            brokers: vec![],
            topics: vec![TopicMetadata {
                error_code: 0,
                name: topic.to_string(),
                partitions: vec![PartitionMetadata {
                    error_code: 0,
                    partition_index: 0,
                    leader_id: 1,
                    replica_nodes: vec![1],
                    isr_nodes: vec![1],
                }],
            }],
        };
        let mut writer = Writer::new();
        write_metadata_response(&mut writer, &response);
        send_response(peer, header.correlation_id, writer.as_slice())
            .await
            .unwrap();

        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, api_key::SYNC_GROUP);
        let response = SyncGroupResponse {
            error_code: 0,
            assignment: encode_assignment(&[(topic.to_string(), vec![0])]),
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        send_response(peer, header.correlation_id, writer.as_slice())
            .await
            .unwrap();

        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, api_key::OFFSET_FETCH);
        let response = OffsetFetchResponse {
            topics: vec![OffsetFetchTopicResponse {
                name: topic.to_string(),
                partitions: vec![OffsetFetchPartitionResponse {
                    partition_index: 0,
                    committed_offset: NO_COMMITTED_OFFSET,
                    metadata: None,
                    error_code: 0,
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        send_response(peer, header.correlation_id, writer.as_slice())
            .await
            .unwrap();

        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, api_key::LIST_OFFSETS);
        let response = ListOffsetsResponse {
            topics: vec![ListOffsetsTopicResponse {
                name: topic.to_string(),
                partitions: vec![ListOffsetsPartitionResponse {
                    partition_index: 0,
                    error_code: 0,
                    timestamp: -1,
                    offset: 0,
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        send_response(peer, header.correlation_id, writer.as_slice())
            .await
            .unwrap();
    }

    /// An empty `FetchResponse` for `topic`'s single partition -- no
    /// records, so `DataProductConsumerBase::run` skips straight to
    /// its next loop check with no `process`/`OffsetCommit` call.
    fn empty_fetch_response(topic: &str) -> FetchResponse {
        FetchResponse {
            throttle_time_ms: 0,
            topics: vec![FetchTopicResponse {
                name: topic.to_string(),
                partitions: vec![FetchPartitionResponse {
                    partition_index: 0,
                    error_code: 0,
                    high_watermark: 0,
                    last_stable_offset: 0,
                    aborted_transactions: vec![],
                    records: vec![],
                }],
            }],
        }
    }

    /// Exercises [`PersonnelAssignmentConsumer::run`] end to end: a
    /// real `startup()` group-join round trip, then one fetched
    /// `PersonnelAssigned` record derives and publishes a readiness
    /// assessment through the shared producer (proving the closure
    /// `run()` builds -- not `process()` itself, see that method's own
    /// doc for why -- actually performs the derive-and-publish work),
    /// commits that record's offset, and stops cleanly via
    /// `stop_handle()`.
    #[rusty_tokio::test]
    async fn personnel_assignment_consumer_run_derives_publishes_and_commits_for_one_event() {
        let topic = "manpower.personnel-lifecycle.assignments";
        let (producer, mut producer_peer) = started_producer().await;

        let (reg_url, _reg_server) = start_fake_http_server(vec![
            (200, r#"[{"id": 1, "name": "personnel-lifecycle"}]"#),
            (
                200,
                r#"[{"id": 10, "data_product_id": 1, "description": "assignments", "topic_name": "manpower.personnel-lifecycle.assignments"}]"#,
            ),
            (404, "{}"),
        ]);

        let consumer_db = temp_db_path("run_personnel");
        let (consumer_client_io, mut consumer_peer) = duplex(8192);
        let consumer_base = DataProductConsumerBase::<PersonnelAssigned, Dup>::new(
            PersonnelAssignmentConsumer::<Dup>::PRODUCT_NAME,
            PersonnelAssignmentConsumer::<Dup>::PORT_NAME,
            PersonnelAssignmentConsumer::<Dup>::GROUP_ID,
            RegistryClient::new(&reg_url),
            LineageTracker::new(&consumer_db).unwrap(),
            KafkaClient::new(consumer_client_io, None),
        );
        let mut consumer = PersonnelAssignmentConsumer::new(consumer_base, producer);

        let startup_server = rusty_tokio::spawn(async move {
            respond_to_startup_single_partition(&mut consumer_peer, topic).await;
            consumer_peer
        });
        consumer.startup().await.unwrap();
        let mut consumer_peer = startup_server.await.unwrap();

        let stop_handle = consumer.stop_handle();
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
        let value = event.serialize();

        let poll_server = rusty_tokio::spawn(async move {
            let (header, _body) = recv_request(&mut consumer_peer).await.unwrap();
            assert_eq!(header.api_key, api_key::HEARTBEAT);
            let response = HeartbeatResponse { error_code: 0 };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut consumer_peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();

            let (header, _body) = recv_request(&mut consumer_peer).await.unwrap();
            assert_eq!(header.api_key, api_key::FETCH);
            let response = FetchResponse {
                throttle_time_ms: 0,
                topics: vec![FetchTopicResponse {
                    name: topic.to_string(),
                    partitions: vec![FetchPartitionResponse {
                        partition_index: 0,
                        error_code: 0,
                        high_watermark: 1,
                        last_stable_offset: 1,
                        aborted_transactions: vec![],
                        records: vec![Record {
                            key: None,
                            value: Some(value),
                            headers: vec![],
                        }],
                    }],
                }],
            };
            let mut writer = Writer::new();
            response.encode(&mut writer, 1_735_689_600_000);
            send_response(&mut consumer_peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();

            let (header, _body) = recv_request(&mut consumer_peer).await.unwrap();
            assert_eq!(header.api_key, api_key::OFFSET_COMMIT);
            let response = OffsetCommitResponse {
                topics: vec![OffsetCommitTopicResponse {
                    name: topic.to_string(),
                    partitions: vec![OffsetCommitPartitionResponse {
                        partition_index: 0,
                        error_code: 0,
                    }],
                }],
            };
            let mut writer = Writer::new();
            response.encode(&mut writer);

            // Stop before sending, not after -- this is a
            // multi-threaded runtime, so calling `stop()` once the
            // response is already sent races the client task's own
            // next `running` check (it could observe `true` and send
            // a second Heartbeat before this task's `stop()` call even
            // runs). Calling it first guarantees the client can only
            // see `running == false` once it reads this response.
            stop_handle.stop();
            send_response(&mut consumer_peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();

            let (header, _body) = recv_request(&mut consumer_peer).await.unwrap();
            assert_eq!(header.api_key, api_key::LEAVE_GROUP);
            let response = LeaveGroupResponse { error_code: 0 };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut consumer_peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();
        });

        let produce_server = rusty_tokio::spawn(async move {
            respond_to_produce(&mut producer_peer, 0).await;
        });

        consumer.run().await.unwrap();
        poll_server.await.unwrap();
        produce_server.await.unwrap();
    }

    /// Proves DOM-025's actual concurrency requirement -- the module
    /// doc's own reasoning for why `ReadinessReportingProduct::run`
    /// uses `try_join!` rather than sequential `.await`: the personnel
    /// consumer's fake broker deliberately stalls its `Fetch` response
    /// until it observes the position consumer's own `Fetch` round
    /// trip has *already* completed. If `run()` drove the two
    /// consumers sequentially instead of concurrently,
    /// `position_peer` would never even receive its `Heartbeat`
    /// request -- `personnel_consumer.run()` wouldn't return (and let
    /// `position_consumer.run()` start) until personnel's own broker
    /// task unblocks, which it can't do until position's completion
    /// signal arrives. A non-concurrent implementation deadlocks this
    /// test.
    #[rusty_tokio::test]
    async fn readiness_reporting_product_run_drives_both_consumers_concurrently() {
        let personnel_topic = "manpower.personnel-lifecycle.assignments";
        let position_topic = "manpower.position-management.fills";

        let (producer, _producer_peer) = started_producer().await;

        let (personnel_reg_url, _personnel_reg_server) = start_fake_http_server(vec![
            (200, r#"[{"id": 1, "name": "personnel-lifecycle"}]"#),
            (
                200,
                r#"[{"id": 10, "data_product_id": 1, "description": "assignments", "topic_name": "manpower.personnel-lifecycle.assignments"}]"#,
            ),
            (404, "{}"),
        ]);
        let (position_reg_url, _position_reg_server) = start_fake_http_server(vec![
            (200, r#"[{"id": 2, "name": "position-management"}]"#),
            (
                200,
                r#"[{"id": 20, "data_product_id": 2, "description": "fills", "topic_name": "manpower.position-management.fills"}]"#,
            ),
            (404, "{}"),
        ]);

        let personnel_db = temp_db_path("concurrent_personnel");
        let (personnel_client_io, mut personnel_peer) = duplex(8192);
        let personnel_base = DataProductConsumerBase::<PersonnelAssigned, Dup>::new(
            PersonnelAssignmentConsumer::<Dup>::PRODUCT_NAME,
            PersonnelAssignmentConsumer::<Dup>::PORT_NAME,
            PersonnelAssignmentConsumer::<Dup>::GROUP_ID,
            RegistryClient::new(&personnel_reg_url),
            LineageTracker::new(&personnel_db).unwrap(),
            KafkaClient::new(personnel_client_io, None),
        );
        let mut personnel_consumer =
            PersonnelAssignmentConsumer::new(personnel_base, producer.clone());

        let position_db = temp_db_path("concurrent_position");
        let (position_client_io, mut position_peer) = duplex(8192);
        let position_base = DataProductConsumerBase::<PositionFilled, Dup>::new(
            PositionFillConsumer::<Dup>::PRODUCT_NAME,
            PositionFillConsumer::<Dup>::PORT_NAME,
            PositionFillConsumer::<Dup>::GROUP_ID,
            RegistryClient::new(&position_reg_url),
            LineageTracker::new(&position_db).unwrap(),
            KafkaClient::new(position_client_io, None),
        );
        let mut position_consumer = PositionFillConsumer::new(position_base, producer.clone());

        let personnel_startup = rusty_tokio::spawn(async move {
            respond_to_startup_single_partition(&mut personnel_peer, personnel_topic).await;
            personnel_peer
        });
        let position_startup = rusty_tokio::spawn(async move {
            respond_to_startup_single_partition(&mut position_peer, position_topic).await;
            position_peer
        });
        personnel_consumer.startup().await.unwrap();
        position_consumer.startup().await.unwrap();
        let mut personnel_peer = personnel_startup.await.unwrap();
        let mut position_peer = position_startup.await.unwrap();

        let mut product =
            ReadinessReportingProduct::new(producer, personnel_consumer, position_consumer);
        let (personnel_stop, position_stop) = product.stop_handles();

        let (position_done_tx, position_done_rx) = rusty_tokio::sync::oneshot::channel::<()>();

        let personnel_poll = rusty_tokio::spawn(async move {
            let (header, _body) = recv_request(&mut personnel_peer).await.unwrap();
            assert_eq!(header.api_key, api_key::HEARTBEAT);
            let response = HeartbeatResponse { error_code: 0 };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(
                &mut personnel_peer,
                header.correlation_id,
                writer.as_slice(),
            )
            .await
            .unwrap();

            let (header, _body) = recv_request(&mut personnel_peer).await.unwrap();
            assert_eq!(header.api_key, api_key::FETCH);

            // Block until position's own Heartbeat+Fetch round trip
            // has already finished -- see this test's own doc.
            position_done_rx.await.unwrap();

            let response = empty_fetch_response(personnel_topic);
            let mut writer = Writer::new();
            response.encode(&mut writer, 1_735_689_600_000);

            // Stop before sending -- see
            // `personnel_assignment_consumer_run_derives_publishes_and_commits_for_one_event`'s
            // own comment for why calling it after the response is
            // sent would race the client's next `running` check on
            // this multi-threaded runtime.
            personnel_stop.stop();
            send_response(
                &mut personnel_peer,
                header.correlation_id,
                writer.as_slice(),
            )
            .await
            .unwrap();

            let (header, _body) = recv_request(&mut personnel_peer).await.unwrap();
            assert_eq!(header.api_key, api_key::LEAVE_GROUP);
            let response = LeaveGroupResponse { error_code: 0 };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(
                &mut personnel_peer,
                header.correlation_id,
                writer.as_slice(),
            )
            .await
            .unwrap();
        });

        let position_poll = rusty_tokio::spawn(async move {
            let (header, _body) = recv_request(&mut position_peer).await.unwrap();
            assert_eq!(header.api_key, api_key::HEARTBEAT);
            let response = HeartbeatResponse { error_code: 0 };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut position_peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();

            let (header, _body) = recv_request(&mut position_peer).await.unwrap();
            assert_eq!(header.api_key, api_key::FETCH);
            let response = empty_fetch_response(position_topic);
            let mut writer = Writer::new();
            response.encode(&mut writer, 1_735_689_600_000);

            // Stop before sending, same reasoning as personnel's own
            // task above.
            position_stop.stop();
            send_response(&mut position_peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();
            let _ = position_done_tx.send(());

            let (header, _body) = recv_request(&mut position_peer).await.unwrap();
            assert_eq!(header.api_key, api_key::LEAVE_GROUP);
            let response = LeaveGroupResponse { error_code: 0 };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut position_peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();
        });

        product.run().await.unwrap();
        personnel_poll.await.unwrap();
        position_poll.await.unwrap();
    }
}
