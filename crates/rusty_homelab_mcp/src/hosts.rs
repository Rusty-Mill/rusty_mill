//! The Fedora host registry: which `rusty_fedora_agent` instances this
//! server can reach, keyed by a host id (e.g. `"baileyai"`,
//! `"samba-lxc-101"`). One `rusty_fedora_agent` runs per managed host --
//! its own unit/package/config-path allowlist, scoped to what that host
//! actually does -- so `rusty_homelab_mcp` needs a small map from id to
//! base URL rather than a single backend, once there's more than one.
//!
//! Loaded once at startup from [`HostsFile`] (see
//! [`FedoraHosts::load`]) plus the legacy single `--fedora-agent-url`
//! flag, which always maps to [`DEFAULT_HOST`] for backward compatibility.
//! Never re-read afterward -- explicit configuration, no magic globals,
//! same spirit as the allowlist config `rusty_fedora_agent` itself loads
//! once at startup.

use std::collections::HashMap;
use std::path::Path;

use rusty_fedora::{FedoraAgentClient, FedoraAgentConfig};
use serde::Deserialize;

/// The host id a `fedora_*` tool call resolves to when it omits `host`.
/// Keeps every call made before multi-host support existed working
/// unchanged.
pub const DEFAULT_HOST: &str = "baileyai";

/// One `[hosts.<id>]` table in `hosts.toml`.
#[derive(Debug, Clone, Deserialize)]
struct HostEntry {
    /// The agent's base URL, e.g. `http://192.168.10.192:8765`. Each host
    /// gets its own entry, so nothing requires the agent's HTTP port to
    /// match across hosts -- it's whatever `base_url` says.
    base_url: String,
}

/// The on-disk shape of `hosts.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
struct HostsFile {
    #[serde(default)]
    hosts: HashMap<String, HostEntry>,
}

/// A registry of configured `rusty_fedora_agent` instances, keyed by host
/// id. Cheap to clone -- every [`FedoraAgentClient`] it holds shares its
/// own connection pool underneath.
#[derive(Debug, Clone, Default)]
pub struct FedoraHosts {
    clients: HashMap<String, FedoraAgentClient>,
}

impl FedoraHosts {
    /// True if no host is configured. Distinguishes "Fedora isn't set up
    /// on this server at all" from "that specific host id isn't known" --
    /// the two error [`HomelabServer::fedora`](crate::server::HomelabServer::fedora)
    /// reports differently.
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Build a registry from an optional `hosts.toml` at `path` layered on
    /// top of the legacy `--fedora-agent-url` value, which is inserted
    /// under [`DEFAULT_HOST`] first so an entry of the same id in the file
    /// can override it.
    ///
    /// `Ok(FedoraHosts::default())` if neither is set. A `path` that doesn't
    /// exist, isn't readable, or doesn't parse as valid `hosts.toml` is a
    /// startup error, not a silently-empty registry.
    pub fn load(path: Option<&Path>, legacy_baileyai_url: Option<String>) -> Result<Self, String> {
        let mut clients = HashMap::new();

        if let Some(base_url) = legacy_baileyai_url {
            clients.insert(DEFAULT_HOST.to_string(), build_client(base_url));
        }

        if let Some(path) = path {
            let text = std::fs::read_to_string(path).map_err(|err| {
                format!("failed to read Fedora hosts file {}: {err}", path.display())
            })?;
            let file: HostsFile = toml::from_str(&text).map_err(|err| {
                format!(
                    "malformed Fedora hosts file {}: {err}",
                    path.display()
                )
            })?;
            for (id, entry) in file.hosts {
                clients.insert(id, build_client(entry.base_url));
            }
        }

        Ok(Self { clients })
    }

    /// Resolve a host id -- or `None`, meaning [`DEFAULT_HOST`] -- to its
    /// client. An id that isn't in the registry is a normal error naming
    /// what *is* configured, never a silent fallback to the default host.
    pub fn resolve(&self, host: Option<&str>) -> Result<&FedoraAgentClient, String> {
        let id = host.unwrap_or(DEFAULT_HOST);
        self.clients.get(id).ok_or_else(|| {
            let mut known: Vec<&str> = self.clients.keys().map(String::as_str).collect();
            known.sort_unstable();
            if known.is_empty() {
                format!("unknown Fedora host \"{id}\" -- no hosts are configured")
            } else {
                format!(
                    "unknown Fedora host \"{id}\" -- configured hosts are: {}",
                    known.join(", ")
                )
            }
        })
    }
}

