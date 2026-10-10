//! The server: the three backend clients, registered as MCP tools.

use std::sync::Arc;

use rusty_fedora::FedoraAgentClient;
use rusty_mcp_server::{BuildError, Server};
use rusty_opnsense::OpnsenseClient;
use rusty_proxmox::ProxmoxClient;
use tokio::runtime::Handle;

use crate::hosts::FedoraHosts;
use crate::tool_support::{ErrorData, ToolError};
use crate::tools;

/// The MCP server for `rusty_homelab_mcp`.
///
/// Holds the three backends: Proxmox and OPNsense are each `None` if
/// their flags weren't set at startup, and Fedora is a
/// [`FedoraHosts`] registry that's empty in the same case (see
/// [`crate::config::HomelabCli`]). Cheap to clone: every client shares
/// its own connection pool underneath, so cloning this only clones the
/// `Option`/registry wrappers.
#[derive(Clone)]
pub struct HomelabServer {
    proxmox: Option<ProxmoxClient>,
    opnsense: Option<OpnsenseClient>,
    fedora: FedoraHosts,
}

impl HomelabServer {
    /// Build a server. None of the clients connect here -- the first real
    /// network request is whichever tool is called first.
    pub fn new(
        proxmox: Option<ProxmoxClient>,
        opnsense: Option<OpnsenseClient>,
        fedora: FedoraHosts,
    ) -> Self {
        Self {
            proxmox,
            opnsense,
            fedora,
        }
    }

    /// The `rusty_mcp_server` description of this server: every tool of every
    /// backend, run on `rt`. Every tool is always listed regardless of what is
    /// configured.
    ///
    /// # Errors
    /// If the assembled description is invalid (a duplicate tool name).
    pub fn wire_server(&self, rt: &Handle) -> Result<Arc<Server>, BuildError> {
        let builder = Server::builder(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
            .instructions(self.instructions());
        // Each backend registers its own tools; a fourth backend is one more
        // module and one more call here.
        let builder = tools::proxmox::register_tools(builder, self, rt);
        let builder = tools::opnsense::register_tools(builder, self, rt);
        let builder = tools::fedora::register_tools(builder, self, rt);
        Ok(Arc::new(builder.build()?))
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

    /// The `rusty_fedora_agent` client for `host` (or the default host,
    /// "baileyai", if `host` is `None`), or a protocol error -- naming the
    /// flags to set if no Fedora host is configured at all, or naming the
    /// known host ids if `host` doesn't match any of them.
    pub(crate) fn fedora(&self, host: Option<&str>) -> Result<&FedoraAgentClient, ErrorData> {
        if self.fedora.is_empty() {
            return Err(ToolError::invalid(
                "Fedora is not configured on this server -- set --fedora-agent-url \
                 and/or --fedora-hosts-file (or the matching FEDORA_AGENT_URL/ \
                 FEDORA_HOSTS_FILE environment variables) at startup, pointing at \
                 one or more rusty_fedora_agent instances.",
            )
            .into());
        }
        self.fedora
            .resolve(host)
            .map_err(|msg| ToolError::invalid(msg).into())
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
        let fedora = if self.fedora.is_empty() {
            "not configured"
        } else {
            "configured"
        };
        format!(
            "Control a homelab's infrastructure. Proxmox VE ({proxmox}): node and \
             guest (QEMU VM / LXC container) listing, status, and power control. \
             OPNsense ({opnsense}): system status, service control, interfaces, \
             firewall aliases, and gateways. Fedora ({fedora}): system status, \
             systemd service listing/control, journal reads, dnf update listing/ \
             install/remove, and allowlisted config file read/write, via one or \
             more rusty_fedora_agent instances -- one per managed host, selected \
             with each tool's optional `host` argument (defaults to \"baileyai\"). \
             Calling a tool for an unconfigured backend, or naming an unknown \
             Fedora host, returns an error explaining which flags to set or which \
             host ids are known, rather than failing silently or omitting the \
             tool."
        )
    }
}
