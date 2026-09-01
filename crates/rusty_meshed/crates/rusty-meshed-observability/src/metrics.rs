//! Kafka event-flow metrics and schema-violation tracking -- the Rust
//! port of `meshed.observability.metrics` (`SchemaViolation`,
//! `MetricsCollector`, GOV-033..039).
//!
//! `compute_lag`/`get_throughput` are built directly on
//! `rusty_kafka`'s `ListOffsets`/`OffsetFetch` client methods (added
//! specifically to unblock this module -- see those protocol modules'
//! own docs for the wire-level detail, including `OffsetFetch`'s
//! coordinator-routing caveat, which applies here unchanged).
//! `record_violation`/`get_violation_count` are plain SQLite functions
//! (the source's `@staticmethod`s -- no `MetricsCollector` state
//! needed for either), following this crate family's per-call-
//! connection convention.

use rusty_err::Error;
use rusty_kafka::protocol::list_offsets::{
    ListOffsetsPartitionRequest, ListOffsetsRequest, ListOffsetsTopicRequest, LATEST_TIMESTAMP,
};
use rusty_kafka::protocol::offset_fetch::{OffsetFetchRequest, OffsetFetchTopicRequest};
use rusty_kafka::{ClientError, KafkaClient};
use rusty_sqlite::rusqlite::{params, Connection, Result as SqlResult};
use rusty_tokio::io::{AsyncRead, AsyncWrite, TcpStream};

/// Errors from a [`MetricsCollector`] Kafka call.
#[derive(Debug, Error)]
pub enum MetricsError {
    /// The underlying Kafka request itself failed (connection, framing,
    /// correlation mismatch, ...).
    #[error("Kafka client error: {0}")]
    Kafka(#[from] ClientError),
    /// The broker's response didn't include a result for the
    /// topic/partition queried.
    #[error("no result for the requested topic/partition in the broker's response")]
    MissingPartitionResult,
    /// The broker returned a non-zero error code for the
    /// topic/partition queried (e.g. `UNKNOWN_TOPIC_OR_PARTITION`).
    #[error("broker returned Kafka error code {0}")]
    KafkaErrorCode(i16),
    /// The `schema_violations` lookup itself failed.
    #[error("failed to read violation count: {0}")]
    Sql(String),
}

/// `{lag, throughput, violation_count}` -- the Rust port of
/// `get_product_metrics()`'s return dict (GOV-039).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductMetrics {
    pub lag: i64,
    pub throughput: i64,
    pub violation_count: i64,
}

/// Creates the `schema_violations` table if it doesn't already exist.
pub fn ensure_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_violations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            subject TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            error_message TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_schema_violations_subject ON schema_violations(subject);",
    )
}

/// Persists a schema violation record (GOV-037). A plain function, not
/// a `MetricsCollector` method -- the source's `record_violation` is a
/// `@staticmethod` needing no Kafka state either.
pub fn record_violation(conn: &Connection, subject: &str, error_message: &str) -> SqlResult<()> {
    let timestamp = now_iso();
    conn.execute(
        "INSERT INTO schema_violations (subject, timestamp, error_message) VALUES (?1, ?2, ?3)",
        params![subject, timestamp, error_message],
    )?;
    Ok(())
}

/// Counts recorded violations for `subject` (GOV-038).
pub fn get_violation_count(conn: &Connection, subject: &str) -> SqlResult<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM schema_violations WHERE subject = ?1",
        params![subject],
        |row| row.get(0),
    )
}

/// Computes Kafka lag/throughput for a data product, backed by a
/// single [`rusty_kafka::KafkaClient`] connection.
pub struct MetricsCollector<S> {
    client: KafkaClient<S>,
}

