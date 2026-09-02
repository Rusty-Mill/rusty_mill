//! `meshed metrics` -- the Rust port of `meshed.cli.commands.metrics`
//! (CLI-016..025): Kafka lag, throughput, and schema-violation count
//! for a data product's first output port.

use crate::command_output::CommandOutput;
use crate::format::{red, yellow, OutputFormat, Table};
use rusty_json::json;
use rusty_meshed_observability::{get_violation_count, MetricsCollector};
use rusty_sqlite::rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

struct ProductRow {
    id: i64,
}

struct OutputPortRow {
    topic_name: String,
    schema_subject: String,
}

fn fetch_product_id_by_name(conn: &Connection, name: &str) -> SqlResult<Option<ProductRow>> {
    conn.query_row(
        "SELECT id FROM data_products WHERE name = ?1",
        params![name],
        |row| Ok(ProductRow { id: row.get(0)? }),
    )
    .optional()
}

/// `dp.output_ports[0]` (CLI-021): the first port, `id ASC`.
fn fetch_first_output_port(conn: &Connection, product_id: i64) -> SqlResult<Option<OutputPortRow>> {
    conn.query_row(
        "SELECT topic_name, schema_subject FROM output_ports \
         WHERE data_product_id = ?1 ORDER BY id ASC LIMIT 1",
        params![product_id],
        |row| {
            Ok(OutputPortRow {
                topic_name: row.get(0)?,
                schema_subject: row.get(1)?,
            })
        },
    )
    .optional()
}

fn internal_error() -> CommandOutput {
    CommandOutput::error(format!("{} internal error.\n", red("Error:")), 1)
}

/// Either a measured value or the source's own `"unavailable"` sentinel
/// (CLI-023) -- distinct types in Python only because a dict value can
/// hold either an `int` or a `str`; here it's an explicit enum so the
/// JSON/table renderers below can each decide how to show it.
enum MetricValue {
    Number(i64),
    Unavailable,
}

impl MetricValue {
    fn to_json(&self) -> rusty_json::Value {
        match self {
            MetricValue::Number(n) => json!(*n),
            MetricValue::Unavailable => json!("unavailable"),
        }
    }

    fn to_display(&self) -> String {
        match self {
            MetricValue::Number(n) => n.to_string(),
            MetricValue::Unavailable => "unavailable".to_string(),
        }
    }
}

/// Runs `meshed metrics <product> [--group-id ID] [--format table|json]`.
pub async fn run(
    conn: &Connection,
    kafka_bootstrap_servers: &str,
    product: &str,
    group_id: Option<&str>,
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
    let port = match fetch_first_output_port(conn, dp.id) {
        Ok(Some(port)) => port,
        Ok(None) => {
            return CommandOutput::error(
                format!(
                    "{} Data product '{product}' has no output ports.\n",
                    yellow("Warning:")
                ),
                1,
            )
        }
        Err(_) => return internal_error(),
    };

    let effective_group_id = group_id
        .map(str::to_string)
        .unwrap_or_else(|| format!("meshed-cli-{product}"));

    let (lag, throughput, violation_count) =
        match MetricsCollector::connect(kafka_bootstrap_servers).await {
            Ok(mut collector) => match collector
                .get_product_metrics(
                    conn,
                    &effective_group_id,
                    &port.topic_name,
                    1,
                    &port.schema_subject,
                )
                .await
            {
                Ok(metrics) => (
                    MetricValue::Number(metrics.lag),
                    MetricValue::Number(metrics.throughput),
                    metrics.violation_count,
                ),
                Err(_) => {
                    let violation_count =
                        get_violation_count(conn, &port.schema_subject).unwrap_or(0);
                    (
                        MetricValue::Unavailable,
                        MetricValue::Unavailable,
                        violation_count,
                    )
                }
            },
            Err(_) => {
                let violation_count = get_violation_count(conn, &port.schema_subject).unwrap_or(0);
                (
                    MetricValue::Unavailable,
                    MetricValue::Unavailable,
                    violation_count,
                )
            }
        };

    match format {
        OutputFormat::Json => {
            let data = json!({
                "product": product,
                "lag": lag.to_json(),
                "throughput": throughput.to_json(),
                "violation_count": violation_count
            });
            CommandOutput::ok(format!(
                "{}\n",
                rusty_json::to_string(&data).expect("built from strings, always serializes")
            ))
        }
        OutputFormat::Table => {
            let mut table = Table::new(format!("Metrics: {product}"), &["Metric", "Value"]);
            table.add_row(vec!["Product".to_string(), product.to_string()]);
            table.add_row(vec!["Lag".to_string(), lag.to_display()]);
            table.add_row(vec!["Throughput".to_string(), throughput.to_display()]);
            table.add_row(vec![
                "Violation Count".to_string(),
                violation_count.to_string(),
            ]);
            CommandOutput::ok(table.render())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_meshed_observability::{ensure_metrics_schema, record_violation};
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

    fn insert_output_port(conn: &Connection, product_id: i64, topic: &str) {
        conn.execute(
            "INSERT INTO output_ports (data_product_id, topic_name, schema_subject, event_type) VALUES (?1, ?2, ?3, ?4)",
            params![product_id, topic, format!("{topic}-value"), "delta"],
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
            None,
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
        let output = run(
            &conn,
            UNREACHABLE_KAFKA,
            "orders",
            None,
            OutputFormat::Table,
        )
        .await;
        assert_eq!(output.exit_code, 1);
        assert!(output
            .text
            .contains("Data product 'orders' has no output ports."));
    }

    #[rusty_tokio::test]
    async fn a_kafka_failure_reports_unavailable_but_still_computes_violation_count() {
        let conn = seeded_connection();
        let id = insert_product(&conn, "orders");
        insert_output_port(&conn, id, "commerce.orders");
        record_violation(&conn, "commerce.orders-value", "bad field").unwrap();

        let output = run(&conn, UNREACHABLE_KAFKA, "orders", None, OutputFormat::Json).await;
        assert_eq!(output.exit_code, 0);
        let json = rusty_json::from_str::<rusty_json::Value>(output.text.trim()).unwrap();
        assert_eq!(json.get("lag").unwrap().as_str(), Some("unavailable"));
        assert_eq!(
            json.get("throughput").unwrap().as_str(),
            Some("unavailable")
        );
        assert_eq!(json.get("violation_count").unwrap().as_f64(), Some(1.0));
    }

    #[rusty_tokio::test]
    async fn table_output_includes_the_product_and_metric_labels() {
        let conn = seeded_connection();
        let id = insert_product(&conn, "orders");
        insert_output_port(&conn, id, "commerce.orders");

        let output = run(
            &conn,
            UNREACHABLE_KAFKA,
            "orders",
            None,
            OutputFormat::Table,
        )
        .await;
        assert!(output.text.contains("Metrics: orders"));
        assert!(output.text.contains("Lag"));
        assert!(output.text.contains("Throughput"));
        assert!(output.text.contains("Violation Count"));
        assert!(output.text.contains("unavailable"));
    }

    #[rusty_tokio::test]
    async fn an_explicit_group_id_is_accepted_without_erroring() {
        let conn = seeded_connection();
        let id = insert_product(&conn, "orders");
        insert_output_port(&conn, id, "commerce.orders");

        let output = run(
            &conn,
            UNREACHABLE_KAFKA,
            "orders",
            Some("custom-group"),
            OutputFormat::Table,
        )
        .await;
        assert_eq!(output.exit_code, 0);
    }
}
