//! `meshed lineage` -- the Rust port of `meshed.cli.commands.lineage`
//! (CLI-009..015): topology dependency pairs for a data product,
//! filtered to rows where it's the consumer.

use crate::command_output::CommandOutput;
use crate::format::{red, yellow, OutputFormat, Table};
use rusty_json::json;
use rusty_meshed_observability::LineageTracker;

/// Runs `meshed lineage <product_name> [--format table|json] [--db-path PATH]`.
/// `db_path` is the already-resolved path (CLI-011's `""` ->
/// `PlatformConfig::registry_db_path` fallback happens in the caller,
/// matching where `app.rs`/`main.rs` resolve `PlatformConfig` for
/// every other command too).
pub fn run(db_path: &str, product_name: &str, format: OutputFormat) -> CommandOutput {
    let Ok(tracker) = LineageTracker::new(db_path) else {
        return CommandOutput::error(format!("{} internal error.\n", red("Error:")), 1);
    };
    let Ok(all_deps) = tracker.get_topology_dependencies() else {
        return CommandOutput::error(format!("{} internal error.\n", red("Error:")), 1);
    };
    let deps: Vec<_> = all_deps
        .into_iter()
        .filter(|dep| dep.consumer == product_name)
        .collect();

    match format {
        OutputFormat::Json => {
            let mut arr = Vec::new();
            for dep in &deps {
                arr.push(json!({
                    "consumer": dep.consumer.as_str(),
                    "input_topic": dep.input_topic.as_str()
                }));
            }
            CommandOutput::ok(format!(
                "{}\n",
                rusty_json::to_string(&rusty_json::Value::Array(arr))
                    .expect("built from strings, always serializes")
            ))
        }
        OutputFormat::Table => {
            if deps.is_empty() {
                return CommandOutput::ok(format!(
                    "{}\n",
                    yellow(&format!(
                        "No lineage topology recorded for '{product_name}'."
                    ))
                ));
            }
            let mut table = Table::new(
                format!("Lineage Topology: {product_name}"),
                &["Consumer Product", "Input Topic"],
            );
            for dep in &deps {
                table.add_row(vec![dep.consumer.clone(), dep.input_topic.clone()]);
            }
            CommandOutput::ok(table.render())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempPath(PathBuf);

    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn temp_db_path() -> TempPath {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        TempPath(std::env::temp_dir().join(format!(
            "rusty_meshed_cli_lineage_test_{}_{n}.db",
            std::process::id()
        )))
    }

    #[test]
    fn table_output_shows_a_yellow_message_with_no_recorded_deps() {
        let path = temp_db_path();
        let output = run(path.0.to_str().unwrap(), "orders", OutputFormat::Table);
        assert_eq!(output.exit_code, 0);
        assert!(output
            .text
            .contains("No lineage topology recorded for 'orders'."));
    }

    #[test]
    fn table_output_lists_deps_where_the_product_is_the_consumer() {
        let path = temp_db_path();
        let tracker = LineageTracker::new(path.0.to_str().unwrap()).unwrap();
        tracker
            .record_job_run(
                "orders",
                "meshed",
                &[("kafka".to_string(), "commerce.orders".to_string())],
                &[],
            )
            .unwrap();
        tracker
            .record_job_run(
                "other-product",
                "meshed",
                &[("kafka".to_string(), "commerce.unrelated".to_string())],
                &[],
            )
            .unwrap();

        let output = run(path.0.to_str().unwrap(), "orders", OutputFormat::Table);
        assert!(output.text.contains("Lineage Topology: orders"));
        assert!(output.text.contains("commerce.orders"));
        assert!(!output.text.contains("commerce.unrelated"));
    }

    #[test]
    fn json_output_filters_to_the_requested_consumer() {
        let path = temp_db_path();
        let tracker = LineageTracker::new(path.0.to_str().unwrap()).unwrap();
        tracker
            .record_job_run(
                "orders",
                "meshed",
                &[("kafka".to_string(), "commerce.orders".to_string())],
                &[],
            )
            .unwrap();

        let output = run(path.0.to_str().unwrap(), "orders", OutputFormat::Json);
        let json = rusty_json::from_str::<rusty_json::Value>(output.text.trim()).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("consumer").unwrap().as_str(), Some("orders"));
        assert_eq!(
            arr[0].get("input_topic").unwrap().as_str(),
            Some("commerce.orders")
        );
    }
}
