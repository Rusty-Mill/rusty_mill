//! Dashboard monitor endpoints -- the Rust port of
//! `meshed.registry.routers.monitor` (REG-118..135).
//!
//! Three endpoints:
//! - `GET /monitor/topology` (REG-118..124): a producer/processor/
//!   consumer graph derived from registered data products and their
//!   ports, via the app's own [`AppState`] session.
//! - `GET /monitor/events` (REG-125..132): an SSE stream of `lineage_records`
//!   activity. Like `routers::lineage` and `routers::transformation`'s
//!   own SSE endpoint, this one takes no `AppState` -- it opens its own
//!   connection against a hardcoded [`DEFAULT_DB_PATH`] (REG-135: a
//!   *second*, independently-declared copy of the same
//!   `"meshed_registry.db"` literal, not a shared constant -- the
//!   source repeats the module-level default in every router file that
//!   needs it, and the manifest row exists specifically to capture that
//!   duplication as intentional).
//! - `GET /monitor/metrics` (REG-133..135): aggregate counts. Structured
//!   counts (products/ports/contracts/violations) come from the app's
//!   `AppState` session; `lineage_events`/`lineage_records` come from a
//!   second, raw connection against the same hardcoded
//!   [`DEFAULT_DB_PATH`], defaulting to `0` if that query fails
//!   (REG-134) rather than failing the whole response.
//!
//! One thing the source's `_event_generator` does that this port
//! doesn't: after polling `lineage_records`, it also runs a `SELECT
//! ... FROM schema_violations ORDER BY id DESC LIMIT 5` query and binds
//! the result to `violations` -- which is then never read again in the
//! function. That query has no observable effect (nothing is emitted
//! from it, no test could tell it ran from the outside), so it isn't
//! ported; there's nothing to preserve.

use super::detail_error;
use crate::app::AppState;
use crate::http::request::Request;
use crate::http::response::Response;
use crate::http::router::Router;
use rusty_http::StatusCode;
use rusty_request::Json;
use rusty_sqlite::rusqlite::{params, Connection};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

/// The source's `monitor.py` own `_DEFAULT_DB_PATH` module constant
/// (REG-135) -- see the module doc for why this isn't shared with
/// `routers::lineage`/`routers::transformation`'s identical copies.
pub const DEFAULT_DB_PATH: &str = "meshed_registry.db";

fn internal_error() -> Response {
    detail_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
}

fn session_error() -> Response {
    detail_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Database engine is not initialized",
    )
}

// ---------------------------------------------------------------------
// Topology (REG-118..124)
// ---------------------------------------------------------------------

struct ProductRow {
    id: i64,
    name: String,
    domain: String,
    version: String,
}

struct Node {
    id: String,
    node_type: &'static str,
    label: String,
    sub: String,
    x: i64,
    y: i64,
}

struct Edge {
    id: String,
    from: String,
    to: String,
}

fn node_json(node: &Node) -> Json {
    let mut json = Json::object();
    json.insert("id", node.id.as_str());
    json.insert("type", node.node_type);
    json.insert("label", node.label.as_str());
    json.insert("sub", node.sub.as_str());
    json.insert("x", node.x);
    json.insert("y", node.y);
    json
}

fn edge_json(edge: &Edge) -> Json {
    let mut json = Json::object();
    json.insert("id", edge.id.as_str());
    json.insert("from", edge.from.as_str());
    json.insert("to", edge.to.as_str());
    json
}

/// The last dot-delimited segment of `topic`, or `topic` itself if it
/// has no dots (REG-121) -- `str.rsplit('.', 1)`'s Rust equivalent via
/// [`str::rsplit`], whose first item is always the tail segment
/// whether or not a `.` is present.
fn short_topic_label(topic: &str) -> &str {
    topic.rsplit('.').next().unwrap_or(topic)
}

fn fetch_products(conn: &Connection) -> rusty_sqlite::rusqlite::Result<Vec<ProductRow>> {
    let mut stmt =
        conn.prepare("SELECT id, name, domain, version FROM data_products ORDER BY id ASC")?;
    let rows = stmt.query_map([], |row| {
        Ok(ProductRow {
            id: row.get(0)?,
            name: row.get(1)?,
            domain: row.get(2)?,
            version: row.get(3)?,
        })
    })?;
    rows.collect()
}

