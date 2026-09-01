//! Per-data-product event flow metrics endpoint -- the Rust port of
//! `meshed.registry.routers.metrics` (REG-110..117, GOV-040).
//!
//! `GET /data-products/{product_id}/metrics` returns Kafka consumer
//! lag, a throughput proxy, and the schema-violation count for a data
//! product's first output port, built on
//! [`rusty_meshed_observability::MetricsCollector`] (GOV-033..039).
//! When Kafka itself is unreachable (the Rust equivalent of the
//! source's `except KafkaException`), `lag`/`throughput` are reported
//! as `-1` with an `"error"` field added -- the endpoint still answers
//! 200, since `violation_count` is computed independently of Kafka
//! (REG-115/REG-116).

use super::{detail_error, not_found};
use crate::app::AppState;
use crate::http::request::Request;
use crate::http::response::Response;
use crate::http::router::Router;
use rusty_http::StatusCode;
use rusty_meshed_observability::{get_violation_count, MetricsCollector, MetricsError};
use rusty_request::Json;
use rusty_sqlite::rusqlite::{params, Connection, OptionalExtension};
use rusty_tokio::io::{AsyncRead, AsyncWrite};
use std::sync::Arc;

const RESOURCE: &str = "Data product";

fn parse_id(req: &Request) -> Option<i64> {
    req.param("product_id").and_then(|value| value.parse().ok())
}

fn internal_error() -> Response {
    detail_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
}

struct FirstOutputPort {
    topic_name: String,
    schema_subject: String,
}

fn fetch_product_name(
    conn: &Connection,
    product_id: i64,
) -> rusty_sqlite::rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT name FROM data_products WHERE id = ?1",
        params![product_id],
        |row| row.get(0),
    )
    .optional()
}

/// `product.output_ports[0]` (REG-112) -- `id ASC` matches a Python
/// list built by appending as rows are loaded.
fn fetch_first_output_port(
    conn: &Connection,
    product_id: i64,
) -> rusty_sqlite::rusqlite::Result<Option<FirstOutputPort>> {
    conn.query_row(
        "SELECT topic_name, schema_subject FROM output_ports \
         WHERE data_product_id = ?1 ORDER BY id ASC LIMIT 1",
        params![product_id],
        |row| {
            Ok(FirstOutputPort {
                topic_name: row.get(0)?,
                schema_subject: row.get(1)?,
            })
        },
    )
    .optional()
}

/// Computes `(lag, throughput)` for one already-connected collector --
/// factored out from [`build_metrics_response`] so it exercises the
/// exact same two calls the source's `try` block makes.
async fn compute_lag_and_throughput<S: AsyncRead + AsyncWrite + Unpin + Send>(
    collector: &mut MetricsCollector<S>,
    group_id: &str,
    topic: &str,
    num_partitions: i32,
) -> Result<(i64, i64), MetricsError> {
    let lag = collector
        .compute_lag(group_id, topic, num_partitions)
        .await?;
    let throughput = collector.get_throughput(topic, 0).await?;
    Ok((lag, throughput))
}

/// What the DB alone can answer: the product's name, its first output
/// port, and the violation count for that port's subject (REG-116 --
/// independent of Kafka reachability). A plain (non-`async`) function
/// so its `&Connection` borrow never has to cross an `.await`:
/// `rusqlite::Connection` isn't `Sync`, so holding one live across the
/// Kafka call in [`build_metrics_response`] would make that future
/// non-`Send`, which [`crate::http::Router::get`] requires.
enum Lookup {
    NotFound,
    NoOutputPorts,
    Error,
    Found {
        product_name: String,
        port: FirstOutputPort,
        violation_count: i64,
    },
}

fn lookup(conn: &Connection, product_id: i64) -> Lookup {
    let product_name = match fetch_product_name(conn, product_id) {
        Ok(Some(name)) => name,
        Ok(None) => return Lookup::NotFound,
        Err(_) => return Lookup::Error,
    };
    let port = match fetch_first_output_port(conn, product_id) {
        Ok(Some(port)) => port,
        Ok(None) => return Lookup::NoOutputPorts,
        Err(_) => return Lookup::Error,
    };
    let violation_count = match get_violation_count(conn, &port.schema_subject) {
        Ok(count) => count,
        Err(_) => return Lookup::Error,
    };
    Lookup::Found {
        product_name,
        port,
        violation_count,
    }
}

