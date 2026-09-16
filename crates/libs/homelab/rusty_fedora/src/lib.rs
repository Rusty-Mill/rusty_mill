//! An async client for [`rusty_fedora_agent`](../rusty_fedora_agent)'s
//! local HTTP API: system status, systemd service listing/control,
//! journal reads, dnf update listing/install/remove (with task polling),
//! and allowlisted config file read/write.
//!
//! Same shape as [`rusty_opnsense::OpnsenseClient`]/
//! [`rusty_proxmox::ProxmoxClient`] -- built on
//! [`rusty_request`](https://github.com/baileyrd/rusty_request), the
//! ecosystem's own async HTTP client, and returns the agent's own JSON
//! (`serde_json::Value`) rather than a hand-maintained struct per
//! endpoint, so `rusty_homelab_mcp`'s tool layer stays a thin
//! pass-through, consistent with the other two backends.
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> rusty_fedora::Result<()> {
//! use rusty_fedora::{FedoraAgentClient, FedoraAgentConfig, ServiceAction};
//!
//! let client = FedoraAgentClient::new(FedoraAgentConfig {
//!     base_url: "http://100.x.y.z:8765".to_string(),
//!     timeout: None,
//! });
//!
//! let status = client.system_status().await?;
//! client.service_control("ollama.service", ServiceAction::Restart).await?;
//! # Ok(())
//! # }
//! ```

mod client;
mod error;
mod model;

pub use client::{FedoraAgentClient, FedoraAgentConfig};
pub use error::{Error, Result};
pub use model::{Priority, ServiceAction, UnitType};
