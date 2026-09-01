//! `meshed slo` -- the Rust port of `meshed.cli.commands.slo`
//! (CLI-026..042): SLO compliance status (freshness, completeness,
//! schema conformance) for a data product's output ports, publishing
//! an [`SLOViolationPayload`] for each failing dimension.
//!
//! Unblocked now that `rusty_kafka`'s `Produce` support (landed for
//! GOV-047..049's `SLOViolationPublisher`) exists -- see [`crate::app`]'s
//! own module doc for why this command was deferred until now.
//!
//! # One connect-time check replaces two per-dimension `try`/`except`s
//!
//! The source wraps each of `check_freshness()`/`check_completeness()`
//! in its own `try`/`except`, reporting `"unavailable"` on whatever
//! exception a stalled/unreachable broker raises through
//! `confluent-kafka`'s lazily-connecting `AdminClient`. This port's
//! [`SLOMonitor::connect`] connects *eagerly*, and once connected,
//! [`SLOMonitor::check_freshness`]/[`SLOMonitor::check_completeness`]
//! can't fail at all -- every Kafka-level failure they might hit
//! already collapses internally to `actual_value = f64::INFINITY`
//! (see that crate's own module doc). So there's exactly one place a
//! Kafka failure can surface here: the initial `connect()` call, made
//! once before the port loop (matching the source's own single
//! `monitor = SLOMonitor(...)` construction site there). If it fails,
//! every contract-configured port's freshness *and* completeness rows
//! become `"unavailable"` with that one error's message -- behaviorally
//! equivalent to every per-dimension `except` branch firing with the
//! same underlying cause, just detected once instead of twice per port.
//!
//! # Publish failures are swallowed, not just publisher-construction ones
//!
//! The source only wraps `SLOViolationPublisher(...)` construction and
//! `.flush()` in `try`/`except` -- a bare `publisher.publish(...)` call
//! inside the loop is *not* guarded, so a mid-loop Kafka failure there
//! would raise all the way out of the source's own `slo()` function.
//! Every row around it, though, is about resilience (report SLOs even
//! when Kafka is flaky; publishing is explicitly best-effort) -- so
//! this port swallows a failed `publish()` the same way it swallows a
//! failed publisher construction, rather than letting a side-channel
//! governance-event failure take down the whole SLO report. A
//! deliberate strengthening past the source's literal gap, not a
//! silent behavior change: nothing about *this* command's own SLO
//! reporting depends on whether the violation event made it to Kafka.

use crate::command_output::CommandOutput;
use crate::format::{dim, green, red, yellow, OutputFormat, Table};
use rusty_json::json;
use rusty_meshed_observability::{
    get_violation_count, SLOMonitor, SLOResult, SLOViolationPayload, SLOViolationPublisher,
};
use rusty_sqlite::rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
use rusty_tokio::io::TcpStream;

struct ProductRow {
    id: i64,
}

struct PortWithContract {
    topic_name: String,
    schema_subject: String,
    /// `None` when the port has no linked `data_contracts` row (SLO
    /// not configured for it).
    slo_freshness_seconds: Option<i64>,
}

fn fetch_product_id_by_name(conn: &Connection, name: &str) -> SqlResult<Option<ProductRow>> {
    conn.query_row(
        "SELECT id FROM data_products WHERE name = ?1",
        params![name],
        |row| Ok(ProductRow { id: row.get(0)? }),
    )
    .optional()
}

fn fetch_output_ports_with_contract(
    conn: &Connection,
    product_id: i64,
) -> SqlResult<Vec<PortWithContract>> {
    let mut stmt = conn.prepare(
        "SELECT op.topic_name, op.schema_subject, dc.slo_freshness_seconds \
         FROM output_ports op LEFT JOIN data_contracts dc ON dc.output_port_id = op.id \
         WHERE op.data_product_id = ?1 ORDER BY op.id ASC",
    )?;
    let rows = stmt.query_map(params![product_id], |row| {
        Ok(PortWithContract {
            topic_name: row.get(0)?,
            schema_subject: row.get(1)?,
            slo_freshness_seconds: row.get(2)?,
        })
    })?;
    rows.collect()
}

