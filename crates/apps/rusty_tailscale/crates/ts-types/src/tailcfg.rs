//! Control-protocol wire types, mirroring Go `tailcfg` at v1.86 (capability
//! version 123). Only the subset the Phase-2 control client sends and reads
//! is modeled; unknown fields are ignored, absent/null fields default (the
//! netmap is a long-lived delta stream and must tolerate server evolution).
//!
//! See PROTOCOL.md for the request/response flow.

use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

use crate::{DiscoPublic, IpPrefix, MachinePublic, NodePublic, Rfc3339, StableNodeID, UserID};

/// The capability version this client advertises (Go
/// `tailcfg.CurrentCapabilityVersion`). Also the Noise handshake's protocol
/// version.
pub const CURRENT_CAPABILITY_VERSION: u16 = 123;

/// A node's integer ID within a tailnet (`tailcfg.NodeID`).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct NodeID(pub i64);

/// Client host metadata (`tailcfg.Hostinfo`); only the fields Headscale
/// reads/echoes are modeled.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Hostinfo {
    #[serde(
        rename = "IPNVersion",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub ipn_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hostname: String,
    #[serde(rename = "OS", default, skip_serializing_if = "String::is_empty")]
    pub os: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routable_ips: Vec<IpPrefix>,
}

/// Authentication material for [`RegisterRequest`] (`tailcfg.RegisterResponseAuth`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RegisterResponseAuth {
    #[serde(rename = "AuthKey", default, skip_serializing_if = "String::is_empty")]
    pub auth_key: String,
}

/// `POST /machine/register` body (`tailcfg.RegisterRequest`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RegisterRequest {
    pub version: u16,
    pub node_key: NodePublic,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_node_key: Option<NodePublic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<RegisterResponseAuth>,
    /// RFC3339; zero time (`0001-01-01T00:00:00Z`) means "no expiry".
    pub expiry: Rfc3339,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub followup: String,
    pub hostinfo: Hostinfo,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ephemeral: bool,
}

/// `POST /machine/register` response (`tailcfg.RegisterResponse`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct RegisterResponse {
    pub node_key_expired: bool,
    pub machine_authorized: bool,
    #[serde(rename = "AuthURL")]
    pub auth_url: String,
    pub error: String,
}

/// `POST /machine/map` body (`tailcfg.MapRequest`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct MapRequest {
    pub version: u16,
    /// "zstd" or "" (none). We always request none (see DESIGN.md).
    pub compress: String,
    pub keep_alive: bool,
    pub node_key: NodePublic,
    pub disco_key: DiscoPublic,
    /// True for the long-poll streaming netmap; false for a one-shot lite
    /// request.
    pub stream: bool,
    pub hostinfo: Hostinfo,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<SocketAddr>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub omit_peers: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
}

/// One frame of the `/machine/map` stream (`tailcfg.MapResponse`).
///
/// Delta semantics: a `None`/absent field means "unchanged"; an empty
/// collection means "explicitly empty". `Peers` is a full replacement;
/// `PeersChanged`/`PeersRemoved` are incremental.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct MapResponse {
    /// Heartbeat frame: no other fields are meaningful when true.
    pub keep_alive: bool,
    pub node: Option<Node>,
    pub peers: Option<Vec<Node>>,
    pub peers_changed: Option<Vec<Node>>,
    pub peers_removed: Option<Vec<NodeID>>,
    /// User profiles referenced by nodes' `User` field. Delta: `None` means
    /// unchanged, so accumulate across frames.
    pub user_profiles: Option<Vec<crate::UserProfile>>,
    pub domain: String,
    #[serde(rename = "DERPMap")]
    pub derp_map: Option<serde_json::Value>,
    #[serde(rename = "DNSConfig")]
    pub dns_config: Option<serde_json::Value>,
    /// Legacy flat packet filter (`tailcfg.MapResponse.PacketFilter`). Older
    /// control servers send this; modern Headscale sends `PacketFilters`.
    pub packet_filter: Option<Vec<FilterRule>>,
    /// Named packet-filter sets (`tailcfg.MapResponse.PacketFilters`), e.g.
    /// `{"base": [rule, …]}`. The effective filter is the union of all sets.
    pub packet_filters: Option<std::collections::BTreeMap<String, Vec<FilterRule>>>,
}

/// One ACL rule in the compiled packet filter (`tailcfg.FilterRule`).
///
/// An allow-list entry: a packet is permitted if its source matches one of
/// `src_ips` and its destination matches one of `dst_ports` (and, if
/// `ip_proto` is non-empty, its protocol is listed).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct FilterRule {
    #[serde(rename = "SrcIPs")]
    pub src_ips: Vec<String>,
    pub dst_ports: Vec<NetPortRange>,
    /// IP protocol numbers this rule applies to; empty means all protocols.
    #[serde(rename = "IPProto")]
    pub ip_proto: Vec<i32>,
}

/// A destination CIDR + port range (`tailcfg.NetPortRange`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct NetPortRange {
    /// Destination CIDR, bare IP, or `"*"` for any.
    #[serde(rename = "IP")]
    pub ip: String,
    pub ports: PortRange,
}

/// An inclusive port range (`tailcfg.PortRange`); `{0, 65535}` means all ports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct PortRange {
    pub first: u16,
    pub last: u16,
}

