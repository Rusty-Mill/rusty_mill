//! An async client for the Proxmox VE REST API.
//!
//! Covers cluster/node listing, guest (QEMU VM and LXC container) listing
//! and status, and guest power control -- the small slice of the API a
//! homelab automation tool typically needs. Built on
//! [`rusty_request`](https://github.com/baileyrd/rusty_request), the
//! ecosystem's own async HTTP client, and returns Proxmox's own JSON
//! (`serde_json::Value`) rather than a hand-maintained struct per endpoint,
//! since Proxmox's response shape varies by guest configuration and is
//! already documented by the API viewer bundled with every Proxmox install
//! (`https://<host>:8006/pve-docs/api-viewer/`).
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> rusty_proxmox::Result<()> {
//! use rusty_proxmox::{GuestKind, PowerAction, ProxmoxClient, ProxmoxConfig};
//!
//! let client = ProxmoxClient::new(ProxmoxConfig {
//!     base_url: "https://pve.lan:8006".to_string(),
//!     token_id: "automation@pve!homelab-mcp".to_string(),
//!     token_secret: "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx".to_string(),
//!     insecure: true, // self-signed cert, typical for a homelab
//!     timeout: None,
//! });
//!
//! let nodes = client.list_nodes().await?;
//! let vms = client.list_guests("pve", GuestKind::Qemu).await?;
//! let upid = client
//!     .guest_power("pve", GuestKind::Qemu, 100, PowerAction::Start)
//!     .await?;
//! println!("started as task {upid}");
//! # Ok(())
//! # }
//! ```

mod client;
mod error;
mod model;

pub use client::{ProxmoxClient, ProxmoxConfig};
pub use error::{Error, Result};
pub use model::{GuestKind, PowerAction};
