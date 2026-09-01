//! The `meshed` command-line surface -- the Rust port of
//! `meshed.cli.app`'s Typer application factory (CLI-001), now
//! registering all 4 of the source's subcommands: `health`/`lineage`/
//! `metrics`/`slo`. `slo` was deferred through an earlier pass since
//! `meshed.cli.commands.slo` publishes SLO-violation events via
//! `SLOViolationPublisher`, which needed a Kafka `Produce` request
//! `rusty_kafka` didn't implement yet at the time (see that crate's own
//! module doc, and `rusty-meshed-observability::slo`'s) -- that landed
//! since (GOV-047..049), so `slo` (CLI-026..042) is registered here now
//! too. See [`crate::slo`]'s own module doc for the command's behavior.

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
    /// Show SLO compliance status for a data product's output ports.
    Slo {
        /// Data product name to inspect
        product: String,
        /// Output format: table or json
        #[arg(short, long, value_enum, default_value = "table")]
        format: OutputFormat,
        /// Registry API base URL (unused in v1; reserved)
        #[arg(long, default_value = "http://localhost:8000")]
        registry_url: String,
        /// Kafka bootstrap servers
        #[arg(short = 'b', long, default_value = "localhost:9092")]
        bootstrap_servers: String,
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
        assert!(help.contains("slo"));
    }

    #[test]
    fn slo_defaults_match_the_source() {
        let cli = Cli::try_parse_from(["meshed", "slo", "orders"]).unwrap();
        let Command::Slo {
            format,
            registry_url,
            bootstrap_servers,
            ..
        } = cli.command
        else {
            panic!("expected Slo");
        };
        assert_eq!(format, OutputFormat::Table);
        assert_eq!(registry_url, "http://localhost:8000");
        assert_eq!(bootstrap_servers, "localhost:9092");
    }

    #[test]
    fn slo_accepts_the_short_bootstrap_servers_flag() {
        let cli = Cli::try_parse_from(["meshed", "slo", "orders", "-b", "broker:9092"]).unwrap();
        let Command::Slo {
            bootstrap_servers, ..
        } = cli.command
        else {
            panic!("expected Slo");
        };
        assert_eq!(bootstrap_servers, "broker:9092");
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