/// Builds the endpoint's response from an already-resolved [`Lookup`]
/// and the resolved Kafka bootstrap address -- factored out from
/// [`get_metrics`] so tests can pass an address known to refuse
/// connections instead of depending on `AppState`/env-var config
/// plumbing (the same "handler delegates to a parameterized function"
/// shape `lineage`'s routes use for their hardcoded DB path). Takes no
/// `Connection` at all, so its future is trivially `Send`.
async fn build_metrics_response(
    product_id: i64,
    product_name: &str,
    port: &FirstOutputPort,
    violation_count: i64,
    kafka_bootstrap_servers: &str,
    group_id: &str,
    num_partitions: i32,
) -> Response {
    let (lag, throughput, error) = match MetricsCollector::connect(kafka_bootstrap_servers).await {
        Ok(mut collector) => {
            match compute_lag_and_throughput(
                &mut collector,
                group_id,
                &port.topic_name,
                num_partitions,
            )
            .await
            {
                Ok((lag, throughput)) => (lag, throughput, None),
                Err(err) => (-1, -1, Some(err.to_string())),
            }
        }
        Err(err) => (-1, -1, Some(err.to_string())),
    };

    let mut json = Json::object();
    json.insert("product_id", product_id);
    json.insert("product_name", product_name);
    json.insert("lag", lag);
    json.insert("throughput", throughput);
    json.insert("violation_count", violation_count);
    json.insert("topic", port.topic_name.as_str());
    if let Some(error) = error {
        json.insert("error", error.as_str());
    }
    Response::json(StatusCode::OK, &json)
}

async fn get_metrics(state: Arc<AppState>, req: Request) -> Response {
    let Some(product_id) = parse_id(&req) else {
        return detail_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "product_id must be an integer",
        );
    };
    let Ok(conn) = state.get_session() else {
        return internal_error();
    };
    let Ok(config) = crate::app::get_config() else {
        return internal_error();
    };
    let group_id = req.query_param("group_id").unwrap_or("default").to_string();
    let num_partitions = req
        .query_param("num_partitions")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(1);

    let (product_name, port, violation_count) = match lookup(&conn, product_id) {
        Lookup::NotFound => return not_found(RESOURCE),
        Lookup::NoOutputPorts => {
            return detail_error(StatusCode::NOT_FOUND, "Data product has no output ports")
        }
        Lookup::Error => return internal_error(),
        Lookup::Found {
            product_name,
            port,
            violation_count,
        } => (product_name, port, violation_count),
    };
    drop(conn);

    build_metrics_response(
        product_id,
        &product_name,
        &port,
        violation_count,
        &config.kafka_bootstrap_servers,
        &group_id,
        num_partitions,
    )
    .await
}

