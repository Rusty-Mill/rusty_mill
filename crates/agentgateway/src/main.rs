//! Command line entry point. The data plane lives in the library crate.

use std::net::SocketAddr;

use agentgateway::{Gateway, Telemetry, serve};
use agentgateway_config::Config;
use clap::Parser;
use rusty_mcp::limits::LimitsLayer;

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

    if cli.check {
        // Before installing telemetry: --check should not open a connection to
        // a collector just to say a file parses.
        println!("{}: configuration is valid", cli.file);
        for finding in config.lint() {
            println!("  warning: {finding}");
        }
        return Ok(());
    }

    let telemetry = Telemetry::install(&config, &cli.log)?;

    // Everything the config asked for that this build does not do. Reported
    // before serving, so an operator learns it from startup rather than from
    // a policy quietly not applying in production.
    for finding in config.lint() {
        tracing::warn!("{finding}");
    }

    let gateway = Gateway::build(&config, telemetry.instruments()).await?;

    let addrs: Vec<SocketAddr> = gateway
        .ports()
        .into_iter()
        .map(|port| -> anyhow::Result<SocketAddr> { Ok(format!("{}:{}", cli.host, port).parse()?) })
        .collect::<Result<_, _>>()?;

    let result = serve::run(gateway, addrs, limits(&config)).await;

    // After serving stops, not before: spans and metrics are batched, and
    // whatever is still buffered dies with the process otherwise. This is the
    // single most common way to end up staring at an empty collector.
    telemetry.shutdown();

    result
}

/// Build the process-wide shedding layer from `config.limits`.
///
/// Both bounds are off unless configured. There is no value right for
/// everyone, and a default would be a silent regression for a gateway already
/// serving more than it.
fn limits(config: &Config) -> LimitsLayer {
    let mut layer = LimitsLayer::new();
    let Some(limits) = config.config.as_ref().and_then(|c| c.limits.as_ref()) else {
        return layer;
    };

    if let Some(max) = limits.max_concurrent_requests {
        layer = layer.with_max_concurrent(max);
        tracing::info!(max_concurrent = max, "shedding above the concurrency limit");
    }
    // The per-route timeout in the data plane is the one that produces a 504
    // with the route named. This is the outer backstop for anything that
    // escapes it.
    if let Some(timeout) = limits.request_timeout {
        layer = layer.with_timeout(timeout.into());
    }
    layer
}
