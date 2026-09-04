//! `rusty_homelab_mcp`: an MCP server for controlling a homelab.
//!
//! Proxmox VE, OPNsense, and Fedora (via a `rusty_fedora_agent` instance)
//! today; every backend is optional and independent of the others, and
//! more are welcome (see `src/tools/mod.rs`).
//!
//! Run it over stdio -- what a desktop client launches:
//!
//! ```text
//! cargo run -p rusty_homelab_mcp -- \
//!     --proxmox-url https://pve.lan:8006 \
//!     --proxmox-token-id automation@pve!homelab-mcp \
//!     --proxmox-token-secret xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx \
//!     --proxmox-insecure
//! ```
//!
//! or over Streamable HTTP with `--transport http --bind 127.0.0.1:8080`.
//! Every flag has an environment fallback (`PROXMOX_URL`, `OPNSENSE_URL`,
//! `FEDORA_AGENT_URL`, ...); see `--help` for the full list.

mod config;
mod json_result;
mod server;
mod tools;

use clap::Parser as _;
use config::HomelabCli;
use rusty_fedora::FedoraAgentClient;
use rusty_opnsense::OpnsenseClient;
use rusty_proxmox::ProxmoxClient;
use server::HomelabServer;

#[tokio::main]
async fn main() -> Result<(), rusty_mcp::ServeError> {
    let cli = HomelabCli::parse();

    // A URL set without its matching credentials is much more likely a
    // typo'd flag than an intentionally half-configured backend -- fail
    // fast with a plain message rather than starting a server whose tools
    // for that backend can never work.
    let proxmox_config = cli.proxmox_config().unwrap_or_else(|msg| {
        eprintln!("error: {msg}");
        std::process::exit(2);
    });
    let opnsense_config = cli.opnsense_config().unwrap_or_else(|msg| {
        eprintln!("error: {msg}");
        std::process::exit(2);
    });
    let fedora_config = cli.fedora_config();

    let server_config: rusty_mcp::ServerConfig = cli.mcp.into();
    rusty_mcp::telemetry::init(&server_config.log_filter);

    if proxmox_config.is_none() && opnsense_config.is_none() && fedora_config.is_none() {
        tracing::warn!(
            "no backend is configured -- every tool call will fail until PROXMOX_*, \
             OPNSENSE_*, and/or FEDORA_AGENT_URL flags or environment variables are set"
        );
    }

    // Built once, cloned into each handler: Streamable HTTP constructs a
    // fresh handler per request, but every client (and its connection
    // pool) should be shared across every call, not rebuilt each time.
    let proxmox = proxmox_config.map(ProxmoxClient::new);
    let opnsense = opnsense_config.map(OpnsenseClient::new);
    let fedora = fedora_config.map(FedoraAgentClient::new);

    rusty_mcp::serve(
        move || Ok(HomelabServer::new(proxmox.clone(), opnsense.clone(), fedora.clone())),
        server_config,
    )
    .await
}