fn internal_error() -> CommandOutput {
    CommandOutput::error(format!("{} internal error.\n", red("Error:")), 1)
}

/// One row of `meshed slo`'s output (CLI-032..036/041): a single SLO
/// dimension's status for one port, or (CLI-032) an `"unconfigured"`
/// stand-in row when the port has no data contract at all.
struct SloRow {
    port: String,
    slo_type: String,
    status: String,
    threshold: String,
    actual: String,
    message: String,
}

impl SloRow {
    fn unconfigured(topic: &str) -> Self {
        SloRow {
            port: topic.to_string(),
            slo_type: "all".to_string(),
            status: "unconfigured".to_string(),
            threshold: "—".to_string(),
            actual: "—".to_string(),
            message: "No data contract — SLO not configured".to_string(),
        }
    }

    fn unavailable(topic: &str, slo_type: &str, threshold_seconds: i64, error: &str) -> Self {
        SloRow {
            port: topic.to_string(),
            slo_type: slo_type.to_string(),
            status: "unavailable".to_string(),
            threshold: format!("{threshold_seconds}s"),
            actual: "unavailable".to_string(),
            message: format!("Kafka unavailable: {error}"),
        }
    }

    fn from_result(topic: &str, result: &SLOResult) -> Self {
        SloRow {
            port: topic.to_string(),
            slo_type: result.slo_type.clone(),
            status: if result.passed { "PASS" } else { "FAIL" }.to_string(),
            threshold: format!("{:.0}s", result.threshold),
            actual: if result.actual_value.is_infinite() {
                "∞".to_string()
            } else {
                format!("{:.1}s", result.actual_value)
            },
            message: result.message.clone(),
        }
    }

    /// The schema-conformance row's threshold/actual are plain integer
    /// counts, not `Ns` durations (CLI-036) -- built directly rather
    /// than through [`Self::from_result`].
    fn schema_conformance(topic: &str, passed: bool, violation_count: i64, subject: &str) -> Self {
        let message = if passed {
            format!("0 violations for '{subject}'")
        } else {
            format!("{violation_count} violation(s) for '{subject}'")
        };
        SloRow {
            port: topic.to_string(),
            slo_type: "schema_conformance".to_string(),
            status: if passed { "PASS" } else { "FAIL" }.to_string(),
            threshold: "0".to_string(),
            actual: violation_count.to_string(),
            message,
        }
    }

    fn to_json(&self) -> rusty_json::Value {
        json!({
            "port": self.port.as_str(),
            "slo_type": self.slo_type.as_str(),
            "status": self.status.as_str(),
            "threshold": self.threshold.as_str(),
            "actual": self.actual.as_str(),
            "message": self.message.as_str()
        })
    }

    fn status_styled(&self) -> String {
        match self.status.as_str() {
            "PASS" => green(&self.status),
            "FAIL" => red(&self.status),
            "unavailable" => yellow(&self.status),
            "unconfigured" => dim(&self.status),
            _ => self.status.clone(),
        }
    }
}

