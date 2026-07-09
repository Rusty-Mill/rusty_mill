//! LocalAPI status types, mirroring Go's `ipn/ipnstate` at v1.86.
//!
//! Every struct tolerates unknown/missing/null fields (see crate docs); the
//! golden tests in `tests/golden.rs` pin the fields we actually read against
//! JSON captured from a live tailscaled 1.86.2.

use std::collections::BTreeMap;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::{IpPrefix, NodePublic, Rfc3339, StableNodeID, UserID, null_default};

/// Response of `GET /localapi/v0/status` (`ipnstate.Status`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Status {
    pub version: String,
    #[serde(rename = "TUN")]
    pub tun: bool,
    /// The IPN backend state machine state: `"NoState"`, `"NeedsLogin"`,
    /// `"NeedsMachineAuth"`, `"Stopped"`, `"Starting"`, `"Running"`.
    pub backend_state: String,
    pub have_node_key: bool,
    #[serde(rename = "AuthURL")]
    pub auth_url: String,
    #[serde(rename = "TailscaleIPs", deserialize_with = "null_default")]
    pub tailscale_ips: Vec<IpAddr>,
    #[serde(rename = "Self")]
    pub self_: Option<PeerStatus>,
    pub exit_node_status: Option<ExitNodeStatus>,
    #[serde(deserialize_with = "null_default")]
    pub health: Vec<String>,
    #[serde(rename = "MagicDNSSuffix")]
    pub magic_dns_suffix: String,
    pub current_tailnet: Option<TailnetStatus>,
    #[serde(deserialize_with = "null_default")]
    pub cert_domains: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub peer: BTreeMap<NodePublic, PeerStatus>,
    #[serde(deserialize_with = "null_default")]
    pub user: BTreeMap<UserID, UserProfile>,
    pub client_version: Option<serde_json::Value>,
}

impl Status {
    /// Peers sorted by DNS name (hostname as tiebreak), the order the CLI
    /// renders them in.
    pub fn sorted_peers(&self) -> Vec<&PeerStatus> {
        let mut peers: Vec<&PeerStatus> = self.peer.values().collect();
        peers.sort_by(|a, b| (&a.dns_name, &a.host_name).cmp(&(&b.dns_name, &b.host_name)));
        peers
    }
}

/// One node's view in `Status` (`ipnstate.PeerStatus`) — used for both
/// `Self` and each entry of `Peer`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct PeerStatus {
    #[serde(rename = "ID")]
    pub id: StableNodeID,
    pub public_key: Option<NodePublic>,
    pub host_name: String,
    /// Fully qualified DNS name with trailing dot, e.g. `"node1.tailnet.test."`.
    #[serde(rename = "DNSName")]
    pub dns_name: String,
    #[serde(rename = "OS")]
    pub os: String,
    #[serde(rename = "UserID")]
    pub user_id: UserID,
    #[serde(rename = "TailscaleIPs", deserialize_with = "null_default")]
    pub tailscale_ips: Vec<IpAddr>,
    #[serde(rename = "AllowedIPs", deserialize_with = "null_default")]
    pub allowed_ips: Vec<IpPrefix>,
    #[serde(deserialize_with = "null_default")]
    pub tags: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub primary_routes: Vec<IpPrefix>,
    /// Advertised endpoints (`ip:port` strings), if known.
    #[serde(deserialize_with = "null_default")]
    pub addrs: Vec<String>,
    /// Current direct endpoint in use, empty if relayed or inactive.
    pub cur_addr: String,
    /// DERP region code of the peer's home relay, e.g. `"headscale"`.
    pub relay: String,
    pub peer_relay: String,
    pub rx_bytes: i64,
    pub tx_bytes: i64,
    pub created: Rfc3339,
    pub last_write: Rfc3339,
    pub last_seen: Rfc3339,
    pub last_handshake: Rfc3339,
    pub online: bool,
    pub exit_node: bool,
    pub exit_node_option: bool,
    pub active: bool,
    #[serde(rename = "PeerAPIURL", deserialize_with = "null_default")]
    pub peer_api_url: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub capabilities: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub cap_map: BTreeMap<String, serde_json::Value>,
    #[serde(rename = "sshHostKeys", deserialize_with = "null_default")]
    pub ssh_host_keys: Vec<String>,
    pub sharee_node: bool,
    pub in_network_map: bool,
    pub in_magic_sock: bool,
    pub in_engine: bool,
    pub expired: bool,
    pub key_expiry: Option<Rfc3339>,
    pub taildrop_target: i64,
    pub no_file_sharing_reason: String,
}

impl PeerStatus {
    /// The peer's first IPv4 Tailscale address, falling back to any address.
    pub fn primary_ip(&self) -> Option<IpAddr> {
        self.tailscale_ips
            .iter()
            .find(|ip| ip.is_ipv4())
            .or_else(|| self.tailscale_ips.first())
            .copied()
    }

    /// First label of the DNS name, falling back to the hostname.
    pub fn name(&self) -> &str {
        match self.dns_name.split('.').next() {
            Some(label) if !label.is_empty() => label,
            _ => &self.host_name,
        }
    }
}

/// `ipnstate.ExitNodeStatus`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ExitNodeStatus {
    #[serde(rename = "ID")]
    pub id: StableNodeID,
    pub online: bool,
    #[serde(rename = "TailscaleIPs", deserialize_with = "null_default")]
    pub tailscale_ips: Vec<IpPrefix>,
}

/// `ipnstate.TailnetStatus`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct TailnetStatus {
    pub name: String,
    #[serde(rename = "MagicDNSSuffix")]
    pub magic_dns_suffix: String,
    #[serde(rename = "MagicDNSEnabled")]
    pub magic_dns_enabled: bool,
}

/// `tailcfg.UserProfile`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct UserProfile {
    #[serde(rename = "ID")]
    pub id: UserID,
    pub login_name: String,
    pub display_name: String,
    #[serde(rename = "ProfilePicURL")]
    pub profile_pic_url: String,
}

/// Response of `POST /localapi/v0/ping` (`ipnstate.PingResult`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct PingResult {
    #[serde(rename = "IP")]
    pub ip: String,
    #[serde(rename = "NodeIP")]
    pub node_ip: String,
    pub node_name: String,
    /// Non-empty on failure.
    pub err: String,
    pub latency_seconds: f64,
    /// `ip:port` of the direct path used, empty if relayed.
    pub endpoint: String,
    pub peer_relay: String,
    #[serde(rename = "DERPRegionID")]
    pub derp_region_id: i64,
    #[serde(rename = "DERPRegionCode")]
    pub derp_region_code: String,
    #[serde(rename = "PeerAPIPort")]
    pub peer_api_port: Option<u16>,
    #[serde(rename = "IsLocalIP")]
    pub is_local_ip: Option<bool>,
}
