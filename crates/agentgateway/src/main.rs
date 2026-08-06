//! Command line entry point. The data plane lives in the library crate.

use std::net::SocketAddr;

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use clap::Parser;

/// Command line arguments.
#[derive(Debug, Parser)]
#[command(name = "agentgateway", version, about)]
struct Cli {
    /// Path to the configuration file.
    #[arg(short, long, env = "AGENTGATEWAY_CONFIG", default_value = "config.yaml")]
    file: String,

    /// Address to bind listeners on, as a host. Ports come from the config.
    #[arg(long, env = "AGENTGATEWAY_HOST", default_value = "0.0.0.0")]
    host: String,

    /// Log filter, in `RUST_LOG` syntax.
    #[arg(long, env = "AGENTGATEWAY_LOG", default_value = "info")]
    log: String,

    /// Load and check the configuration, then exit without binding anything.
    #[arg(long)]
    check: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let config = Config::load(&cli.file)?;
    let filter = config
        .config
        .as_ref()
        .and_then(|c| c.logging.as_ref())
        .and_then(|l| l.filter.clone())
        .unwrap_or(cli.log);

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    // Everything the config asked for that this build does not do. Reported
    // before serving, so an operator learns it from startup rather than from
    // a policy quietly not applying in production.
    for finding in config.lint() {
        tracing::warn!("{finding}");
    }

    if cli.check {
        println!("{}: configuration is valid", cli.file);
        return Ok(());
    }

    let gateway = Gateway::build(&config).await?;

    let addrs: Vec<SocketAddr> = gateway
        .ports()
        .into_iter()
        .map(|port| -> anyhow::Result<SocketAddr> {
            Ok(format!("{}:{}", cli.host, port).parse()?)
        })
        .collect::<Result<_, _>>()?;

    serve::run(gateway, addrs).await
}
