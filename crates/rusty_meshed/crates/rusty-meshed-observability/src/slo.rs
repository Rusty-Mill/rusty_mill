//! SLO evaluation and violation publishing -- the Rust port of
//! `meshed.observability.slo`'s `SLOResult`, `SLOViolationPayload`,
//! `SLOMonitor` (GOV-041..046), and `SLOViolationPublisher`
//! (GOV-047..049).
//!
//! Freshness and completeness both read the same signal --
//! `_get_latest_timestamp_seconds_ago`'s high-watermark timestamp, via
//! `rusty_kafka`'s `ListOffsets` (v1, added specifically to unblock
//! this and [`crate::MetricsCollector`]) -- and, per the source's own
//! docstring, completeness is a v1 liveness proxy (a stalled partition
//! counts as incomplete), not a true expected-vs-actual record count.
//!
//! [`SLOViolationPublisher`] is built on `rusty_kafka`'s `Produce`
//! support -- see that crate's own module doc for the caveat this
//! inherits (no live broker to validate the record-batch v2 wire
//! format against in this environment).

use crate::metrics::{get_violation_count, KAFKA_TIMEOUT};
use rusty_err::Error;
use rusty_json::json;
use rusty_kafka::protocol::list_offsets::{
    ListOffsetsPartitionRequest, ListOffsetsRequest, ListOffsetsTopicRequest, LATEST_TIMESTAMP,
};
use rusty_kafka::protocol::produce::{
    ProducePartitionRequest, ProduceRequest, ProduceTopicRequest,
};
use rusty_kafka::record_batch::Record;
use rusty_kafka::{ClientError, KafkaClient};
use rusty_sqlite::rusqlite::{Connection, Result as SqlResult};
use rusty_tokio::io::{AsyncRead, AsyncWrite, TcpStream};
use rusty_tokio::time::timeout;

/// The outcome of evaluating one SLO dimension (GOV-041).
#[derive(Debug, Clone, PartialEq)]
pub struct SLOResult {
    /// `"freshness"`, `"completeness"`, or `"schema_conformance"`.
    pub slo_type: String,
    pub passed: bool,
    pub threshold: f64,
    pub actual_value: f64,
    pub message: String,
}

/// A governance event payload for an SLO violation, published by
/// [`SLOViolationPublisher`] to `mesh.governance.slo-violations` as
/// plain JSON, not Avro -- SLO violations are platform infrastructure,
/// not a domain data product (GOV-05, per the source's own design
/// note) (GOV-042).
#[derive(Debug, Clone, PartialEq)]
pub struct SLOViolationPayload {
    pub product_name: String,
    pub port_name: String,
    pub slo_type: String,
    pub threshold: f64,
    pub actual_value: f64,
    pub violation_message: String,
    /// Auto-generated UUID v4, fresh per instance.
    pub event_id: String,
    /// Auto-generated UTC timestamp, fresh per instance.
    pub timestamp: String,
    /// Auto-generated UUID v4, fresh per instance -- independent of
    /// `event_id`, matching the source's two separate
    /// `default_factory=lambda: str(uuid.uuid4())` fields.
    pub correlation_id: String,
}

impl SLOViolationPayload {
    /// Builds a payload for `slo_result`'s violation, auto-generating
    /// `event_id`/`timestamp`/`correlation_id` the same way the
    /// source's dataclass field factories do.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        product_name: impl Into<String>,
        port_name: impl Into<String>,
        slo_type: impl Into<String>,
        threshold: f64,
        actual_value: f64,
        violation_message: impl Into<String>,
    ) -> Self {
        SLOViolationPayload {
            product_name: product_name.into(),
            port_name: port_name.into(),
            slo_type: slo_type.into(),
            threshold,
            actual_value,
            violation_message: violation_message.into(),
            event_id: rusty_uuid::Uuid::new_v4().to_string(),
            timestamp: now_iso(),
            correlation_id: rusty_uuid::Uuid::new_v4().to_string(),
        }
    }
}

