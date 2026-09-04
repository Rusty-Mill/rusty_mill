//! Extra CLI flags: where to find Proxmox and/or OPNsense, on top of the
//! scaffold's standard transport/logging flags.

use clap::Parser;
use rusty_fedora::FedoraAgentConfig;
use rusty_opnsense::OpnsenseConfig;
use rusty_proxmox::ProxmoxConfig;

/// This server's CLI: the scaffold's standard flags (`--transport`,
/// `--bind`, `--log`, ...) plus where to find Proxmox, OPNsense, and/or a
/// `rusty_fedora_agent` instance.
///
/// Every backend is optional -- set only the URL/credential flags for
/// whichever homelab service is reachable from wherever this server runs.
/// Every tool is always listed regardless of what's configured; a tool
/// call against an unconfigured backend fails with a protocol error naming
/// the flags to set, rather than the tool silently vanishing from the list
/// (see [`crate::server::HomelabServer::proxmox`]/`::opnsense`/`::fedora`).
#[derive(Debug, Clone, Parser)]
pub struct HomelabCli {
    /// The scaffold's own flags: transport, bind address, logging, etc.
    #[command(flatten)]
    pub mcp: rusty_mcp::Cli,

    /// Proxmox VE API base URL, e.g. `https://pve.lan:8006`.
    #[arg(long, env = "PROXMOX_URL")]
    pub proxmox_url: Option<String>,

    /// Proxmox API token, `<user>@<realm>!<token-id>` form.
    #[arg(long, env = "PROXMOX_TOKEN_ID")]
    pub proxmox_token_id: Option<String>,

    /// Proxmox API token secret.
    #[arg(long, env = "PROXMOX_TOKEN_SECRET")]
    pub proxmox_token_secret: Option<String>,

    /// Skip TLS certificate verification for the Proxmox API (its default
    /// self-signed certificate, typical for a homelab). Never set this for
    /// a host reachable outside a trusted network.
    #[arg(long, env = "PROXMOX_INSECURE")]
    pub proxmox_insecure: bool,

    /// OPNsense API base URL, e.g. `https://opnsense.lan`.
    #[arg(long, env = "OPNSENSE_URL")]
    pub opnsense_url: Option<String>,

    /// OPNsense API key.
    #[arg(long, env = "OPNSENSE_KEY")]
    pub opnsense_key: Option<String>,

    /// OPNsense API secret.
    #[arg(long, env = "OPNSENSE_SECRET")]
    pub opnsense_secret: Option<String>,

    /// Skip TLS certificate verification for the OPNsense API (its default
    /// self-signed certificate, typical for a homelab). Never set this for
    /// a host reachable outside a trusted network.
    #[arg(long, env = "OPNSENSE_INSECURE")]
    pub opnsense_insecure: bool,

    /// `rusty_fedora_agent` base URL, e.g. `http://100.x.y.z:8765`. The
    /// agent has no authentication of its own, so this should always be a
    /// private/Tailscale address.
    #[arg(long, env = "FEDORA_AGENT_URL")]
    pub fedora_agent_url: Option<String>,
}

impl HomelabCli {
    /// Builds a Proxmox client config if `--proxmox-url` and both token
    /// flags are set; `Ok(None)` if none of the three are set at all.
    ///
    /// A URL set with a credential missing is reported as an error rather
    /// than treated as "unconfigured": that combination is much more likely
    /// a typo'd flag than an intentionally half-configured backend.
    pub fn proxmox_config(&self) -> Result<Option<ProxmoxConfig>, String> {
        match (
            &self.proxmox_url,
            &self.proxmox_token_id,
            &self.proxmox_token_secret,
        ) {
            (None, None, None) => Ok(None),
            (Some(base_url), Some(token_id), Some(token_secret)) => Ok(Some(ProxmoxConfig {
                base_url: base_url.clone(),
                token_id: token_id.clone(),
                token_secret: token_secret.clone(),
                insecure: self.proxmox_insecure,
                timeout: None,
            })),
            _ => Err(
                "--proxmox-url requires --proxmox-token-id and --proxmox-token-secret \
                 (or PROXMOX_URL/PROXMOX_TOKEN_ID/PROXMOX_TOKEN_SECRET) to all be set"
                    .to_string(),
            ),
        }
    }

    /// Builds an OPNsense client config if `--opnsense-url` and both key
    /// flags are set; `Ok(None)` if none of the three are set at all.
    pub fn opnsense_config(&self) -> Result<Option<OpnsenseConfig>, String> {
        match (
            &self.opnsense_url,
            &self.opnsense_key,
            &self.opnsense_secret,
        ) {
            (None, None, None) => Ok(None),
            (Some(base_url), Some(key), Some(secret)) => Ok(Some(OpnsenseConfig {
                base_url: base_url.clone(),
                key: key.clone(),
                secret: secret.clone(),
                insecure: self.opnsense_insecure,
                timeout: None,
            })),
            _ => Err(
                "--opnsense-url requires --opnsense-key and --opnsense-secret \
                 (or OPNSENSE_URL/OPNSENSE_KEY/OPNSENSE_SECRET) to all be set"
                    .to_string(),
            ),
        }
    }

    /// Builds a Fedora agent client config if `--fedora-agent-url` is set;
    /// `None` otherwise. No credential pair to cross-check -- the agent
    /// has no auth of its own, so there's nothing a half-set combination
    /// could mean.
    pub fn fedora_config(&self) -> Option<FedoraAgentConfig> {
        self.fedora_agent_url
            .clone()
            .map(|base_url| FedoraAgentConfig {
                base_url,
                timeout: None,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_backend_flags_means_no_backends() {
        let cli = HomelabCli::try_parse_from(["homelab"]).expect("parses");
        assert!(cli.proxmox_config().expect("no error").is_none());
        assert!(cli.opnsense_config().expect("no error").is_none());
        assert!(cli.fedora_config().is_none());
    }

    #[test]
    fn a_fedora_agent_url_builds_a_config() {
        let cli =
            HomelabCli::try_parse_from(["homelab", "--fedora-agent-url", "http://100.64.0.1:8765"])
                .expect("parses");

        let config = cli.fedora_config().expect("fedora is configured");
        assert_eq!(config.base_url, "http://100.64.0.1:8765");
    }

    #[test]
    fn a_fully_specified_proxmox_backend_builds_a_config() {
        let cli = HomelabCli::try_parse_from([
            "homelab",
            "--proxmox-url",
            "https://pve.lan:8006",
            "--proxmox-token-id",
            "automation@pve!mcp",
            "--proxmox-token-secret",
            "secret",
            "--proxmox-insecure",
        ])
        .expect("parses");

        let config = cli
            .proxmox_config()
            .expect("no error")
            .expect("proxmox is configured");
        assert_eq!(config.base_url, "https://pve.lan:8006");
        assert!(config.insecure);
    }

    #[test]
    fn a_fully_specified_opnsense_backend_builds_a_config() {
        let cli = HomelabCli::try_parse_from([
            "homelab",
            "--opnsense-url",
            "https://opnsense.lan",
            "--opnsense-key",
            "key",
            "--opnsense-secret",
            "secret",
        ])
        .expect("parses");

        let config = cli
            .opnsense_config()
            .expect("no error")
            .expect("opnsense is configured");
        assert_eq!(config.base_url, "https://opnsense.lan");
        assert!(!config.insecure);
    }

    #[test]
    fn a_partially_specified_backend_is_an_error() {
        let cli = HomelabCli::try_parse_from(["homelab", "--proxmox-url", "https://pve.lan:8006"])
            .expect("parses");

        assert!(cli.proxmox_config().is_err());
    }
}
