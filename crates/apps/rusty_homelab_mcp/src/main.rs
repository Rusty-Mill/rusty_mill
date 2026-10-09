//! `rusty_homelab_mcp`: an MCP server for controlling a homelab.
//!
//! Proxmox VE, OPNsense, and Fedora (via one or more `rusty_fedora_agent`
//! instances -- see [`hosts`]) today; every backend is optional and
//! independent of the others, and more are welcome (see
//! `src/tools/mod.rs`).
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
//! or over Streamable HTTP with `--transport http --bind 127.0.0.1:8080
//! --auth-token <token>` (the HTTP transport is open to any caller
//! unless `--auth-token`/`HOMELAB_MCP_AUTH_TOKEN` is set).
//! Every flag has an environment fallback (`PROXMOX_URL`, `OPNSENSE_URL`,
//! `FEDORA_AGENT_URL`, `FEDORA_HOSTS_FILE`, `HOMELAB_MCP_AUTH_TOKEN`,
//! ...); see `--help` for the full list.

mod config;
mod hosts;
mod json_result;
mod serve;
mod server;
mod tool_support;
mod tools;

use clap::Parser as _;
use config::HomelabCli;
use rusty_opnsense::OpnsenseClient;
use rusty_proxmox::ProxmoxClient;
use server::HomelabServer;

#[tokio::main]
async fn main() {
    let cli = HomelabCli::parse();

    // A URL set without its matching credentials is much more likely a
    // typo'd flag than an intentionally half-configured backend -- fail
    // fast with a plain message rather than starting a server whose tools
    // for that backend can never work.
    let proxmox_config = cli.proxmox_config().unwrap_or_else(|msg| exit_with(&msg));
    let opnsense_config = cli.opnsense_config().unwrap_or_else(|msg| exit_with(&msg));
    let fedora_hosts = cli.fedora_hosts().unwrap_or_else(|msg| exit_with(&msg));
    let token = cli.bearer_token();
    if token.is_some() && cli.mcp.transport == serve::TransportArg::Stdio {
        exit_with("--auth-token/HOMELAB_MCP_AUTH_TOKEN requires --transport http");
    }
    serve::init_logging(&cli.mcp.log);
    if cli.auth_resource_url.is_some() {
        tracing::warn!("--auth-resource-url has no effect any more and is ignored");
    }

    if proxmox_config.is_none() && opnsense_config.is_none() && fedora_hosts.is_empty() {
        tracing::warn!(
            "no backend is configured -- every tool call will fail until PROXMOX_*, \
             OPNSENSE_*, and/or FEDORA_AGENT_URL/FEDORA_HOSTS_FILE flags or environment \
             variables are set"
        );
    }

    // Built once and shared by every call, so each client's connection pool
    // is reused, not rebuilt per request.
    let proxmox = proxmox_config.map(ProxmoxClient::new);
    let opnsense = opnsense_config.map(OpnsenseClient::new);
    let homelab = HomelabServer::new(proxmox, opnsense, fedora_hosts);

    let runtime = tokio::runtime::Handle::current();
    let server = homelab
        .wire_server(&runtime)
        .unwrap_or_else(|e| exit_with(&format!("invalid server description: {e}")));
    let args = cli.mcp.clone();
    // The server's handlers block, so serve from a thread of its own and keep
    // the runtime free to run the tools.
    let served = tokio::task::spawn_blocking(move || serve::serve(server, &args, token)).await;
    match served {
        Ok(Ok(())) => {}
        Ok(Err(msg)) => exit_with(&msg),
        Err(e) => exit_with(&format!("server task failed: {e}")),
    }
}

fn exit_with(message: &str) -> ! {
    eprintln!("error: {message}");
    std::process::exit(2);
}
