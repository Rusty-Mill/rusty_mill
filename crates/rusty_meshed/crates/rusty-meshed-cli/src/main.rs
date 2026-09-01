//! Thin binary entry point: parses argv, resolves shared setup
//! (`PlatformConfig`, a registry `Connection`), dispatches to the
//! matched subcommand's business logic, then prints its output and
//! exits with its reported code. See `rusty-meshed-cli`'s own
//! (library) module doc for why the actual logic lives there instead
//! of here.

use clap::Parser;
use rusty_meshed_cli::app::{Cli, Command};
use rusty_meshed_cli::{health, lineage, metrics};
use rusty_meshed_registry::AppState;

fn open_registry_connection(db_path: &str) -> Result<rusty_sqlite::rusqlite::Connection, String> {
    let mut state = AppState::new();
    state.set_engine(db_path);
    state.get_session().map_err(|err| err.to_string())
}

#[rusty_tokio::main]
async fn main() {
    let cli = Cli::parse();

    let config = match rusty_meshed_core::PlatformConfig::from_env() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Error: invalid configuration: {err}");
            std::process::exit(1);
        }
    };

    let output = match cli.command {
        Command::Health { product, format } => {
            let conn = match open_registry_connection(&config.registry_db_path) {
                Ok(conn) => conn,
                Err(err) => {
                    eprintln!("Error: {err}");
                    std::process::exit(1);
                }
            };
            health::run(&conn, &product, format)
        }
        Command::Lineage {
            product_name,
            format,
            db_path,
        } => {
            let resolved_db_path = if db_path.is_empty() {
                config.registry_db_path.clone()
            } else {
                db_path
            };
            lineage::run(&resolved_db_path, &product_name, format)
        }
        Command::Metrics {
            product,
            group_id,
            format,
        } => {
            let conn = match open_registry_connection(&config.registry_db_path) {
                Ok(conn) => conn,
                Err(err) => {
                    eprintln!("Error: {err}");
                    std::process::exit(1);
                }
            };
            metrics::run(
                &conn,
                &config.kafka_bootstrap_servers,
                &product,
                group_id.as_deref(),
                format,
            )
            .await
        }
    };

    print!("{}", output.text);
    std::process::exit(output.exit_code);
}