fn fetch_output_topics(
    conn: &Connection,
    product_id: i64,
) -> rusty_sqlite::rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT topic_name FROM output_ports WHERE data_product_id = ?1 ORDER BY id ASC",
    )?;
    let rows = stmt.query_map(params![product_id], |row| row.get::<_, String>(0))?;
    rows.collect()
}

fn fetch_input_topics(
    conn: &Connection,
    product_id: i64,
) -> rusty_sqlite::rusqlite::Result<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT topic_name FROM input_ports WHERE data_product_id = ?1 ORDER BY id ASC")?;
    let rows = stmt.query_map(params![product_id], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// Adds (or reuses) a broker node for `topic` in `topic_node_ids`,
/// returning its node id -- REG-119's dedup-by-topic-name.
fn broker_node_id(
    topic: &str,
    topic_node_ids: &mut HashMap<String, String>,
    nodes: &mut Vec<Node>,
) -> String {
    if let Some(id) = topic_node_ids.get(topic) {
        return id.clone();
    }
    let topic_id = format!("topic-{}", topic_node_ids.len());
    topic_node_ids.insert(topic.to_string(), topic_id.clone());
    nodes.push(Node {
        id: topic_id.clone(),
        node_type: "broker",
        label: short_topic_label(topic).to_string(),
        sub: topic.to_string(),
        x: 0,
        y: 0,
    });
    topic_id
}

/// Builds the topology graph (REG-118..121): one node per data product
/// (classified producer/processor/consumer, REG-118), one deduplicated
/// broker node per distinct topic name (REG-119), and one edge per
/// port (REG-120) -- product→topic for each output port, topic→product
/// for each input port, in that order, matching the source's own
/// per-product iteration order.
fn build_topology(conn: &Connection) -> rusty_sqlite::rusqlite::Result<(Vec<Node>, Vec<Edge>)> {
    let products = fetch_products(conn)?;

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut topic_node_ids: HashMap<String, String> = HashMap::new();
    let mut edge_counter = 0u64;

    for product in &products {
        let output_topics = fetch_output_topics(conn, product.id)?;
        let input_topics = fetch_input_topics(conn, product.id)?;
        let node_type = match (!input_topics.is_empty(), !output_topics.is_empty()) {
            (true, true) => "processor",
            (false, true) => "producer",
            _ => "consumer",
        };
        let node_id = format!("dp-{}", product.id);
        nodes.push(Node {
            id: node_id.clone(),
            node_type,
            label: product.name.clone(),
            sub: format!("{} · v{}", product.domain, product.version),
            x: 0,
            y: 0,
        });

        for topic in &output_topics {
            let topic_id = broker_node_id(topic, &mut topic_node_ids, &mut nodes);
            edge_counter += 1;
            edges.push(Edge {
                id: format!("e{edge_counter}"),
                from: node_id.clone(),
                to: topic_id,
            });
        }
        for topic in &input_topics {
            let topic_id = broker_node_id(topic, &mut topic_node_ids, &mut nodes);
            edge_counter += 1;
            edges.push(Edge {
                id: format!("e{edge_counter}"),
                from: topic_id,
                to: node_id.clone(),
            });
        }
    }

    Ok((layout_nodes(nodes), edges))
}

/// Assigns SVG coordinates in a 4-column layout (REG-122): producer
/// x=90, broker x=350, processor x=560, consumer x=720; within a
/// column, nodes are spaced evenly across a 500px viewBox with 50px
/// padding on each end, in their original (insertion) order. A lone
/// node in a column sits at `y = 250` (REG-123).
fn layout_nodes(nodes: Vec<Node>) -> Vec<Node> {
    const VIEWBOX_HEIGHT: f64 = 500.0;
    const PADDING: f64 = 50.0;

    let mut producer = Vec::new();
    let mut broker = Vec::new();
    let mut processor = Vec::new();
    let mut consumer = Vec::new();
    for node in nodes {
        match node.node_type {
            "producer" => producer.push(node),
            "broker" => broker.push(node),
            "processor" => processor.push(node),
            "consumer" => consumer.push(node),
            _ => unreachable!("Node::node_type is always one of the four column types"),
        }
    }

    let mut laid_out = Vec::new();
    for (column, x) in [
        (producer, 90),
        (broker, 350),
        (processor, 560),
        (consumer, 720),
    ] {
        let count = column.len();
        if count == 0 {
            continue;
        }
        if count == 1 {
            let mut node = column.into_iter().next().expect("count == 1");
            node.x = x;
            node.y = (VIEWBOX_HEIGHT as i64) / 2;
            laid_out.push(node);
        } else {
            let usable = VIEWBOX_HEIGHT - 2.0 * PADDING;
            let spacing = usable / (count as f64 - 1.0);
            for (i, mut node) in column.into_iter().enumerate() {
                node.x = x;
                node.y = (PADDING + i as f64 * spacing).round() as i64;
                laid_out.push(node);
            }
        }
    }
    laid_out
}

async fn get_topology(state: Arc<AppState>, _req: Request) -> Response {
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    match build_topology(&conn) {
        Ok((nodes, edges)) => {
            let mut nodes_json = Json::array();
            for node in &nodes {
                nodes_json.push(node_json(node));
            }
            let mut edges_json = Json::array();
            for edge in &edges {
                edges_json.push(edge_json(edge));
            }
            let mut body = Json::object();
            body.insert("nodes", nodes_json);
            body.insert("edges", edges_json);
            Response::json(StatusCode::OK, &body)
        }
        Err(_) => internal_error(),
    }
}

// ---------------------------------------------------------------------
// SSE event stream (REG-125..132)
// ---------------------------------------------------------------------

/// One poll cycle's worth of state, carried between
/// [`MonitorEventStream::next_chunk`] calls -- same shape as
/// `routers::transformation::TransformationEventStream`, polling
/// `lineage_records` instead of `transformation_events`. `poll_interval`
/// is a test seam; production always uses REG-126's literal 1.0s.
struct MonitorEventStream {
    db_path: String,
    poll_interval: Duration,
    last_id: i64,
    initialized: bool,
    pending: VecDeque<String>,
}

impl MonitorEventStream {
    fn new(db_path: String) -> Self {
        MonitorEventStream::with_poll_interval(db_path, Duration::from_secs(1))
    }

    fn with_poll_interval(db_path: String, poll_interval: Duration) -> Self {
        MonitorEventStream {
            db_path,
            poll_interval,
            last_id: 0,
            initialized: false,
            pending: VecDeque::new(),
        }
    }

    /// REG-127: on first call only, seeks `last_id = MAX(id)` so only
    /// records newer than connect-time stream -- defaults to 0 (full
    /// history) on any error, including a missing table/db.
    fn seed_last_id(&mut self) {
        self.initialized = true;
        let Ok(conn) = Connection::open(&self.db_path) else {
            return;
        };
        if let Ok(max_id) = conn.query_row("SELECT MAX(id) FROM lineage_records", [], |row| {
            row.get::<_, Option<i64>>(0)
        }) {
            self.last_id = max_id.unwrap_or(0);
        }
    }

    /// REG-128/130: polls for up to 50 new rows past `last_id`;
    /// swallows all sqlite errors silently (an empty poll, not a
    /// stream failure) rather than propagating them.
    fn poll(&mut self) {
        let Ok(conn) = Connection::open(&self.db_path) else {
            return;
        };
        let Ok(mut stmt) = conn.prepare(
            "SELECT id, event_id, product_name, topic_name, event_timestamp \
             FROM lineage_records WHERE id > ?1 ORDER BY id ASC LIMIT 50",
        ) else {
            return;
        };
        let Ok(rows) = stmt.query_map(params![self.last_id], |row| {
            let id: i64 = row.get(0)?;
            let event_id: String = row.get(1)?;
            let product_name: String = row.get(2)?;
            let topic_name: String = row.get(3)?;
            let event_timestamp: String = row.get(4)?;
            Ok((id, event_id, product_name, topic_name, event_timestamp))
        }) else {
            return;
        };

        for row in rows.flatten() {
            let (id, event_id, product_name, topic_name, event_timestamp) = row;
            self.last_id = id;
            // REG-132: payload shape, lat/kb/isErr hardcoded.
            let mut json = Json::object();
            json.insert("type", format!("{product_name}.publish").as_str());
            json.insert("from", product_name.as_str());
            json.insert("fromType", "producer");
            json.insert("to", short_topic_label(&topic_name));
            json.insert("toType", "broker");
            json.insert("lat", 0);
            json.insert("kb", 0);
            json.insert("isErr", false);
            json.insert("eventId", event_id.as_str());
            json.insert("timestamp", event_timestamp.as_str());
            self.pending
                .push_back(format!("data: {}\n\n", json.to_json_string()));
        }
    }

    /// Produces the next SSE chunk, polling (and sleeping
    /// `poll_interval` once the current batch is drained) exactly as
    /// the source's `while True` generator body does: query, yield
    /// every matched row (REG-131), or one heartbeat if none (REG-129),
    /// then sleep once per cycle regardless of which happened.
    async fn next_chunk(&mut self) -> String {
        if !self.initialized {
            self.seed_last_id();
        }

        if self.pending.is_empty() {
            self.poll();
            if self.pending.is_empty() {
                rusty_tokio::time::sleep(self.poll_interval).await;
                return ": heartbeat\n\n".to_string();
            }
        }

        let chunk = self.pending.pop_front().expect("checked non-empty above");
        if self.pending.is_empty() {
            rusty_tokio::time::sleep(self.poll_interval).await;
        }
        chunk
    }
}

async fn stream_events(_req: Request) -> Response {
    let stream = Arc::new(rusty_tokio::sync::Mutex::new(MonitorEventStream::new(
        DEFAULT_DB_PATH.to_string(),
    )));
    Response::sse(Box::new(move || {
        let stream = stream.clone();
        Box::pin(async move {
            let mut guard = stream.lock_owned().await;
            guard.next_chunk().await
        })
    }))
}

// ---------------------------------------------------------------------
// Aggregate metrics (REG-133..135)
// ---------------------------------------------------------------------

struct MonitorCounts {
    data_products: i64,
    input_ports: i64,
    output_ports: i64,
    contracts: i64,
    schema_violations: i64,
}

fn fetch_monitor_counts(conn: &Connection) -> rusty_sqlite::rusqlite::Result<MonitorCounts> {
    let count = |table: &str| -> rusty_sqlite::rusqlite::Result<i64> {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
    };
    Ok(MonitorCounts {
        data_products: count("data_products")?,
        input_ports: count("input_ports")?,
        output_ports: count("output_ports")?,
        contracts: count("data_contracts")?,
        schema_violations: count("schema_violations")?,
    })
}

/// `lineage_events`/`lineage_records` counts from a raw connection
/// against `db_path`, defaulting to `(0, 0)` on any failure (REG-134)
/// -- a missing/unreadable database degrades the snapshot rather than
/// failing the whole response, matching the source's `except Exception:
/// pass` around this specific pair of queries only.
fn fetch_lineage_counts(db_path: &str) -> (i64, i64) {
    (|| -> rusty_sqlite::rusqlite::Result<(i64, i64)> {
        let conn = Connection::open(db_path)?;
        let events: i64 =
            conn.query_row("SELECT COUNT(*) FROM lineage_events", [], |row| row.get(0))?;
        let records: i64 =
            conn.query_row("SELECT COUNT(*) FROM lineage_records", [], |row| row.get(0))?;
        Ok((events, records))
    })()
    .unwrap_or((0, 0))
}

/// Builds the endpoint's response from an already-open `conn` and a
/// resolved lineage DB path -- factored out from [`get_metrics`] so
/// tests can pass a temp path for the lineage counts instead of the
/// real [`DEFAULT_DB_PATH`] (dispatching through the router would
/// otherwise create/open the literal `"meshed_registry.db"` file
/// relative to wherever the test runs, the stray-file pitfall
/// `routers::lineage`'s module doc calls out).
fn build_metrics_response(conn: &Connection, lineage_db_path: &str) -> Response {
    let counts = match fetch_monitor_counts(conn) {
        Ok(counts) => counts,
        Err(_) => return internal_error(),
    };
    let (lineage_events, lineage_records) = fetch_lineage_counts(lineage_db_path);

    let mut json = Json::object();
    json.insert("data_products", counts.data_products);
    json.insert("input_ports", counts.input_ports);
    json.insert("output_ports", counts.output_ports);
    json.insert("contracts", counts.contracts);
    json.insert("schema_violations", counts.schema_violations);
    json.insert("lineage_events", lineage_events);
    json.insert("lineage_records", lineage_records);
    json.insert("total_flows", counts.input_ports + counts.output_ports);
    Response::json(StatusCode::OK, &json)
}

async fn get_metrics(state: Arc<AppState>, _req: Request) -> Response {
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    build_metrics_response(&conn, DEFAULT_DB_PATH)
}

/// Builds the `/monitor` router. `topology`/`metrics` are bound to
/// `state`; `events` isn't -- see the module doc for why.
pub fn router(state: Arc<AppState>) -> Router {
    let s = state.clone();
    let router = Router::new().get("/monitor/topology", move |req| {
        let state = s.clone();
        async move { get_topology(state, req).await }
    });

    let router = router.get("/monitor/metrics", move |req| {
        let state = state.clone();
        async move { get_metrics(state, req).await }
    });

    router.get(
        "/monitor/events",
        |req| async move { stream_events(req).await },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::request::Request as HttpRequest;
    use rusty_http::{HeaderMap, Method};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

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
            "rusty_meshed_monitor_test_{}_{n}.db",
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

    fn create_product(state: &Arc<AppState>, name: &str, domain: &str, version: &str) -> i64 {
        let conn = state.get_session().unwrap();
        conn.execute(
            "INSERT INTO data_products (name, owner, version, domain, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![name, "team-a", version, domain, "desc"],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn add_output_port(state: &Arc<AppState>, product_id: i64, topic_name: &str) {
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

    fn add_input_port(state: &Arc<AppState>, product_id: i64, topic_name: &str) {
        let conn = state.get_session().unwrap();
        conn.execute(
            "INSERT INTO input_ports (data_product_id, topic_name) VALUES (?1, ?2)",
            params![product_id, topic_name],
        )
        .unwrap();
    }

    // ---- Topology ----

    #[rusty_tokio::test]
    async fn empty_registry_returns_empty_nodes_and_edges() {
        let state = temp_state();
        let response = router((*state).clone())
            .dispatch(req("/monitor/topology".to_string()))
            .await;
        assert_eq!(response.status, StatusCode::OK);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert!(json.get("nodes").unwrap().as_array().unwrap().is_empty());
        assert!(json.get("edges").unwrap().as_array().unwrap().is_empty());
    }

    #[rusty_tokio::test]
    async fn classifies_products_as_producer_processor_or_consumer() {
        let state = temp_state();
        let producer = create_product(&state, "producer-only", "commerce", "1.0.0");
        add_output_port(&state, producer, "commerce.orders");
        let consumer = create_product(&state, "consumer-only", "commerce", "1.0.0");
        add_input_port(&state, consumer, "commerce.orders");
        let processor = create_product(&state, "both", "commerce", "1.0.0");
        add_output_port(&state, processor, "commerce.enriched");
        add_input_port(&state, processor, "commerce.raw");
        let idle = create_product(&state, "idle", "commerce", "1.0.0");
        let _ = idle;

        let response = router((*state).clone())
            .dispatch(req("/monitor/topology".to_string()))
            .await;
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        let nodes = json.get("nodes").unwrap().as_array().unwrap();

        let type_of = |label: &str| -> String {
            nodes
                .iter()
                .find(|n| n.get("label").unwrap().as_str() == Some(label))
                .unwrap()
                .get("type")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(type_of("producer-only"), "producer");
        assert_eq!(type_of("consumer-only"), "consumer");
        assert_eq!(type_of("both"), "processor");
        assert_eq!(type_of("idle"), "consumer");
    }

    #[rusty_tokio::test]
    async fn dedupes_broker_nodes_sharing_a_topic_name() {
        let state = temp_state();
        let producer = create_product(&state, "producer", "commerce", "1.0.0");
        add_output_port(&state, producer, "commerce.orders");
        let consumer = create_product(&state, "consumer", "commerce", "1.0.0");
        add_input_port(&state, consumer, "commerce.orders");

        let response = router((*state).clone())
            .dispatch(req("/monitor/topology".to_string()))
            .await;
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        let nodes = json.get("nodes").unwrap().as_array().unwrap();
        let broker_nodes: Vec<_> = nodes
            .iter()
            .filter(|n| n.get("type").unwrap().as_str() == Some("broker"))
            .collect();
        assert_eq!(broker_nodes.len(), 1);
        assert_eq!(
            broker_nodes[0].get("label").unwrap().as_str(),
            Some("orders")
        );
        assert_eq!(
            broker_nodes[0].get("sub").unwrap().as_str(),
            Some("commerce.orders")
        );
    }

    #[rusty_tokio::test]
    async fn creates_one_edge_per_port() {
        let state = temp_state();
        let producer = create_product(&state, "producer", "commerce", "1.0.0");
        add_output_port(&state, producer, "commerce.orders");
        let consumer = create_product(&state, "consumer", "commerce", "1.0.0");
        add_input_port(&state, consumer, "commerce.orders");

        let response = router((*state).clone())
            .dispatch(req("/monitor/topology".to_string()))
            .await;
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        let edges = json.get("edges").unwrap().as_array().unwrap();
        assert_eq!(edges.len(), 2);
    }

    #[rusty_tokio::test]
    async fn a_lone_node_in_a_column_sits_at_y_250() {
        let state = temp_state();
        let producer = create_product(&state, "solo-producer", "commerce", "1.0.0");
        add_output_port(&state, producer, "commerce.orders");

        let response = router((*state).clone())
            .dispatch(req("/monitor/topology".to_string()))
            .await;
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        let nodes = json.get("nodes").unwrap().as_array().unwrap();
        let producer_node = nodes
            .iter()
            .find(|n| n.get("type").unwrap().as_str() == Some("producer"))
            .unwrap();
        assert_eq!(producer_node.get("x").unwrap().as_f64(), Some(90.0));
        assert_eq!(producer_node.get("y").unwrap().as_f64(), Some(250.0));
    }

    #[rusty_tokio::test]
    async fn multiple_nodes_in_a_column_are_spaced_evenly() {
        let state = temp_state();
        for name in ["a", "b", "c"] {
            let p = create_product(&state, name, "commerce", "1.0.0");
            add_output_port(&state, p, format!("commerce.{name}").as_str());
        }

        let response = router((*state).clone())
            .dispatch(req("/monitor/topology".to_string()))
            .await;
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        let nodes = json.get("nodes").unwrap().as_array().unwrap();
        let mut ys: Vec<i64> = nodes
            .iter()
            .filter(|n| n.get("type").unwrap().as_str() == Some("producer"))
            .map(|n| n.get("y").unwrap().as_f64().unwrap() as i64)
            .collect();
        ys.sort_unstable();
        assert_eq!(ys, vec![50, 250, 450]);
    }

    // ---- Metrics ----

    #[test]
    fn metrics_counts_products_ports_and_contracts() {
        let state = temp_state();
        let product = create_product(&state, "orders", "commerce", "1.0.0");
        add_output_port(&state, product, "commerce.orders");
        add_input_port(&state, product, "commerce.raw");
        let conn = state.get_session().unwrap();

        let response = build_metrics_response(&conn, "/no/such/directory/exists/db.sqlite");
        assert_eq!(response.status, StatusCode::OK);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(json.get("data_products").unwrap().as_f64(), Some(1.0));
        assert_eq!(json.get("input_ports").unwrap().as_f64(), Some(1.0));
        assert_eq!(json.get("output_ports").unwrap().as_f64(), Some(1.0));
        assert_eq!(json.get("total_flows").unwrap().as_f64(), Some(2.0));
        assert_eq!(json.get("schema_violations").unwrap().as_f64(), Some(0.0));
        assert_eq!(json.get("lineage_events").unwrap().as_f64(), Some(0.0));
        assert_eq!(json.get("lineage_records").unwrap().as_f64(), Some(0.0));
    }

    // No router-level test drives /monitor/metrics against the real
    // DEFAULT_DB_PATH ("meshed_registry.db", relative to the process
    // cwd) -- doing so would create (or open) that literal file in
    // whatever directory the test runs from, the same stray-file
    // pitfall `routers::lineage`'s module doc calls out. REG-134's
    // "defaults to 0 on a query failure" behavior is exercised directly
    // against fetch_lineage_counts below instead.
    #[test]
    fn fetch_lineage_counts_defaults_to_zero_for_a_missing_directory() {
        let counts = fetch_lineage_counts("/no/such/directory/exists/db.sqlite");
        assert_eq!(counts, (0, 0));
    }

    #[test]
    fn default_db_path_matches_the_source_constant() {
        assert_eq!(DEFAULT_DB_PATH, "meshed_registry.db");
    }

    // ---- SSE events ----

    fn temp_db_path() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "rusty_meshed_monitor_events_test_{}_{n}.db",
            std::process::id()
        ))
    }

    fn ensure_lineage_schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS lineage_records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL,
                correlation_id TEXT NOT NULL,
                source_event_ids TEXT NOT NULL,
                product_name TEXT NOT NULL,
                topic_name TEXT NOT NULL,
                event_timestamp TEXT NOT NULL
            );",
        )
        .unwrap();
    }

    #[rusty_tokio::test]
    async fn event_stream_emits_a_heartbeat_with_no_events() {
        let path = temp_db_path();
        let conn = Connection::open(&path).unwrap();
        ensure_lineage_schema(&conn);
        drop(conn);

        let mut stream = MonitorEventStream::with_poll_interval(
            path.to_str().unwrap().to_string(),
            Duration::from_millis(1),
        );
        let chunk = stream.next_chunk().await;
        assert_eq!(chunk, ": heartbeat\n\n");
        let _ = std::fs::remove_file(&path);
    }

    #[rusty_tokio::test]
    async fn event_stream_only_yields_records_newer_than_connect_time() {
        let path = temp_db_path();
        let conn = Connection::open(&path).unwrap();
        ensure_lineage_schema(&conn);
        conn.execute(
            "INSERT INTO lineage_records (event_id, correlation_id, source_event_ids, product_name, topic_name, event_timestamp) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["e0", "corr-0", "[]", "orders", "commerce.orders", "2026-01-01T00:00:00Z"],
        )
        .unwrap();

        let mut stream = MonitorEventStream::with_poll_interval(
            path.to_str().unwrap().to_string(),
            Duration::from_millis(1),
        );
        stream.seed_last_id();

        conn.execute(
            "INSERT INTO lineage_records (event_id, correlation_id, source_event_ids, product_name, topic_name, event_timestamp) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["e1", "corr-1", "[]", "orders", "commerce.orders", "2026-01-01T00:00:01Z"],
        )
        .unwrap();
        drop(conn);

        let chunk = stream.next_chunk().await;
        assert!(chunk.starts_with("data: "));
        assert!(chunk.ends_with("\n\n"));
        assert!(chunk.contains("\"eventId\":\"e1\""));
        assert!(!chunk.contains("\"eventId\":\"e0\""));
        let _ = std::fs::remove_file(&path);
    }

    #[rusty_tokio::test]
    async fn event_stream_payload_matches_the_expected_shape() {
        let path = temp_db_path();
        let conn = Connection::open(&path).unwrap();
        ensure_lineage_schema(&conn);

        let mut stream = MonitorEventStream::with_poll_interval(
            path.to_str().unwrap().to_string(),
            Duration::from_millis(1),
        );
        stream.seed_last_id();

        conn.execute(
            "INSERT INTO lineage_records (event_id, correlation_id, source_event_ids, product_name, topic_name, event_timestamp) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["e1", "corr-1", "[]", "orders", "commerce.orders.assessments", "2026-01-01T00:00:01Z"],
        )
        .unwrap();
        drop(conn);

        let chunk = stream.next_chunk().await;
        let payload = chunk
            .strip_prefix("data: ")
            .unwrap()
            .strip_suffix("\n\n")
            .unwrap();
        let json = Json::parse(payload).unwrap();
        assert_eq!(json.get("type").unwrap().as_str(), Some("orders.publish"));
        assert_eq!(json.get("from").unwrap().as_str(), Some("orders"));
        assert_eq!(json.get("fromType").unwrap().as_str(), Some("producer"));
        assert_eq!(json.get("to").unwrap().as_str(), Some("assessments"));
        assert_eq!(json.get("toType").unwrap().as_str(), Some("broker"));
        assert_eq!(json.get("lat").unwrap().as_f64(), Some(0.0));
        assert_eq!(json.get("kb").unwrap().as_f64(), Some(0.0));
        assert_eq!(json.get("isErr").unwrap().as_bool(), Some(false));
        assert_eq!(json.get("eventId").unwrap().as_str(), Some("e1"));
        assert_eq!(
            json.get("timestamp").unwrap().as_str(),
            Some("2026-01-01T00:00:01Z")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[rusty_tokio::test]
    async fn event_stream_fetches_at_most_fifty_rows_per_poll() {
        let path = temp_db_path();
        let conn = Connection::open(&path).unwrap();
        ensure_lineage_schema(&conn);
        for i in 0..60 {
            conn.execute(
                "INSERT INTO lineage_records (event_id, correlation_id, source_event_ids, product_name, topic_name, event_timestamp) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![format!("e{i}"), "corr", "[]", "orders", "commerce.orders", "2026-01-01T00:00:00Z"],
            )
            .unwrap();
        }
        drop(conn);

        let mut stream = MonitorEventStream::with_poll_interval(
            path.to_str().unwrap().to_string(),
            Duration::from_millis(1),
        );
        // last_id defaults to 0 (fresh stream, never seeded) so every
        // row is "new" -- the cap is exercised on the very first poll.
        stream.poll();
        assert_eq!(stream.pending.len(), 50);
        let _ = std::fs::remove_file(&path);
    }

    #[rusty_tokio::test]
    async fn event_stream_defaults_last_id_to_zero_when_db_missing() {
        let path = temp_db_path();
        let mut stream = MonitorEventStream::with_poll_interval(
            path.to_str().unwrap().to_string(),
            Duration::from_millis(1),
        );
        stream.seed_last_id();
        assert_eq!(stream.last_id, 0);
        let _ = std::fs::remove_file(&path);
    }

    #[rusty_tokio::test]
    async fn event_stream_polling_a_missing_table_does_not_panic_or_error() {
        // No lineage_records table at all -- poll() must swallow the
        // sqlite error and simply produce no pending events (REG-130).
        let path = temp_db_path();
        Connection::open(&path).unwrap();

        let mut stream = MonitorEventStream::with_poll_interval(
            path.to_str().unwrap().to_string(),
            Duration::from_millis(1),
        );
        stream.poll();
        assert!(stream.pending.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[rusty_tokio::test]
    async fn sse_route_sets_the_expected_headers() {
        let response = router(Arc::new(AppState::new()))
            .dispatch(req("/monitor/events".to_string()))
            .await;
        assert_eq!(
            response.headers.get("Content-Type"),
            Some("text/event-stream")
        );
        assert_eq!(response.headers.get("Cache-Control"), Some("no-cache"));
        assert_eq!(response.headers.get("Connection"), Some("keep-alive"));
        assert_eq!(response.headers.get("X-Accel-Buffering"), Some("no"));
    }
}
