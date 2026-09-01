//! The `meshed` command-line surface -- the Rust port of
//! `meshed.cli.app`'s Typer application factory (CLI-001).
//!
//! `slo` isn't registered here yet: `meshed.cli.commands.slo` publishes
//! SLO-violation events via `SLOViolationPublisher`, which needs a
//! Kafka `Produce` request `rusty_kafka` doesn't implement (see that
//! crate's own module doc, and `rusty-meshed-observability::slo`'s).
//! `health`/`lineage`/`metrics` need no such thing -- they only read
//! (SQLite queries, `ListOffsets`/`OffsetFetch` via
//! `rusty-meshed-observability::MetricsCollector`) -- so this pass
//! registers those three and stops there rather than shipping a `slo`
//! subcommand that can only ever fail to publish.

use crate::format::OutputFormat;
use clap::{Parser, Subcommand};

/// Meshed data mesh platform CLI — inspect data products and their metrics.
#[derive(Debug, Parser)]
#[command(name = "meshed")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show health status and SLO configuration for a data product.
    Health {
        /// Data product name to inspect
        product: String,
        /// Output format: table or json
        #[arg(short, long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Show data lineage topology for a data product.
    Lineage {
        /// Data product name to show lineage for
        product_name: String,
        /// Output format: table or json
        #[arg(short, long, value_enum, default_value = "table")]
        format: OutputFormat,
        /// Path to the registry SQLite database
        #[arg(long, default_value = "")]
        db_path: String,
    },
    /// Show Kafka lag, throughput, and violation count for a data product.
    Metrics {
        /// Data product name to inspect
        product: String,
        /// Consumer group ID for lag computation
        #[arg(short, long)]
        group_id: Option<String>,
        /// Output format: table or json
        #[arg(short, long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_output_lists_the_registered_subcommands() {
        let mut command = <Cli as clap::CommandFactory>::command();
        let help = command.render_help().to_string();
        assert!(help.contains("health"));
        assert!(help.contains("lineage"));
        assert!(help.contains("metrics"));
    }

    #[test]
    fn defaults_format_to_table() {
        let cli = Cli::try_parse_from(["meshed", "health", "orders"]).unwrap();
        let Command::Health { format, .. } = cli.command else {
            panic!("expected Health");
        };
        assert_eq!(format, OutputFormat::Table);
    }

    #[test]
    fn accepts_the_short_format_flag() {
        let cli = Cli::try_parse_from(["meshed", "metrics", "orders", "-f", "json"]).unwrap();
        let Command::Metrics { format, .. } = cli.command else {
            panic!("expected Metrics");
        };
        assert_eq!(format, OutputFormat::Json);
    }

    #[test]
    fn lineage_db_path_defaults_to_empty() {
        let cli = Cli::try_parse_from(["meshed", "lineage", "orders"]).unwrap();
        let Command::Lineage { db_path, .. } = cli.command else {
            panic!("expected Lineage");
        };
        assert_eq!(db_path, "");
    }
}