impl MetricsCollector<TcpStream> {
    /// Connects to the Kafka broker at `bootstrap_servers` (e.g.
    /// `PlatformConfig::kafka_bootstrap_servers`).
    pub async fn connect(bootstrap_servers: &str) -> Result<Self, MetricsError> {
        let client =
            KafkaClient::connect(bootstrap_servers, Some("rusty_meshed_metrics".to_string()))
                .await?;
        Ok(MetricsCollector { client })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> MetricsCollector<S> {
    /// Wraps an already-connected [`rusty_kafka::KafkaClient`] -- the
    /// seam this crate's own tests use (an in-memory
    /// `rusty_tokio::io::duplex` pair standing in for a broker) instead
    /// of a real TCP connection.
    pub fn with_client(client: KafkaClient<S>) -> Self {
        MetricsCollector { client }
    }

    /// Sums `max(0, high_watermark - committed_offset)` across
    /// partitions `0..num_partitions` for `group_id` on `topic`
    /// (GOV-034). Any committed offset `< 0` (no commit yet, including
    /// `confluent_kafka`'s `-1001` sentinel that the source's own
    /// comment names -- `rusty_kafka`'s wire-level sentinel is `-1`,
    /// see [`rusty_kafka::protocol::offset_fetch::NO_COMMITTED_OFFSET`])
    /// counts as `0` committed (GOV-035): an unconsumed topic's lag
    /// equals its full high-watermark.
    pub async fn compute_lag(
        &mut self,
        group_id: &str,
        topic: &str,
        num_partitions: i32,
    ) -> Result<i64, MetricsError> {
        let mut total_lag: i64 = 0;
        for partition in 0..num_partitions {
            let high = self.watermark(topic, partition).await?;
            let committed = self.committed_offset(group_id, topic, partition).await?;
            let committed = committed.max(0);
            total_lag += (high - committed).max(0);
        }
        Ok(total_lag)
    }

    /// Returns the high-watermark offset for `topic`/`partition` as a
    /// throughput proxy (GOV-036) -- a v1 approximation, per the
    /// source's own docstring, not an actual message-rate measurement.
    pub async fn get_throughput(
        &mut self,
        topic: &str,
        partition: i32,
    ) -> Result<i64, MetricsError> {
        self.watermark(topic, partition).await
    }

    /// Combines [`compute_lag`](Self::compute_lag),
    /// [`get_throughput`](Self::get_throughput), and
    /// [`get_violation_count`] into one result (GOV-039).
    pub async fn get_product_metrics(
        &mut self,
        conn: &Connection,
        group_id: &str,
        topic: &str,
        num_partitions: i32,
        subject: &str,
    ) -> Result<ProductMetrics, MetricsError> {
        let lag = self.compute_lag(group_id, topic, num_partitions).await?;
        let throughput = self.get_throughput(topic, 0).await?;
        let violation_count =
            get_violation_count(conn, subject).map_err(|e| MetricsError::Sql(e.to_string()))?;
        Ok(ProductMetrics {
            lag,
            throughput,
            violation_count,
        })
    }

    async fn watermark(&mut self, topic: &str, partition: i32) -> Result<i64, MetricsError> {
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
        let response = self.client.list_offsets(&request).await?;
        let result = response
            .topics
            .first()
            .and_then(|t| t.partitions.first())
            .ok_or(MetricsError::MissingPartitionResult)?;
        if result.error_code != 0 {
            return Err(MetricsError::KafkaErrorCode(result.error_code));
        }
        Ok(result.offset)
    }

    async fn committed_offset(
        &mut self,
        group_id: &str,
        topic: &str,
        partition: i32,
    ) -> Result<i64, MetricsError> {
        let request = OffsetFetchRequest {
            group_id: format!("_meshed_metrics_{group_id}"),
            topics: vec![OffsetFetchTopicRequest {
                name: topic.to_string(),
                partitions: vec![partition],
            }],
        };
        let response = self.client.offset_fetch(&request).await?;
        let result = response
            .topics
            .first()
            .and_then(|t| t.partitions.first())
            .ok_or(MetricsError::MissingPartitionResult)?;
        if result.error_code != 0 {
            return Err(MetricsError::KafkaErrorCode(result.error_code));
        }
        Ok(result.committed_offset)
    }
}

/// A minimal RFC 3339 UTC "now" formatter -- same hand-rolled
/// civil-from-days algorithm used elsewhere in this crate family for a
/// `timestamp` field no test asserts the exact value of; see
/// `rusty-meshed-registry::transformation::engine::now_iso`'s doc for
/// why there's no shared clock type to build on instead.
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
    use rusty_kafka::protocol::header::RequestHeader;
    use rusty_kafka::protocol::list_offsets::{
        ListOffsetsPartitionResponse, ListOffsetsResponse, ListOffsetsTopicResponse,
    };
    use rusty_kafka::protocol::offset_fetch::{
        OffsetFetchPartitionResponse, OffsetFetchResponse, OffsetFetchTopicResponse,
    };
    use rusty_kafka::testing::{recv_request, send_response};
    use rusty_tokio::io::duplex;
    use rusty_wire::Writer;

    fn seeded_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn record_violation_and_get_violation_count_round_trip() {
        let conn = seeded_connection();
        record_violation(&conn, "orders-value", "field removed").unwrap();
        record_violation(&conn, "orders-value", "type changed").unwrap();
        record_violation(&conn, "other-value", "unrelated").unwrap();

        assert_eq!(get_violation_count(&conn, "orders-value").unwrap(), 2);
        assert_eq!(get_violation_count(&conn, "other-value").unwrap(), 1);
        assert_eq!(get_violation_count(&conn, "no-such-subject").unwrap(), 0);
    }

    /// Serves one `ListOffsets`/`OffsetFetch` request per call, driven
    /// by a handler that decodes the request header and produces the
    /// response body -- lets each test describe just "what does the
    /// fake broker send back" without repeating the frame/header
    /// plumbing.
    async fn respond_once<S, F>(peer: &mut S, api_key: i16, respond: F)
    where
        S: rusty_tokio::io::AsyncRead + rusty_tokio::io::AsyncWrite + Unpin + Send,
        F: FnOnce(&RequestHeader) -> Vec<u8>,
    {
        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, api_key);
        let response_body = respond(&header);
        send_response(peer, header.correlation_id, &response_body)
            .await
            .unwrap();
    }

