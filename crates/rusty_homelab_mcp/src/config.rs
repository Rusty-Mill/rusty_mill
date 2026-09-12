//! Extra CLI flags: where to find Proxmox and/or OPNsense, and how to
//! require bearer-token authorization on the HTTP transport, on top of
//! the scaffold's standard transport/logging flags.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use rusty_mcp::auth::{AuthConfig, StaticTokenValidator, VerifiedToken};
use rusty_opnsense::OpnsenseConfig;
use rusty_proxmox::ProxmoxConfig;

use crate::hosts::FedoraHosts;

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

    /// `rusty_fedora_agent` base URL, e.g. `http://100.x.y.z:8765`, for the
    /// default host id ("baileyai"). The agent has no authentication of
    /// its own, so this should always be a private/Tailscale address.
    /// Kept as its own flag for backward compatibility -- an entry for
    /// "baileyai" in `--fedora-hosts-file` takes precedence if both are
    /// set.
    #[arg(long, env = "FEDORA_AGENT_URL")]
    pub fedora_agent_url: Option<String>,

    /// Path to a `hosts.toml` registry mapping additional Fedora host ids
    /// (e.g. `samba-lxc-101`) to their own `rusty_fedora_agent` base URLs.
    /// Every `fedora_*` tool takes an optional `host` argument resolved
    /// against this registry, defaulting to "baileyai" if omitted. See
    /// `deploy/hosts.toml.example`.
    #[arg(long, env = "FEDORA_HOSTS_FILE")]
    pub fedora_hosts_file: Option<PathBuf>,

    /// Shared-secret bearer token required on every request to the HTTP
    /// endpoint (HTTP transport only; stdio has no use for it). Unset
    /// leaves the endpoint open, which is only safe behind a gateway that
    /// already authenticates callers -- given this server can power off
    /// Proxmox VMs, delete OPNsense firewall rules, and write Fedora
    /// config files, set this for any deployment reachable beyond
    /// localhost.
    ///
    /// This is a static shared secret, not an OAuth token issued by a
    /// real authorization server -- adequate for a homelab's own
    /// clients, not a substitute for real token issuance at larger
    /// scale.
    #[arg(long, env = "HOMELAB_MCP_AUTH_TOKEN")]
    pub auth_token: Option<String>,

    /// Canonical resource URL bound into the required token's audience,
    /// e.g. `https://homelab-mcp.example.com/mcp`. Only used when
    /// `--auth-token` is set. Defaults to `http://<bind><path>` (this
    /// server's own `--bind`/`--path`) -- override it when the server
    /// sits behind a reverse proxy under a different public URL.
    #[arg(long, env = "HOMELAB_MCP_AUTH_RESOURCE_URL")]
    pub auth_resource_url: Option<String>,
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

    /// Builds the Fedora host registry from `--fedora-agent-url` and/or
    /// `--fedora-hosts-file`. An empty registry (neither flag set) is not
    /// an error -- Fedora is simply unconfigured, same as the other
    /// backends. A `--fedora-hosts-file` that's missing or malformed is a
    /// startup error, not a silently-empty registry.
    pub fn fedora_hosts(&self) -> Result<FedoraHosts, String> {
        FedoraHosts::load(
            self.fedora_hosts_file.as_deref(),
            self.fedora_agent_url.clone(),
        )
    }

    /// Builds OAuth 2.1 resource-server authorization from `--auth-token`,
    /// or `Ok(None)` if it's unset -- an HTTP endpoint with no
    /// `--auth-token` is left open, same as every other optional backend
    /// here.
    ///
    /// The audience bound into the required token defaults to
    /// `http://<bind><path>` (this server's own `--bind`/`--path`) unless
    /// `--auth-resource-url` overrides it.
    pub fn auth_config(&self) -> Result<Option<Arc<AuthConfig>>, String> {
        let Some(token) = &self.auth_token else {
            return Ok(None);
        };

        let resource = self
            .auth_resource_url
            .clone()
            .unwrap_or_else(|| format!("http://{}{}", self.mcp.bind, self.mcp.path));

        let validator = StaticTokenValidator::new()
            .with_token(token.clone(), VerifiedToken::new([resource.clone()]));

        let auth = AuthConfig::new(&resource, Arc::new(validator)).map_err(|err| {
            format!("--auth-resource-url `{resource}` is not usable as a resource URI: {err}")
        })?;

        Ok(Some(Arc::new(auth)))
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
        assert!(cli.fedora_hosts().expect("no error").is_empty());
    }

    #[test]
    fn a_fedora_agent_url_registers_the_default_host() {
        let cli =
            HomelabCli::try_parse_from(["homelab", "--fedora-agent-url", "http://100.64.0.1:8765"])
                .expect("parses");

        let hosts = cli.fedora_hosts().expect("fedora is configured");
        assert!(hosts.resolve(None).is_ok());
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

    #[test]
    fn no_auth_token_flag_leaves_the_http_endpoint_open() {
        let cli = HomelabCli::try_parse_from(["homelab", "--transport", "http"]).expect("parses");
        assert!(cli.auth_config().expect("no error").is_none());
    }

    #[test]
    fn an_auth_token_flag_enables_bearer_authorization_on_http_config() {
        let cli = HomelabCli::try_parse_from([
            "homelab",
            "--transport",
            "http",
            "--auth-token",
            "s3cr3t",
        ])
        .expect("parses");

        let auth = cli
            .auth_config()
            .expect("no error")
            .expect("auth is configured");
        assert_eq!(auth.resource(), "http://127.0.0.1:8080/mcp");

        let mut server_config: rusty_mcp::ServerConfig = cli.mcp.into();
        match &mut server_config.transport {
            rusty_mcp::Transport::Http(http_config) => http_config.auth = Some(auth),
            rusty_mcp::Transport::Stdio => panic!("expected http transport"),
        }

        match server_config.transport {
            rusty_mcp::Transport::Http(http_config) => assert!(http_config.auth.is_some()),
            rusty_mcp::Transport::Stdio => panic!("expected http transport"),
        }
    }

    #[test]
    fn an_auth_resource_url_override_is_used_as_the_audience() {
        let cli = HomelabCli::try_parse_from([
            "homelab",
            "--auth-token",
            "s3cr3t",
            "--auth-resource-url",
            "https://homelab-mcp.example.com/mcp",
        ])
        .expect("parses");

        let auth = cli
            .auth_config()
            .expect("no error")
            .expect("auth is configured");
        assert_eq!(auth.resource(), "https://homelab-mcp.example.com/mcp");
    }
}