/// Runs `meshed slo <product> [--format table|json] [--bootstrap-servers ADDR]`
/// against an already-open registry connection. `--registry-url`
/// (CLI-028) isn't a parameter here at all -- the source's own help
/// text calls it "unused in v1; reserved" and never reads it in the
/// function body either.
pub async fn run(
    conn: &Connection,
    kafka_bootstrap_servers: &str,
    product: &str,
    format: OutputFormat,
) -> CommandOutput {
    let dp = match fetch_product_id_by_name(conn, product) {
        Ok(Some(dp)) => dp,
        Ok(None) => {
            return CommandOutput::error(
                format!("{} Data product '{product}' not found.\n", red("Error:")),
                1,
            )
        }
        Err(_) => return internal_error(),
    };
    let ports = match fetch_output_ports_with_contract(conn, dp.id) {
        Ok(ports) => ports,
        Err(_) => return internal_error(),
    };
    if ports.is_empty() {
        return CommandOutput::error(
            format!(
                "{} Data product '{product}' has no output ports.\n",
                yellow("Warning:")
            ),
            1,
        );
    }

    let monitor_result = SLOMonitor::<TcpStream>::connect(kafka_bootstrap_servers).await;
    let kafka_error_text = monitor_result.as_ref().err().map(|err| err.to_string());
    let mut monitor = monitor_result.ok();
    let mut publisher = SLOViolationPublisher::<TcpStream>::connect(kafka_bootstrap_servers)
        .await
        .ok();

    let mut rows = Vec::new();
    for port in &ports {
        let Some(freshness_seconds) = port.slo_freshness_seconds else {
            rows.push(SloRow::unconfigured(&port.topic_name));
            continue;
        };

        for slo_type in ["freshness", "completeness"] {
            let result = match &mut monitor {
                Some(monitor) => {
                    if slo_type == "freshness" {
                        monitor
                            .check_freshness(&port.topic_name, 0, freshness_seconds)
                            .await
                    } else {
                        monitor
                            .check_completeness(&port.topic_name, 0, freshness_seconds)
                            .await
                    }
                }
                None => {
                    rows.push(SloRow::unavailable(
                        &port.topic_name,
                        slo_type,
                        freshness_seconds,
                        kafka_error_text.as_deref().unwrap_or("unknown error"),
                    ));
                    continue;
                }
            };
            if !result.passed {
                publish_violation(&mut publisher, product, &port.topic_name, &result).await;
            }
            rows.push(SloRow::from_result(&port.topic_name, &result));
        }

        let violation_count = get_violation_count(conn, &port.schema_subject).unwrap_or(0);
        let passed = violation_count == 0;
        if !passed {
            let payload = SLOViolationPayload::new(
                product,
                port.topic_name.clone(),
                "schema_conformance",
                0.0,
                violation_count as f64,
                format!(
                    "{violation_count} violation(s) for '{}'",
                    port.schema_subject
                ),
            );
            if let Some(publisher) = &mut publisher {
                let _ = publisher.publish(&payload).await;
            }
        }
        rows.push(SloRow::schema_conformance(
            &port.topic_name,
            passed,
            violation_count,
            &port.schema_subject,
        ));
    }

    if let Some(publisher) = &mut publisher {
        publisher.flush(5.0);
    }

    match format {
        OutputFormat::Json => {
            let data = rusty_json::Value::Array(rows.iter().map(SloRow::to_json).collect());
            CommandOutput::ok(format!(
                "{}\n",
                rusty_json::to_string(&data).expect("built from strings, always serializes")
            ))
        }
        OutputFormat::Table => {
            let mut table = Table::new(
                format!("SLO Status: {product}"),
                &[
                    "Port",
                    "SLO Type",
                    "Status",
                    "Threshold",
                    "Actual",
                    "Message",
                ],
            );
            for row in &rows {
                table.add_row(vec![
                    row.port.clone(),
                    row.slo_type.clone(),
                    row.status_styled(),
                    row.threshold.clone(),
                    row.actual.clone(),
                    row.message.clone(),
                ]);
            }
            CommandOutput::ok(table.render())
        }
    }
}