/// Runs one Kafka call under [`KAFKA_TIMEOUT`] -- the same bound the
/// metrics collector puts on every broker round trip, for the same
/// reason (see that constant's docs). Surfaced as `ClientError::Io`
/// with `ErrorKind::TimedOut` so the existing `ClientError`-shaped
/// signatures here (`connect`, `publish`) stay as they are.
async fn bounded<T>(
    call: impl std::future::Future<Output = Result<T, ClientError>>,
) -> Result<T, ClientError> {
    match timeout(KAFKA_TIMEOUT, call).await {
        Ok(result) => result,
        Err(_elapsed) => Err(ClientError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("Kafka call did not complete within {KAFKA_TIMEOUT:?}"),
        ))),
    }
}

/// Evaluates SLO dimensions for a data product's output port, backed
/// by a single [`rusty_kafka::KafkaClient`] connection. Every broker
/// call is bounded at [`KAFKA_TIMEOUT`].
pub struct SLOMonitor<S> {
    client: KafkaClient<S>,
}

impl SLOMonitor<TcpStream> {
    /// Connects to the Kafka broker at `bootstrap_servers`.
    pub async fn connect(bootstrap_servers: &str) -> Result<Self, ClientError> {
        let client = bounded(KafkaClient::connect(
            bootstrap_servers,
            Some("rusty_meshed_slo".to_string()),
        ))
        .await?;
        Ok(SLOMonitor { client })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> SLOMonitor<S> {
    /// Wraps an already-connected [`rusty_kafka::KafkaClient`] -- the
    /// seam this crate's own tests use (an in-memory
    /// `rusty_tokio::io::duplex` pair standing in for a broker) instead
    /// of a real TCP connection.
    pub fn with_client(client: KafkaClient<S>) -> Self {
        SLOMonitor { client }
    }

    /// The age, in seconds, of `topic`/`partition`'s latest message
    /// (GOV-043). `f64::INFINITY` if the partition is empty
    /// (`timestamp < 0`) or the lookup failed for any reason --
    /// connection failure, protocol error, or a non-zero Kafka error
    /// code.
    ///
    /// The source's `admin.list_offsets(...)` result can come back as
    /// either `ListOffsetsResultInfo` directly or wrapped in a
    /// `concurrent.futures.Future`, depending on `confluent-kafka`
    /// version -- only the `Future`-wrapped case's exception is caught
    /// (falling back to `float('inf')`); `rusty_kafka::KafkaClient` has
    /// no such duality (`list_offsets` always awaits one request and
    /// returns one `Result` directly), so there's only one failure path
    /// to handle here, not two -- it collapses to the same `inf`
    /// outcome either way.
    async fn latest_timestamp_seconds_ago(&mut self, topic: &str, partition: i32) -> f64 {
        let request = ListOffsetsRequest {
            replica_id: -1,
            topics: vec![ListOffsetsTopicRequest {
                name: topic.to_string(),
                partitions: vec![ListOffsetsPartitionRequest {
                    partition_index: partition,
                    timestamp: LATEST_TIMESTAMP,
                }],
            }],
        };
        let Ok(response) = bounded(self.client.list_offsets(&request)).await else {
            return f64::INFINITY;
        };
        let Some(result) = response.topics.first().and_then(|t| t.partitions.first()) else {
            return f64::INFINITY;
        };
        if result.error_code != 0 || result.timestamp < 0 {
            return f64::INFINITY;
        }
        (now_millis() - result.timestamp as f64) / 1000.0
    }

    /// Freshness SLO: passes iff the latest message on `topic`/`partition`
    /// is no older than `threshold_seconds` (GOV-044).
    pub async fn check_freshness(
        &mut self,
        topic: &str,
        partition: i32,
        threshold_seconds: i64,
    ) -> SLOResult {
        let age_seconds = self.latest_timestamp_seconds_ago(topic, partition).await;
        let passed = age_seconds <= threshold_seconds as f64;

        let message = if age_seconds.is_infinite() {
            "No messages exist in partition — data product has never published".to_string()
        } else if passed {
            format!(
                "Freshness OK: last message {age_seconds:.1}s ago (threshold={threshold_seconds}s)"
            )
        } else {
            format!(
                "Freshness violated: last message {age_seconds:.1}s ago (threshold={threshold_seconds}s)"
            )
        };

        SLOResult {
            slo_type: "freshness".to_string(),
            passed,
            threshold: threshold_seconds as f64,
            actual_value: age_seconds,
            message,
        }
    }

    /// Completeness SLO (GOV-045) -- a v1 liveness proxy using the same
    /// arithmetic as [`check_freshness`](Self::check_freshness): a
    /// partition whose latest message is older than `threshold_seconds`
    /// is considered stalled, not incomplete in the "expected vs.
    /// actual record count" sense (deferred, per the source's own
    /// docstring).
    pub async fn check_completeness(
        &mut self,
        topic: &str,
        partition: i32,
        threshold_seconds: i64,
    ) -> SLOResult {
        let age_seconds = self.latest_timestamp_seconds_ago(topic, partition).await;
        let passed = age_seconds <= threshold_seconds as f64;

        let message = if age_seconds.is_infinite() {
            "No messages exist — completeness check failed (empty partition)".to_string()
        } else if passed {
            format!(
                "Completeness OK (liveness): last message {age_seconds:.1}s ago (threshold={threshold_seconds}s)"
            )
        } else {
            format!(
                "Completeness violated (liveness): partition stalled for {age_seconds:.1}s (threshold={threshold_seconds}s)"
            )
        };

        SLOResult {
            slo_type: "completeness".to_string(),
            passed,
            threshold: threshold_seconds as f64,
            actual_value: age_seconds,
            message,
        }
    }

    /// Schema conformance SLO: passes iff `subject` has zero recorded
    /// schema violations (GOV-046). `threshold` is fixed `0.0`; unlike
    /// the two Kafka-backed checks above, a lookup failure here
    /// propagates as an `Err` rather than degrading to a result, same
    /// as the source (no `try`/`except` around
    /// `MetricsCollector.get_violation_count`).
    pub fn check_schema_conformance(
        &self,
        conn: &Connection,
        subject: &str,
    ) -> SqlResult<SLOResult> {
        let violation_count = get_violation_count(conn, subject)?;
        let passed = violation_count == 0;

        let message = if passed {
            format!("Schema conformance OK: 0 violations for subject '{subject}'")
        } else {
            format!(
                "Schema conformance violated: {violation_count} violation(s) recorded for subject '{subject}'"
            )
        };

        Ok(SLOResult {
            slo_type: "schema_conformance".to_string(),
            passed,
            threshold: 0.0,
            actual_value: violation_count as f64,
            message,
        })
    }
}

/// Errors from a [`SLOViolationPublisher`] Kafka call -- same shape as
/// [`crate::MetricsError`], this crate family's established pattern for
/// a `rusty_kafka`-backed struct's error type.
#[derive(Debug, Error)]
pub enum PublishError {
    /// The underlying Kafka request itself failed (connection, framing,
    /// correlation mismatch, ...).
    #[error("Kafka client error: {0}")]
    Kafka(#[from] ClientError),
    /// The broker's response didn't include a result for the
    /// topic/partition produced to.
    #[error("no result for the requested topic/partition in the broker's response")]
    MissingPartitionResult,
    /// The broker returned a non-zero error code for the produce
    /// (e.g. `UNKNOWN_TOPIC_OR_PARTITION`).
    #[error("broker returned Kafka error code {0}")]
    KafkaErrorCode(i16),
}

/// Publishes [`SLOViolationPayload`]s to `mesh.governance.slo-violations`,
/// backed by a single [`rusty_kafka::KafkaClient`] connection (GOV-047).
pub struct SLOViolationPublisher<S> {
    client: KafkaClient<S>,
}

impl SLOViolationPublisher<TcpStream> {
    /// Connects to the Kafka broker at `bootstrap_servers`.
    pub async fn connect(bootstrap_servers: &str) -> Result<Self, ClientError> {
        let client = bounded(KafkaClient::connect(
            bootstrap_servers,
            Some("rusty_meshed_slo_publisher".to_string()),
        ))
        .await?;
        Ok(SLOViolationPublisher { client })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> SLOViolationPublisher<S> {
    /// The topic every violation is published to -- fixed, matching the
    /// source's `TOPIC` class attribute.
    pub const TOPIC: &'static str = "mesh.governance.slo-violations";

    /// Wraps an already-connected [`rusty_kafka::KafkaClient`] -- the
    /// seam this crate's own tests use (an in-memory
    /// `rusty_tokio::io::duplex` pair standing in for a broker) instead
    /// of a real TCP connection.
    pub fn with_client(client: KafkaClient<S>) -> Self {
        SLOViolationPublisher { client }
    }

    /// Publishes `violation` to [`Self::TOPIC`] (GOV-048): the value is
    /// `violation` JSON-encoded (field set and value encoding matching
    /// the source's `json.dumps(asdict(violation))`; object key order
    /// is *not* reproduced -- `rusty_json::Map` is `BTreeMap`-backed and
    /// always serializes keys alphabetically, never in declaration
    /// order like Python's `dict`, and nothing here or in the source
    /// depends on key order, per the JSON spec itself), the key is
    /// `product_name`'s UTF-8 bytes, and `event_id`/`correlation_id`
    /// are carried as UTF-8 headers, single record, partition 0 --
    /// this client has no partition-count/`Metadata`-based partitioner
    /// (unlike `librdkafka`'s default one the source relies on), so
    /// this deliberately assumes a single-partition topic, matching the
    /// platform's own local-dev deployment.
    pub async fn publish(&mut self, violation: &SLOViolationPayload) -> Result<(), PublishError> {
        let value = rusty_json::to_string(&violation_json(violation)).unwrap_or_default();
        let request = ProduceRequest {
            acks: -1,
            timeout_ms: 5000,
            base_timestamp_ms: now_millis() as i64,
            topics: vec![ProduceTopicRequest {
                name: Self::TOPIC.to_string(),
                partitions: vec![ProducePartitionRequest {
                    partition_index: 0,
                    records: vec![Record {
                        key: Some(violation.product_name.clone().into_bytes()),
                        value: Some(value.into_bytes()),
                        headers: vec![
                            (
                                "event_id".to_string(),
                                Some(violation.event_id.clone().into_bytes()),
                            ),
                            (
                                "correlation_id".to_string(),
                                Some(violation.correlation_id.clone().into_bytes()),
                            ),
                        ],
                    }],
                }],
            }],
        };
        let response = bounded(self.client.produce(&request)).await?;
        let result = response
            .topics
            .first()
            .and_then(|t| t.partitions.first())
            .ok_or(PublishError::MissingPartitionResult)?;
        if result.error_code != 0 {
            return Err(PublishError::KafkaErrorCode(result.error_code));
        }
        Ok(())
    }

    /// A documented no-op (GOV-049). The source's `Producer.flush()`
    /// waits out `librdkafka`'s asynchronous send buffer; there's
    /// nothing to wait for here since [`publish`](Self::publish)
    /// already synchronously awaits the full `Produce` request/response
    /// before returning -- a stronger delivery guarantee at that point,
    /// not a weaker one, so `timeout_seconds` is accepted for call-site
    /// parity but unused.
    pub fn flush(&mut self, _timeout_seconds: f64) {}
}

/// `violation`'s fields as a [`rusty_json::Value`] object, in the same
/// field set as the source's `asdict(violation)` -- see
/// [`SLOViolationPublisher::publish`]'s doc for why the *order* those
/// keys serialize in doesn't match Python's `dict` order.
fn violation_json(violation: &SLOViolationPayload) -> rusty_json::Value {
    json!({
        "product_name": violation.product_name.as_str(),
        "port_name": violation.port_name.as_str(),
        "slo_type": violation.slo_type.as_str(),
        "threshold": violation.threshold,
        "actual_value": violation.actual_value,
        "violation_message": violation.violation_message.as_str(),
        "event_id": violation.event_id.as_str(),
        "timestamp": violation.timestamp.as_str(),
        "correlation_id": violation.correlation_id.as_str(),
    })
}

fn now_millis() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0
}

/// A minimal RFC 3339 UTC "now" formatter -- same hand-rolled
/// civil-from-days algorithm duplicated elsewhere in this crate family
/// (see `metrics::now_iso`'s doc for why there's no shared clock type
/// to build on instead).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{ensure_schema, record_violation};
    use rusty_kafka::protocol::list_offsets::{
        ListOffsetsPartitionResponse, ListOffsetsResponse, ListOffsetsTopicResponse,
    };
    use rusty_kafka::testing::{recv_request, send_response};
    use rusty_tokio::io::duplex;
    use rusty_wire::Writer;

    fn seeded_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        conn
    }

    async fn respond_with_offset(
        peer: &mut (impl rusty_tokio::io::AsyncRead + rusty_tokio::io::AsyncWrite + Unpin + Send),
        timestamp: i64,
        error_code: i16,
    ) {
        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, rusty_kafka::protocol::api_key::LIST_OFFSETS);
        let response = ListOffsetsResponse {
            topics: vec![ListOffsetsTopicResponse {
                name: "t".to_string(),
                partitions: vec![ListOffsetsPartitionResponse {
                    partition_index: 0,
                    error_code,
                    timestamp,
                    offset: 0,
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        send_response(peer, header.correlation_id, &writer.into_vec())
            .await
            .unwrap();
    }

    #[rusty_tokio::test]
    async fn check_freshness_passes_when_the_latest_message_is_within_threshold() {
        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut monitor = SLOMonitor::with_client(client);

        let now_ms = now_millis() as i64;
        let server = rusty_tokio::spawn(async move {
            respond_with_offset(&mut peer, now_ms - 5_000, 0).await;
        });

        let result = monitor.check_freshness("t", 0, 60).await;
        server.await.unwrap();

        assert_eq!(result.slo_type, "freshness");
        assert!(result.passed);
        assert_eq!(result.threshold, 60.0);
        assert!(result.actual_value >= 5.0 && result.actual_value < 10.0);
        assert!(result.message.starts_with("Freshness OK"));
    }

    #[rusty_tokio::test]
    async fn check_freshness_fails_when_the_latest_message_is_older_than_threshold() {
        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut monitor = SLOMonitor::with_client(client);

        let now_ms = now_millis() as i64;
        let server = rusty_tokio::spawn(async move {
            respond_with_offset(&mut peer, now_ms - 120_000, 0).await;
        });

        let result = monitor.check_freshness("t", 0, 60).await;
        server.await.unwrap();

        assert!(!result.passed);
        assert!(result.message.starts_with("Freshness violated"));
    }

    #[rusty_tokio::test]
    async fn check_freshness_treats_a_negative_timestamp_as_an_empty_partition() {
        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut monitor = SLOMonitor::with_client(client);

        let server = rusty_tokio::spawn(async move {
            respond_with_offset(&mut peer, -1, 0).await;
        });

        let result = monitor.check_freshness("t", 0, 60).await;
        server.await.unwrap();

        assert!(!result.passed);
        assert!(result.actual_value.is_infinite());
        assert_eq!(
            result.message,
            "No messages exist in partition — data product has never published"
        );
    }

    #[rusty_tokio::test]
    async fn check_freshness_treats_a_kafka_error_code_as_infinite_age() {
        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut monitor = SLOMonitor::with_client(client);

        let server = rusty_tokio::spawn(async move {
            respond_with_offset(&mut peer, 0, 3 /* UNKNOWN_TOPIC_OR_PARTITION */).await;
        });

        let result = monitor.check_freshness("no-such-topic", 0, 60).await;
        server.await.unwrap();

        assert!(!result.passed);
        assert!(result.actual_value.is_infinite());
    }

    #[rusty_tokio::test]
    async fn check_freshness_treats_a_connection_failure_as_infinite_age() {
        // Drop the peer immediately -- the client's list_offsets call
        // fails outright (no response ever arrives), which must
        // collapse to the same "no signal" outcome as an explicit
        // negative-timestamp/empty-partition response.
        let (client_io, peer) = duplex(4096);
        drop(peer);
        let client = KafkaClient::new(client_io, None);
        let mut monitor = SLOMonitor::with_client(client);

        let result = monitor.check_freshness("t", 0, 60).await;
        assert!(!result.passed);
        assert!(result.actual_value.is_infinite());
    }

    #[rusty_tokio::test]
    async fn check_completeness_uses_the_same_arithmetic_as_freshness_with_distinct_wording() {
        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut monitor = SLOMonitor::with_client(client);

        let now_ms = now_millis() as i64;
        let server = rusty_tokio::spawn(async move {
            respond_with_offset(&mut peer, now_ms - 120_000, 0).await;
        });

        let result = monitor.check_completeness("t", 0, 60).await;
        server.await.unwrap();

        assert_eq!(result.slo_type, "completeness");
        assert!(!result.passed);
        assert!(result
            .message
            .starts_with("Completeness violated (liveness)"));
    }

    /// `check_schema_conformance` never touches the Kafka client at
    /// all, so an unused (never driven) `duplex` half is a fine stand-in
    /// -- these tests don't need `#[rusty_tokio::test]`.
    fn monitor_with_no_kafka_traffic() -> SLOMonitor<rusty_tokio::io::DuplexStream> {
        let (client_io, _peer) = duplex(4096);
        SLOMonitor::with_client(KafkaClient::new(client_io, None))
    }

    #[test]
    fn check_schema_conformance_passes_with_zero_violations() {
        let conn = seeded_connection();
        let monitor = monitor_with_no_kafka_traffic();
        let result = monitor
            .check_schema_conformance(&conn, "orders-value")
            .unwrap();
        assert_eq!(result.slo_type, "schema_conformance");
        assert!(result.passed);
        assert_eq!(result.threshold, 0.0);
        assert_eq!(result.actual_value, 0.0);
        assert!(result.message.starts_with("Schema conformance OK"));
    }

    #[test]
    fn check_schema_conformance_fails_with_recorded_violations() {
        let conn = seeded_connection();
        record_violation(&conn, "orders-value", "field removed").unwrap();
        record_violation(&conn, "orders-value", "type changed").unwrap();
        let monitor = monitor_with_no_kafka_traffic();
        let result = monitor
            .check_schema_conformance(&conn, "orders-value")
            .unwrap();
        assert!(!result.passed);
        assert_eq!(result.actual_value, 2.0);
        assert!(result.message.starts_with("Schema conformance violated: 2"));
    }

    #[test]
    fn slo_violation_payload_auto_generates_distinct_ids_and_a_timestamp() {
        let payload = SLOViolationPayload::new(
            "orders",
            "commerce.orders",
            "freshness",
            60.0,
            125.4,
            "Freshness violated: last message 125.4s ago (threshold=60s)",
        );
        assert_ne!(payload.event_id, payload.correlation_id);
        assert!(!payload.event_id.is_empty());
        assert!(!payload.timestamp.is_empty());

        let second =
            SLOViolationPayload::new("orders", "commerce.orders", "freshness", 60.0, 125.4, "x");
        assert_ne!(payload.event_id, second.event_id);
        assert_ne!(payload.correlation_id, second.correlation_id);
    }

    fn sample_violation() -> SLOViolationPayload {
        SLOViolationPayload::new(
            "orders",
            "commerce.orders",
            "freshness",
            60.0,
            125.4,
            "Freshness violated: last message 125.4s ago (threshold=60s)",
        )
    }

    async fn respond_to_produce(
        peer: &mut (impl rusty_tokio::io::AsyncRead + rusty_tokio::io::AsyncWrite + Unpin + Send),
        error_code: i16,
    ) -> rusty_kafka::protocol::produce::ProduceRequest {
        use rusty_kafka::protocol::produce::{
            ProducePartitionResponse, ProduceRequest, ProduceResponse, ProduceTopicResponse,
        };

        let (header, body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, rusty_kafka::protocol::api_key::PRODUCE);
        let mut reader = rusty_wire::Reader::new(&body);
        let request = ProduceRequest::decode(&mut reader).unwrap();

        let response = ProduceResponse {
            topics: vec![ProduceTopicResponse {
                name: SLOViolationPublisher::<rusty_tokio::io::DuplexStream>::TOPIC.to_string(),
                partitions: vec![ProducePartitionResponse {
                    partition_index: 0,
                    error_code,
                    base_offset: 7,
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
        request
    }

    #[rusty_tokio::test]
    async fn publish_sends_the_violation_as_a_single_record_to_partition_zero() {
        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut publisher = SLOViolationPublisher::with_client(client);
        let violation = sample_violation();
        let expected_key = violation.product_name.clone().into_bytes();
        let expected_event_id = violation.event_id.clone();
        let expected_correlation_id = violation.correlation_id.clone();

        let server = rusty_tokio::spawn(async move { respond_to_produce(&mut peer, 0).await });

        publisher.publish(&violation).await.unwrap();
        let sent = server.await.unwrap();

        assert_eq!(sent.topics.len(), 1);
        assert_eq!(
            sent.topics[0].name,
            SLOViolationPublisher::<rusty_tokio::io::DuplexStream>::TOPIC
        );
        assert_eq!(sent.topics[0].partitions.len(), 1);
        let partition = &sent.topics[0].partitions[0];
        assert_eq!(partition.partition_index, 0);
        assert_eq!(partition.records.len(), 1);
        let record = &partition.records[0];
        assert_eq!(record.key, Some(expected_key));
        assert_eq!(
            record.headers,
            vec![
                ("event_id".to_string(), Some(expected_event_id.into_bytes())),
                (
                    "correlation_id".to_string(),
                    Some(expected_correlation_id.into_bytes())
                ),
            ]
        );

        let value = record.value.as_ref().unwrap();
        let parsed: rusty_json::Value = rusty_json::from_str(std::str::from_utf8(value).unwrap())
            .expect("value must be valid JSON");
        let object = parsed.as_object().expect("value must be a JSON object");
        assert_eq!(
            object.get("product_name").and_then(|v| v.as_str()),
            Some("orders")
        );
        assert_eq!(
            object.get("slo_type").and_then(|v| v.as_str()),
            Some("freshness")
        );
        assert_eq!(object.get("threshold").and_then(|v| v.as_f64()), Some(60.0));
        assert_eq!(
            object.get("actual_value").and_then(|v| v.as_f64()),
            Some(125.4)
        );
    }

    #[rusty_tokio::test]
    async fn publish_returns_a_kafka_error_code_when_the_broker_rejects_it() {
        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut publisher = SLOViolationPublisher::with_client(client);
        let violation = sample_violation();

        let server = rusty_tokio::spawn(async move {
            respond_to_produce(&mut peer, 3 /* UNKNOWN_TOPIC_OR_PARTITION */).await
        });

        let err = publisher.publish(&violation).await.unwrap_err();
        server.await.unwrap();

        assert!(matches!(err, PublishError::KafkaErrorCode(3)));
    }

    #[test]
    fn flush_is_a_documented_no_op() {
        let (client_io, _peer) = duplex(4096);
        let mut publisher = SLOViolationPublisher::with_client(KafkaClient::new(client_io, None));
        publisher.flush(5.0);
    }

    /// A broker that takes the request but never answers must collapse
    /// to the same "no signal" outcome as a refused connection, after
    /// `KAFKA_TIMEOUT` rather than never. `_peer` is held open so only
    /// the missing response ends the call.
    #[rusty_tokio::test]
    async fn check_freshness_treats_a_silent_broker_as_infinite_age_after_the_bound() {
        let (client_io, _peer) = duplex(4096);
        let mut monitor = SLOMonitor::with_client(KafkaClient::new(client_io, None));
        let started = std::time::Instant::now();
        let result = monitor.check_freshness("orders", 0, 60).await;
        assert!(!result.passed);
        assert!(result.actual_value.is_infinite());
        assert!(
            started.elapsed() >= KAFKA_TIMEOUT - std::time::Duration::from_millis(500),
            "returned too early: {:?}",
            started.elapsed()
        );
    }

    /// Same for the publisher: a silent broker is a `TimedOut` I/O
    /// error after the bound, not a hang.
    #[rusty_tokio::test]
    async fn publish_to_a_silent_broker_times_out_instead_of_hanging() {
        let (client_io, _peer) = duplex(4096);
        let mut publisher = SLOViolationPublisher::with_client(KafkaClient::new(client_io, None));
        let violation = SLOViolationPayload::new(
            "orders",
            "commerce.orders",
            "freshness",
            60.0,
            f64::INFINITY,
            "latest message is older than 60s",
        );
        let started = std::time::Instant::now();
        let err = publisher.publish(&violation).await.unwrap_err();
        assert!(
            matches!(&err, PublishError::Kafka(ClientError::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut),
            "{err:?}"
        );
        assert!(
            started.elapsed() >= KAFKA_TIMEOUT - std::time::Duration::from_millis(500),
            "returned too early: {:?}",
            started.elapsed()
        );
    }
}