    fn list_offsets_response_body(offset: i64, error_code: i16) -> Vec<u8> {
        let response = ListOffsetsResponse {
            topics: vec![ListOffsetsTopicResponse {
                name: "t".to_string(),
                partitions: vec![ListOffsetsPartitionResponse {
                    partition_index: 0,
                    error_code,
                    timestamp: 0,
                    offset,
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        writer.into_vec()
    }

    fn offset_fetch_response_body(committed_offset: i64, error_code: i16) -> Vec<u8> {
        let response = OffsetFetchResponse {
            topics: vec![OffsetFetchTopicResponse {
                name: "t".to_string(),
                partitions: vec![OffsetFetchPartitionResponse {
                    partition_index: 0,
                    committed_offset,
                    metadata: None,
                    error_code,
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        writer.into_vec()
    }

    #[rusty_tokio::test]
    async fn compute_lag_sums_high_watermark_minus_committed_across_partitions() {
        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut collector = MetricsCollector::with_client(client);

        let server = rusty_tokio::spawn(async move {
            // Partition 0: high=100, committed=40 -> lag 60.
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::LIST_OFFSETS,
                |_| list_offsets_response_body(100, 0),
            )
            .await;
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::OFFSET_FETCH,
                |_| offset_fetch_response_body(40, 0),
            )
            .await;
            // Partition 1: high=50, committed=50 -> lag 0.
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::LIST_OFFSETS,
                |_| list_offsets_response_body(50, 0),
            )
            .await;
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::OFFSET_FETCH,
                |_| offset_fetch_response_body(50, 0),
            )
            .await;
        });

        let lag = collector
            .compute_lag("readiness-reporting", "manpower.assessments", 2)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(lag, 60);
    }

    #[rusty_tokio::test]
    async fn compute_lag_treats_no_committed_offset_as_zero() {
        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut collector = MetricsCollector::with_client(client);

        let server = rusty_tokio::spawn(async move {
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::LIST_OFFSETS,
                |_| list_offsets_response_body(75, 0),
            )
            .await;
            // -1: the real Kafka wire sentinel for "never committed".
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::OFFSET_FETCH,
                |_| offset_fetch_response_body(-1, 0),
            )
            .await;
        });

        let lag = collector
            .compute_lag("readiness-reporting", "manpower.assessments", 1)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(lag, 75);
    }

    #[rusty_tokio::test]
    async fn compute_lag_treats_the_confluent_sentinel_as_zero_too() {
        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut collector = MetricsCollector::with_client(client);

        let server = rusty_tokio::spawn(async move {
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::LIST_OFFSETS,
                |_| list_offsets_response_body(10, 0),
            )
            .await;
            // confluent_kafka's own OFFSET_INVALID (-1001), which the
            // source explicitly names -- also just "< 0" to us.
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::OFFSET_FETCH,
                |_| offset_fetch_response_body(-1001, 0),
            )
            .await;
        });

        let lag = collector
            .compute_lag("readiness-reporting", "manpower.assessments", 1)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(lag, 10);
    }

    #[rusty_tokio::test]
    async fn get_throughput_returns_the_high_watermark() {
        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut collector = MetricsCollector::with_client(client);

        let server = rusty_tokio::spawn(async move {
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::LIST_OFFSETS,
                |_| list_offsets_response_body(999, 0),
            )
            .await;
        });

        let throughput = collector
            .get_throughput("manpower.assessments", 0)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(throughput, 999);
    }

    #[rusty_tokio::test]
    async fn watermark_error_code_surfaces_as_kafka_error_code() {
        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut collector = MetricsCollector::with_client(client);

        let server = rusty_tokio::spawn(async move {
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::LIST_OFFSETS,
                |_| {
                    list_offsets_response_body(0, 3) // UNKNOWN_TOPIC_OR_PARTITION
                },
            )
            .await;
        });

        let err = collector
            .get_throughput("no-such-topic", 0)
            .await
            .unwrap_err();
        server.await.unwrap();
        assert!(matches!(err, MetricsError::KafkaErrorCode(3)));
    }

    #[rusty_tokio::test]
    async fn get_product_metrics_combines_lag_throughput_and_violation_count() {
        let conn = seeded_connection();
        record_violation(&conn, "manpower.assessments-value", "bad field").unwrap();

        let (client_io, mut peer) = duplex(4096);
        let client = KafkaClient::new(client_io, None);
        let mut collector = MetricsCollector::with_client(client);

        let server = rusty_tokio::spawn(async move {
            // compute_lag (1 partition)
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::LIST_OFFSETS,
                |_| list_offsets_response_body(20, 0),
            )
            .await;
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::OFFSET_FETCH,
                |_| offset_fetch_response_body(5, 0),
            )
            .await;
            // get_throughput
            respond_once(
                &mut peer,
                rusty_kafka::protocol::api_key::LIST_OFFSETS,
                |_| list_offsets_response_body(20, 0),
            )
            .await;
        });

        let metrics = collector
            .get_product_metrics(
                &conn,
                "readiness-reporting",
                "manpower.assessments",
                1,
                "manpower.assessments-value",
            )
            .await
            .unwrap();
        server.await.unwrap();

        assert_eq!(
            metrics,
            ProductMetrics {
                lag: 15,
                throughput: 20,
                violation_count: 1,
            }
        );
    }
}