/// A node in the netmap (`tailcfg.Node`); Phase-2 subset.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Node {
    #[serde(rename = "ID")]
    pub id: NodeID,
    #[serde(rename = "StableID")]
    pub stable_id: StableNodeID,
    pub name: String,
    pub user: UserID,
    pub key: Option<NodePublic>,
    pub key_expiry: Rfc3339,
    pub machine: Option<MachinePublic>,
    pub disco_key: Option<DiscoPublic>,
    /// Host metadata (OS, hostname). Absent on some frames.
    pub hostinfo: Option<Hostinfo>,
    #[serde(deserialize_with = "crate::null_default")]
    pub addresses: Vec<IpPrefix>,
    #[serde(rename = "AllowedIPs", deserialize_with = "crate::null_default")]
    pub allowed_ips: Vec<IpPrefix>,
    #[serde(deserialize_with = "crate::null_default")]
    pub endpoints: Vec<SocketAddr>,
    /// DERP region ID of the node's home relay (0 = none).
    #[serde(rename = "HomeDERP")]
    pub home_derp: i64,
    pub online: Option<bool>,
    pub created: Rfc3339,
}

impl Node {
    /// The node's first IPv4 tailnet address, if any.
    pub fn primary_ip(&self) -> Option<IpAddr> {
        self.addresses
            .iter()
            .map(|p| p.addr)
            .find(IpAddr::is_ipv4)
            .or_else(|| self.addresses.first().map(|p| p.addr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_request_omits_empty_optionals() {
        let req = RegisterRequest {
            version: CURRENT_CAPABILITY_VERSION,
            node_key: NodePublic([0x11; 32]),
            old_node_key: None,
            auth: Some(RegisterResponseAuth {
                auth_key: "abc".into(),
            }),
            expiry: Rfc3339("0001-01-01T00:00:00Z".into()),
            followup: String::new(),
            hostinfo: Hostinfo {
                hostname: "n1".into(),
                os: "linux".into(),
                ..Default::default()
            },
            ephemeral: false,
        };
        let v: serde_json::Value = serde_json::to_value(&req).unwrap();
        assert_eq!(v["Version"], 123);
        assert_eq!(v["Auth"]["AuthKey"], "abc");
        assert!(v.get("OldNodeKey").is_none(), "None optional omitted");
        assert!(v.get("Followup").is_none(), "empty followup omitted");
        assert!(v.get("Ephemeral").is_none(), "false ephemeral omitted");
        assert_eq!(v["Hostinfo"]["Hostname"], "n1");
    }

    #[test]
    fn map_response_delta_fields() {
        // Heartbeat frame.
        let ka: MapResponse = serde_json::from_str(r#"{"KeepAlive": true}"#).unwrap();
        assert!(ka.keep_alive);
        assert!(ka.node.is_none() && ka.peers.is_none());

        // A frame carrying peers.
        let frame = r#"{
            "Node": {"ID": 1, "Name": "self.ts.", "Addresses": ["100.64.0.1/32"]},
            "Peers": [{"ID": 2, "Name": "peer.ts.", "Addresses": ["100.64.0.2/32"], "Online": true}],
            "Domain": "ts.test"
        }"#;
        let mr: MapResponse = serde_json::from_str(frame).unwrap();
        assert!(!mr.keep_alive);
        assert_eq!(mr.node.unwrap().id, NodeID(1));
        let peers = mr.peers.unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].primary_ip().unwrap().to_string(), "100.64.0.2");
        assert_eq!(peers[0].online, Some(true));
        assert_eq!(mr.domain, "ts.test");
    }

    #[test]
    fn parses_headscale_packet_filters_and_user_profiles() {
        // The exact shape a live Headscale (no ACL policy) sends, captured
        // from the /machine/map stream.
        let frame = r#"{
            "Node": {"ID": 1, "Name": "self.ts.", "Addresses": ["100.64.0.1/32"]},
            "PacketFilters": {"base": [
                {"SrcIPs": ["*"], "DstPorts": [{"IP": "*", "Ports": {"First": 0, "Last": 65535}}]}
            ]},
            "UserProfiles": [{"ID": 1, "LoginName": "interop", "DisplayName": "Interop"}]
        }"#;
        let mr: MapResponse = serde_json::from_str(frame).unwrap();
        let filters = mr.packet_filters.expect("PacketFilters present");
        let base = &filters["base"];
        assert_eq!(base.len(), 1);
        assert_eq!(base[0].src_ips, vec!["*".to_string()]);
        assert_eq!(base[0].dst_ports[0].ip, "*");
        assert_eq!(base[0].dst_ports[0].ports.last, 65535);
        assert!(base[0].ip_proto.is_empty(), "no proto restriction");
        assert!(mr.packet_filter.is_none(), "legacy field absent");

        let users = mr.user_profiles.expect("UserProfiles present");
        assert_eq!(users[0].id, UserID(1));
        assert_eq!(users[0].login_name, "interop");
    }

    #[test]
    fn map_response_is_panic_free_on_garbage() {
        for g in ["", "null", "[]", "{\"Peers\": \"x\"}", "{\"Node\": 5}"] {
            let _ = serde_json::from_str::<MapResponse>(g);
        }
    }
}
