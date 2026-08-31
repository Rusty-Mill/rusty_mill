//! The server handler: the two backend clients plus the composed tool
//! router.

use rmcp::{
    ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{ErrorData, ServerCapabilities, ServerInfo},
    tool_handler,
};
use rusty_mcp::ToolError;
use rusty_opnsense::OpnsenseClient;
use rusty_proxmox::ProxmoxClient;

/// The MCP server handler for `rusty_homelab_mcp`.
///
/// Holds the two backend clients, each `None` if its flags weren't set at
/// startup (see [`crate::config::HomelabCli`]). Cheap to clone: both clients share
/// their own connection pools underneath, so cloning this only clones the
/// `Option` wrappers and the tool router.
#[derive(Clone)]
pub struct HomelabServer {
    proxmox: Option<ProxmoxClient>,
    opnsense: Option<OpnsenseClient>,
    tool_router: ToolRouter<Self>,
}

impl HomelabServer {
    /// Build a server. Neither client connects here -- the first real
    /// network request is whichever tool is called first.
    pub fn new(proxmox: Option<ProxmoxClient>, opnsense: Option<OpnsenseClient>) -> Self {
        Self {
            proxmox,
            opnsense,
            // Each backend contributes its own router; `+` merges them.
            // Adding a third backend (Home Assistant, UniFi, ...) is one
            // more module and one more term here.
            tool_router: Self::proxmox_tools() + Self::opnsense_tools(),
        }
    }

    /// The configured Proxmox client, or a protocol error naming the flags
    /// to set. Every `proxmox_*` tool starts with this rather than each
    /// repeating the same `Option::ok_or_else`.
    pub(crate) fn proxmox(&self) -> Result<&ProxmoxClient, ErrorData> {
        self.proxmox.as_ref().ok_or_else(|| {
            ToolError::invalid(
                "Proxmox is not configured on this server -- set --proxmox-url, \
                 --proxmox-token-id, and --proxmox-token-secret (or the matching \
                 PROXMOX_URL/PROXMOX_TOKEN_ID/PROXMOX_TOKEN_SECRET environment \
                 variables) at startup.",
            )
            .into()
        })
    }

    /// The configured OPNsense client, or a protocol error naming the flags
    /// to set.
    pub(crate) fn opnsense(&self) -> Result<&OpnsenseClient, ErrorData> {
        self.opnsense.as_ref().ok_or_else(|| {
            ToolError::invalid(
                "OPNsense is not configured on this server -- set --opnsense-url, \
                 --opnsense-key, and --opnsense-secret (or the matching \
                 OPNSENSE_URL/OPNSENSE_KEY/OPNSENSE_SECRET environment variables) \
                 at startup.",
            )
            .into()
        })
    }

    fn instructions(&self) -> String {
        let proxmox = if self.proxmox.is_some() {
            "configured"
        } else {
            "not configured"
        };
        let opnsense = if self.opnsense.is_some() {
            "configured"
        } else {
            "not configured"
        };
        format!(
            "Control a homelab's infrastructure. Proxmox VE ({proxmox}): node and \
             guest (QEMU VM / LXC container) listing, status, and power control. \
             OPNsense ({opnsense}): system status, service control, interfaces, \
             firewall aliases, and gateways. Calling a tool for an unconfigured \
             backend returns an error explaining which flags or environment \
             variables to set, rather than failing silently or omitting the tool."
        )
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for HomelabServer {
    fn get_info(&self) -> ServerInfo {
        // `rusty_mcp::server_info` pins the advertised revision to 2026-07-28;
        // `ServerInfo::new` alone would still advertise 2025-11-25.
        rusty_mcp::server_info(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_instructions(self.instructions())
    }
}
