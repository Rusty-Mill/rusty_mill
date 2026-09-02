//! `meshed health` -- the Rust port of `meshed.cli.commands.health`
//! (CLI-003..008): a data product's status, output ports, and SLO
//! configuration.

use crate::command_output::CommandOutput;
use crate::format::{red, OutputFormat, Table};
use rusty_json::json;
use rusty_sqlite::rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

struct ProductRow {
    id: i64,
    name: String,
    domain: String,
    owner: String,
    maturity_tier: String,
}

struct PortHealth {
    topic: String,
    schema_subject: String,
    /// `"configured"` if the port has a linked data contract, else
    /// `"unconfigured"` (CLI-008).
    slo_status: &'static str,
}

fn fetch_product_by_name(conn: &Connection, name: &str) -> SqlResult<Option<ProductRow>> {
    conn.query_row(
        "SELECT id, name, domain, owner, maturity_tier FROM data_products WHERE name = ?1",
        params![name],
        |row| {
            Ok(ProductRow {
                id: row.get(0)?,
                name: row.get(1)?,
                domain: row.get(2)?,
                owner: row.get(3)?,
                maturity_tier: row.get(4)?,
            })
        },
    )
    .optional()
}

fn fetch_ports_with_slo_status(conn: &Connection, product_id: i64) -> SqlResult<Vec<PortHealth>> {
    let mut stmt = conn.prepare(
        "SELECT op.topic_name, op.schema_subject, \
         (SELECT COUNT(*) FROM data_contracts dc WHERE dc.output_port_id = op.id) \
         FROM output_ports op WHERE op.data_product_id = ?1 ORDER BY op.id ASC",
    )?;
    let rows = stmt.query_map(params![product_id], |row| {
        let topic: String = row.get(0)?;
        let schema_subject: String = row.get(1)?;
        let contract_count: i64 = row.get(2)?;
        Ok(PortHealth {
            topic,
            schema_subject,
            slo_status: if contract_count > 0 {
                "configured"
            } else {
                "unconfigured"
            },
        })
    })?;
    rows.collect()
}

fn internal_error() -> CommandOutput {
    CommandOutput::error(format!("{} internal error.\n", red("Error:")), 1)
}

/// Runs `meshed health <product> [--format table|json]` against an
/// already-open registry connection.
pub fn run(conn: &Connection, product: &str, format: OutputFormat) -> CommandOutput {
    let dp = match fetch_product_by_name(conn, product) {
        Ok(Some(dp)) => dp,
        Ok(None) => {
            return CommandOutput::error(
                format!("{} Data product '{product}' not found.\n", red("Error:")),
                1,
            )
        }
        Err(_) => return internal_error(),
    };
    let ports = match fetch_ports_with_slo_status(conn, dp.id) {
        Ok(ports) => ports,
        Err(_) => return internal_error(),
    };
    let slo_status = if ports.iter().any(|p| p.slo_status == "configured") {
        "configured"
    } else {
        "unconfigured"
    };

    match format {
        OutputFormat::Json => {
            let mut ports_json = Vec::new();
            for port in &ports {
                ports_json.push(json!({
                    "topic": port.topic.as_str(),
                    "schema_subject": port.schema_subject.as_str(),
                    "slo_status": port.slo_status
                }));
            }
            let data = json!({
                "name": dp.name.as_str(),
                "domain": dp.domain.as_str(),
                "owner": dp.owner.as_str(),
                "maturity_tier": dp.maturity_tier.as_str(),
                "ports": rusty_json::Value::Array(ports_json),
                "slo_status": slo_status
            });
            CommandOutput::ok(format!(
                "{}\n",
                rusty_json::to_string(&data).expect("built from strings, always serializes")
            ))
        }
        OutputFormat::Table => {
            let mut main = Table::new(format!("Data Product: {}", dp.name), &["Field", "Value"]);
            main.add_row(vec!["Name".to_string(), dp.name.clone()]);
            main.add_row(vec!["Domain".to_string(), dp.domain.clone()]);
            main.add_row(vec!["Owner".to_string(), dp.owner.clone()]);
            main.add_row(vec!["Maturity Tier".to_string(), dp.maturity_tier.clone()]);
            main.add_row(vec!["SLO Status".to_string(), slo_status.to_string()]);
            main.add_row(vec!["Output Ports".to_string(), ports.len().to_string()]);
            let mut out = main.render();

            if !ports.is_empty() {
                let mut port_table =
                    Table::new("Output Ports", &["Topic", "Schema Subject", "SLO Status"]);
                for port in &ports {
                    port_table.add_row(vec![
                        port.topic.clone(),
                        port.schema_subject.clone(),
                        port.slo_status.to_string(),
                    ]);
                }
                out.push('\n');
                out.push_str(&port_table.render());
            }
            CommandOutput::ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_meshed_registry::models;

    fn seeded_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        models::ensure_schema(&conn).unwrap();
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

    #[test]
    fn unknown_product_prints_error_and_exits_1() {
        let conn = seeded_connection();
        let output = run(&conn, "no-such-product", OutputFormat::Table);
        assert_eq!(output.exit_code, 1);
        assert!(output
            .text
            .contains("Data product 'no-such-product' not found."));
    }

    #[test]
    fn json_output_reports_unconfigured_slo_status_with_no_contract() {
        let conn = seeded_connection();
        let id = insert_product(&conn, "orders");
        insert_output_port(&conn, id, "commerce.orders");

        let output = run(&conn, "orders", OutputFormat::Json);
        assert_eq!(output.exit_code, 0);
        let json = rusty_json::from_str::<rusty_json::Value>(output.text.trim()).unwrap();
        assert_eq!(json.get("name").unwrap().as_str(), Some("orders"));
        assert_eq!(
            json.get("slo_status").unwrap().as_str(),
            Some("unconfigured")
        );
        let ports = json.get("ports").unwrap().as_array().unwrap();
        assert_eq!(ports.len(), 1);
        assert_eq!(
            ports[0].get("slo_status").unwrap().as_str(),
            Some("unconfigured")
        );
    }

    #[test]
    fn json_output_reports_configured_slo_status_when_a_contract_exists() {
        let conn = seeded_connection();
        let id = insert_product(&conn, "orders");
        let port_id = insert_output_port(&conn, id, "commerce.orders");
        conn.execute(
            "INSERT INTO data_contracts (output_port_id, schema_ref, owner, slo_freshness_seconds, slo_completeness_pct) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![port_id, "commerce.orders-value:1", "team-a", 60, 99.5],
        )
        .unwrap();

        let output = run(&conn, "orders", OutputFormat::Json);
        let json = rusty_json::from_str::<rusty_json::Value>(output.text.trim()).unwrap();
        assert_eq!(json.get("slo_status").unwrap().as_str(), Some("configured"));
    }

    #[test]
    fn table_output_includes_product_fields_and_a_ports_table() {
        let conn = seeded_connection();
        let id = insert_product(&conn, "orders");
        insert_output_port(&conn, id, "commerce.orders");

        let output = run(&conn, "orders", OutputFormat::Table);
        assert_eq!(output.exit_code, 0);
        assert!(output.text.contains("Data Product: orders"));
        assert!(output.text.contains("Output Ports"));
        assert!(output.text.contains("commerce.orders"));
    }

    #[test]
    fn table_output_omits_the_ports_table_when_there_are_no_ports() {
        let conn = seeded_connection();
        insert_product(&conn, "orders");
        let output = run(&conn, "orders", OutputFormat::Table);
        assert!(!output.text.contains("Schema Subject"));
    }
}