async fn publish_violation(
    publisher: &mut Option<SLOViolationPublisher<TcpStream>>,
    product: &str,
    topic: &str,
    result: &SLOResult,
) {
    let Some(publisher) = publisher else {
        return;
    };
    let payload = SLOViolationPayload::new(
        product,
        topic,
        result.slo_type.clone(),
        result.threshold,
        result.actual_value,
        result.message.clone(),
    );
    let _ = publisher.publish(&payload).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_meshed_observability::ensure_metrics_schema;
    use rusty_meshed_registry::models;

    fn seeded_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        models::ensure_schema(&conn).unwrap();
        ensure_metrics_schema(&conn).unwrap();
        conn
    }

    fn insert_product(conn: &Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO data_products (name, owner, version, domain, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![name, "team-a", "1.0.0", "commerce", "desc"],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_output_port(conn: &Connection, product_id: i64, topic: &str) -> i64 {
        conn.execute(
            "INSERT INTO output_ports (data_product_id, topic_name, schema_subject, event_type) VALUES (?1, ?2, ?3, ?4)",
            params![product_id, topic, format!("{topic}-value"), "delta"],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_contract(conn: &Connection, port_id: i64, freshness_seconds: i64) {
        conn.execute(
            "INSERT INTO data_contracts (output_port_id, schema_ref, owner, slo_freshness_seconds, slo_completeness_pct) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![port_id, "sub:1", "team-a", freshness_seconds, 99.0],
        )
        .unwrap();
    }

    const UNREACHABLE_KAFKA: &str = "127.0.0.1:1";

    #[rusty_tokio::test]
    async fn unknown_product_prints_error_and_exits_1() {
        let conn = seeded_connection();
        let output = run(
            &conn,
            UNREACHABLE_KAFKA,
            "no-such-product",
            OutputFormat::Table,
        )
        .await;
        assert_eq!(output.exit_code, 1);
        assert!(output
            .text
            .contains("Data product 'no-such-product' not found."));
    }

    #[rusty_tokio::test]
    async fn a_product_with_no_output_ports_prints_a_warning_and_exits_1() {
        let conn = seeded_connection();
        insert_product(&conn, "orders");
        let output = run(&conn, UNREACHABLE_KAFKA, "orders", OutputFormat::Table).await;
        assert_eq!(output.exit_code, 1);
        assert!(output
            .text
            .contains("Data product 'orders' has no output ports."));
    }

    #[rusty_tokio::test]
    async fn a_port_with_no_contract_reports_unconfigured() {
        let conn = seeded_connection();
        let id = insert_product(&conn, "orders");
        insert_output_port(&conn, id, "commerce.orders");

        let output = run(&conn, UNREACHABLE_KAFKA, "orders", OutputFormat::Json).await;
        assert_eq!(output.exit_code, 0);
        let json = rusty_json::from_str::<rusty_json::Value>(output.text.trim()).unwrap();
        let rows = json.as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("status").unwrap().as_str(),
            Some("unconfigured")
        );
        assert_eq!(rows[0].get("slo_type").unwrap().as_str(), Some("all"));
        assert_eq!(rows[0].get("threshold").unwrap().as_str(), Some("—"));
    }

    #[rusty_tokio::test]
    async fn an_unreachable_broker_reports_freshness_and_completeness_as_unavailable() {
        let conn = seeded_connection();
        let id = insert_product(&conn, "orders");
        let port_id = insert_output_port(&conn, id, "commerce.orders");
        insert_contract(&conn, port_id, 60);

        let output = run(&conn, UNREACHABLE_KAFKA, "orders", OutputFormat::Json).await;
        assert_eq!(output.exit_code, 0);
        let json = rusty_json::from_str::<rusty_json::Value>(output.text.trim()).unwrap();
        let rows = json.as_array().unwrap();

        let freshness = rows
            .iter()
            .find(|r| r.get("slo_type").unwrap().as_str() == Some("freshness"))
            .unwrap();
        assert_eq!(
            freshness.get("status").unwrap().as_str(),
            Some("unavailable")
        );
        assert_eq!(freshness.get("threshold").unwrap().as_str(), Some("60s"));
        assert_eq!(
            freshness.get("actual").unwrap().as_str(),
            Some("unavailable")
        );
        assert!(freshness
            .get("message")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("Kafka unavailable:"));

        let completeness = rows
            .iter()
            .find(|r| r.get("slo_type").unwrap().as_str() == Some("completeness"))
            .unwrap();
        assert_eq!(
            completeness.get("status").unwrap().as_str(),
            Some("unavailable")
        );
    }

    #[rusty_tokio::test]
    async fn schema_conformance_passes_with_zero_recorded_violations_and_needs_no_kafka() {
        let conn = seeded_connection();
        let id = insert_product(&conn, "orders");
        let port_id = insert_output_port(&conn, id, "commerce.orders");
        insert_contract(&conn, port_id, 60);

        let output = run(&conn, UNREACHABLE_KAFKA, "orders", OutputFormat::Json).await;
        let json = rusty_json::from_str::<rusty_json::Value>(output.text.trim()).unwrap();
        let rows = json.as_array().unwrap();
        let schema_row = rows
            .iter()
            .find(|r| r.get("slo_type").unwrap().as_str() == Some("schema_conformance"))
            .unwrap();
        assert_eq!(schema_row.get("status").unwrap().as_str(), Some("PASS"));
        assert_eq!(schema_row.get("threshold").unwrap().as_str(), Some("0"));
        assert_eq!(schema_row.get("actual").unwrap().as_str(), Some("0"));
    }

    #[rusty_tokio::test]
    async fn schema_conformance_fails_with_recorded_violations() {
        use rusty_meshed_observability::record_violation;

        let conn = seeded_connection();
        let id = insert_product(&conn, "orders");
        let port_id = insert_output_port(&conn, id, "commerce.orders");
        insert_contract(&conn, port_id, 60);
        record_violation(&conn, "commerce.orders-value", "bad field").unwrap();

        let output = run(&conn, UNREACHABLE_KAFKA, "orders", OutputFormat::Json).await;
        let json = rusty_json::from_str::<rusty_json::Value>(output.text.trim()).unwrap();
        let rows = json.as_array().unwrap();
        let schema_row = rows
            .iter()
            .find(|r| r.get("slo_type").unwrap().as_str() == Some("schema_conformance"))
            .unwrap();
        assert_eq!(schema_row.get("status").unwrap().as_str(), Some("FAIL"));
        assert_eq!(schema_row.get("actual").unwrap().as_str(), Some("1"));
        assert!(schema_row
            .get("message")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("1 violation(s)"));
    }

    #[rusty_tokio::test]
    async fn table_output_includes_the_title_and_all_dimension_rows() {
        let conn = seeded_connection();
        let id = insert_product(&conn, "orders");
        let port_id = insert_output_port(&conn, id, "commerce.orders");
        insert_contract(&conn, port_id, 60);

        let output = run(&conn, UNREACHABLE_KAFKA, "orders", OutputFormat::Table).await;
        assert_eq!(output.exit_code, 0);
        assert!(output.text.contains("SLO Status: orders"));
        assert!(output.text.contains("freshness"));
        assert!(output.text.contains("completeness"));
        assert!(output.text.contains("schema_conformance"));
        assert!(output.text.contains("commerce.orders"));
    }

    #[rusty_tokio::test]
    async fn multiple_ports_each_get_their_own_rows() {
        let conn = seeded_connection();
        let id = insert_product(&conn, "orders");
        let port_a = insert_output_port(&conn, id, "commerce.orders.a");
        insert_contract(&conn, port_a, 60);
        insert_output_port(&conn, id, "commerce.orders.b"); // no contract

        let output = run(&conn, UNREACHABLE_KAFKA, "orders", OutputFormat::Json).await;
        let json = rusty_json::from_str::<rusty_json::Value>(output.text.trim()).unwrap();
        let rows = json.as_array().unwrap();
        // Port A: freshness + completeness + schema_conformance = 3 rows.
        // Port B: unconfigured = 1 row.
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().any(
            |r| r.get("port").unwrap().as_str() == Some("commerce.orders.b")
                && r.get("status").unwrap().as_str() == Some("unconfigured")
        ));
    }
}