fn build_client(base_url: String) -> FedoraAgentClient {
    FedoraAgentClient::new(FedoraAgentConfig {
        base_url,
        timeout: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_file_and_no_legacy_url_is_an_empty_registry() {
        let hosts = FedoraHosts::load(None, None).expect("no error");
        assert!(hosts.is_empty());
    }

    #[test]
    fn a_legacy_url_alone_resolves_as_the_default_host() {
        let hosts = FedoraHosts::load(None, Some("http://192.168.10.104:8765".to_string()))
            .expect("no error");
        assert!(!hosts.is_empty());
        let client = hosts.resolve(None).expect("baileyai is configured");
        assert!(format!("{client:?}").contains("192.168.10.104"));
    }

    #[test]
    fn omitting_host_resolves_the_same_client_as_naming_it_explicitly() {
        let hosts = FedoraHosts::load(None, Some("http://192.168.10.104:8765".to_string()))
            .expect("no error");
        assert!(hosts.resolve(None).is_ok());
        assert!(hosts.resolve(Some(DEFAULT_HOST)).is_ok());
    }

    #[test]
    fn an_unknown_host_id_is_a_clean_error_naming_the_configured_hosts() {
        let hosts = FedoraHosts::load(None, Some("http://192.168.10.104:8765".to_string()))
            .expect("no error");
        let err = hosts
            .resolve(Some("samba-lxc-101"))
            .expect_err("samba-lxc-101 isn't configured");
        assert!(err.contains("samba-lxc-101"));
        assert!(err.contains("baileyai"));
    }

    #[test]
    fn an_unknown_host_id_against_an_empty_registry_is_a_clean_error() {
        let hosts = FedoraHosts::default();
        let err = hosts
            .resolve(Some("samba-lxc-101"))
            .expect_err("nothing is configured");
        assert!(err.contains("samba-lxc-101"));
    }

    #[test]
    fn a_missing_hosts_file_errors_cleanly_instead_of_panicking() {
        let err = FedoraHosts::load(Some(Path::new("/no/such/hosts.toml")), None)
            .expect_err("the file doesn't exist");
        assert!(err.contains("hosts.toml") || err.contains("no/such"));
    }

    #[test]
    fn an_empty_hosts_file_parses_to_an_empty_registry() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "rusty_homelab_mcp_empty_hosts_test_{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "").expect("write temp file");

        let result = FedoraHosts::load(Some(&path), None);
        let _ = std::fs::remove_file(&path);

        let hosts = result.expect("an empty file is valid TOML with no hosts");
        assert!(hosts.is_empty());
    }

    #[test]
    fn a_malformed_hosts_file_errors_cleanly_instead_of_panicking() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "rusty_homelab_mcp_malformed_hosts_test_{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "not valid toml [[[").expect("write temp file");

        let result = FedoraHosts::load(Some(&path), None);
        let _ = std::fs::remove_file(&path);

        let err = result.expect_err("malformed TOML is an error");
        assert!(err.contains("malformed"));
    }

    #[test]
    fn a_hosts_file_entry_can_override_the_legacy_default_host() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "rusty_homelab_mcp_override_hosts_test_{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "[hosts.baileyai]\nbase_url = \"http://overridden:8765\"\n\n\
             [hosts.samba-lxc-101]\nbase_url = \"http://192.168.10.192:8765\"\n",
        )
        .expect("write temp file");

        let result = FedoraHosts::load(Some(&path), Some("http://original:8765".to_string()));
        let _ = std::fs::remove_file(&path);

        let hosts = result.expect("valid hosts.toml");
        assert!(hosts.resolve(Some("baileyai")).is_ok());
        assert!(hosts.resolve(Some("samba-lxc-101")).is_ok());
        assert!(hosts.resolve(Some("nonexistent")).is_err());
    }
}