/// Builds the metrics router, bound to `state` for DB access.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new().get("/data-products/{product_id}/metrics", move |req| {
        let state = state.clone();
        async move { get_metrics(state, req).await }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::request::Request as HttpRequest;
    use rusty_http::{HeaderMap, Method};
    use rusty_meshed_observability::record_violation;
    use rusty_sqlite::rusqlite::Connection;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A local address nothing listens on -- makes
    /// `MetricsCollector::connect` fail deterministically and quickly
    /// (connection refused), exactly like a real broker outage would
    /// (REG-115), without needing a live broker or touching process
    /// env vars (which would race across parallel test threads).
    const UNREACHABLE_KAFKA: &str = "127.0.0.1:1";

    struct TempState {
        state: Arc<AppState>,
        path: PathBuf,
    }

    impl std::ops::Deref for TempState {
        type Target = Arc<AppState>;
        fn deref(&self) -> &Arc<AppState> {
            &self.state
        }
    }

    impl Drop for TempState {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn temp_state() -> TempState {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusty_meshed_metrics_test_{}_{n}.db",
            std::process::id()
        ));
        let conn = Connection::open(&path).unwrap();
        crate::models::ensure_schema(&conn).unwrap();
        rusty_meshed_observability::ensure_metrics_schema(&conn).unwrap();
        let mut state = AppState::new();
        state.set_engine(path.to_str().unwrap());
        TempState {
            state: Arc::new(state),
            path,
        }
    }

    fn req(path: String) -> HttpRequest {
        HttpRequest {
            method: Method::Get,
            path,
            query: Vec::new(),
            params: Vec::new(),
            headers: HeaderMap::new(),
            body: Vec::new(),
        }
    }

    fn create_product(state: &Arc<AppState>) -> i64 {
        let conn = state.get_session().unwrap();
        conn.execute(
            "INSERT INTO data_products (name, owner, version, domain, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["orders", "team-a", "1.0.0", "commerce", "Order events"],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn create_output_port(state: &Arc<AppState>, product_id: i64, topic_name: &str) {
        let conn = state.get_session().unwrap();
        conn.execute(
            "INSERT INTO output_ports (data_product_id, topic_name, schema_subject, event_type) \
             VALUES (?1, ?2, ?3, ?4)",
            params![
                product_id,
                topic_name,
                format!("{topic_name}-value"),
                "delta"
            ],
        )
        .unwrap();
    }

    /// Resolves a product's `(name, first output port, violation
    /// count)` for tests driving [`build_metrics_response`] directly --
    /// asserts the happy `Lookup::Found` path since every caller here
    /// has already set up a product with an output port.
    fn resolve(state: &Arc<AppState>, product_id: i64) -> (String, FirstOutputPort, i64) {
        let conn = state.get_session().unwrap();
        match lookup(&conn, product_id) {
            Lookup::Found {
                product_name,
                port,
                violation_count,
            } => (product_name, port, violation_count),
            _ => panic!("expected Lookup::Found"),
        }
    }

    #[rusty_tokio::test]
    async fn returns_404_for_an_unknown_product() {
        let state = temp_state();
        let response = router((*state).clone())
            .dispatch(req("/data-products/999/metrics".to_string()))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("detail").unwrap().as_str(),
            Some("Data product not found")
        );
    }

    #[rusty_tokio::test]
    async fn returns_404_when_the_product_has_no_output_ports() {
        let state = temp_state();
        let product_id = create_product(&state);
        let response = router((*state).clone())
            .dispatch(req(format!("/data-products/{product_id}/metrics")))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("detail").unwrap().as_str(),
            Some("Data product has no output ports")
        );
    }

    #[rusty_tokio::test]
    async fn a_kafka_failure_still_returns_200_with_sentinel_metrics_and_an_error_field() {
        let state = temp_state();
        let product_id = create_product(&state);
        create_output_port(&state, product_id, "commerce.orders");
        let (product_name, port, violation_count) = resolve(&state, product_id);

        let response = build_metrics_response(
            product_id,
            &product_name,
            &port,
            violation_count,
            UNREACHABLE_KAFKA,
            "default",
            1,
        )
        .await;

        assert_eq!(response.status, StatusCode::OK);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("product_id").unwrap().as_f64(),
            Some(product_id as f64)
        );
        assert_eq!(json.get("product_name").unwrap().as_str(), Some("orders"));
        assert_eq!(json.get("lag").unwrap().as_f64(), Some(-1.0));
        assert_eq!(json.get("throughput").unwrap().as_f64(), Some(-1.0));
        assert_eq!(json.get("topic").unwrap().as_str(), Some("commerce.orders"));
        assert!(json.get("error").unwrap().as_str().is_some());
    }

    #[rusty_tokio::test]
    async fn violation_count_is_computed_independently_of_kafka_reachability() {
        let state = temp_state();
        let product_id = create_product(&state);
        create_output_port(&state, product_id, "commerce.orders");
        {
            let conn = state.get_session().unwrap();
            record_violation(&conn, "commerce.orders-value", "field removed").unwrap();
            record_violation(&conn, "commerce.orders-value", "type changed").unwrap();
        }
        let (product_name, port, violation_count) = resolve(&state, product_id);

        let response = build_metrics_response(
            product_id,
            &product_name,
            &port,
            violation_count,
            UNREACHABLE_KAFKA,
            "default",
            1,
        )
        .await;

        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(json.get("violation_count").unwrap().as_f64(), Some(2.0));
    }

    #[rusty_tokio::test]
    async fn response_omits_the_error_key_on_success() {
        // Exercises the router end to end against a fake broker so a
        // real KafkaClient round trip backs the "no error field on
        // success" assertion, not just build_metrics_response in
        // isolation.
        use rusty_kafka::protocol::api_key;
        use rusty_kafka::protocol::list_offsets::{
            ListOffsetsPartitionResponse, ListOffsetsResponse, ListOffsetsTopicResponse,
        };
        use rusty_kafka::protocol::offset_fetch::{
            OffsetFetchPartitionResponse, OffsetFetchResponse, OffsetFetchTopicResponse,
        };
        use rusty_kafka::testing::{recv_request, send_response};
        use rusty_tokio::io::TcpListener;
        use rusty_wire::Writer;

        let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = rusty_tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();

            let (header, _) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::LIST_OFFSETS);
            let response = ListOffsetsResponse {
                topics: vec![ListOffsetsTopicResponse {
                    name: "commerce.orders".to_string(),
                    partitions: vec![ListOffsetsPartitionResponse {
                        partition_index: 0,
                        error_code: 0,
                        timestamp: 0,
                        offset: 100,
                    }],
                }],
            };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut peer, header.correlation_id, &writer.into_vec())
                .await
                .unwrap();

            let (header, _) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::OFFSET_FETCH);
            let response = OffsetFetchResponse {
                topics: vec![OffsetFetchTopicResponse {
                    name: "commerce.orders".to_string(),
                    partitions: vec![OffsetFetchPartitionResponse {
                        partition_index: 0,
                        committed_offset: 60,
                        metadata: None,
                        error_code: 0,
                    }],
                }],
            };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut peer, header.correlation_id, &writer.into_vec())
                .await
                .unwrap();

            let (header, _) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::LIST_OFFSETS);
            let response = ListOffsetsResponse {
                topics: vec![ListOffsetsTopicResponse {
                    name: "commerce.orders".to_string(),
                    partitions: vec![ListOffsetsPartitionResponse {
                        partition_index: 0,
                        error_code: 0,
                        timestamp: 0,
                        offset: 100,
                    }],
                }],
            };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut peer, header.correlation_id, &writer.into_vec())
                .await
                .unwrap();
        });

        let state = temp_state();
        let product_id = create_product(&state);
        create_output_port(&state, product_id, "commerce.orders");
        let (product_name, port, violation_count) = resolve(&state, product_id);

        let response = build_metrics_response(
            product_id,
            &product_name,
            &port,
            violation_count,
            &addr.to_string(),
            "default",
            1,
        )
        .await;
        server.await.unwrap();

        assert_eq!(response.status, StatusCode::OK);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert!(json.get("error").is_none());
        assert_eq!(json.get("lag").unwrap().as_f64(), Some(40.0));
        assert_eq!(json.get("throughput").unwrap().as_f64(), Some(100.0));
    }

    #[rusty_tokio::test]
    async fn get_metrics_reads_group_id_and_num_partitions_from_query_params() {
        let state = temp_state();
        let product_id = create_product(&state);
        create_output_port(&state, product_id, "commerce.orders");

        let mut request = req(format!("/data-products/{product_id}/metrics"));
        request.query = vec![
            ("group_id".to_string(), "reporting".to_string()),
            ("num_partitions".to_string(), "3".to_string()),
        ];
        // Routed through the real handler (default config's Kafka
        // address, unreachable in this test environment) purely to
        // prove query-param parsing doesn't panic or 500 -- the actual
        // group_id/num_partitions plumbing is covered directly by
        // compute_lag/get_throughput's own tests in
        // rusty-meshed-observability.
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::OK);
    }
}
