//! An async client for the [OPNsense](https://opnsense.org/) REST API.
//!
//! Covers system status, service listing/control, interface listing,
//! firewall alias export, and gateway status -- the small slice of the API a
//! homelab automation tool typically needs. Built on
//! [`rusty_request`](https://github.com/baileyrd/rusty_request), the
//! ecosystem's own async HTTP client, and returns OPNsense's own JSON
//! (`serde_json::Value`) rather than a hand-maintained struct per endpoint,
//! since the response shape (and field naming) differs across endpoints and
//! plugin versions -- see the [module reference](https://docs.opnsense.org/development/api.html)
//! for what each one returns.
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> rusty_opnsense::Result<()> {
//! use rusty_opnsense::{OpnsenseClient, OpnsenseConfig, ServiceAction};
//!
//! let client = OpnsenseClient::new(OpnsenseConfig {
//!     base_url: "https://opnsense.lan".to_string(),
//!     key: "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string(),
//!     secret: "yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy".to_string(),
//!     insecure: true, // self-signed cert, typical for a homelab
//!     timeout: None,
//! });
//!
//! let status = client.system_status().await?;
//! client.service_control("unbound", ServiceAction::Restart).await?;
//! # Ok(())
//! # }
//! ```

mod client;
mod error;
mod model;

pub use client::{OpnsenseClient, OpnsenseConfig};
pub use error::{Error, Result};
pub use model::ServiceAction;
