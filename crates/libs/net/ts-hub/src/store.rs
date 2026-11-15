//! The hub's node table: who registered, their keys and endpoints, and the
//! netmap built from them. Pure data, no I/O. [`NodeStore`] is the seam a
//! persistent table plugs into; [`MemoryStore`] is the in-memory one.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use ts_types::tailcfg::{Hostinfo, MapResponse, Node, NodeID};
use ts_types::{
    DiscoPublic, IpPrefix, MachinePublic, NodePublic, Rfc3339, StableNodeID, UserID, UserProfile,
};

/// The all-zero time Tailscale uses for "never expires".
const ZERO_TIME: &str = "0001-01-01T00:00:00Z";

/// One registered node.
#[derive(Debug, Clone)]
pub struct NodeRec {
    pub id: i64,
    pub login: String,
    pub hostname: String,
    pub machine: MachinePublic,
    pub node_key: NodePublic,
    pub disco: Option<DiscoPublic>,
    pub endpoints: Vec<SocketAddr>,
    pub ip: Ipv4Addr,
}

/// Why a registration was refused.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    #[error("unknown or empty pre-auth key")]
    BadAuthKey,
    #[error("the node table is full")]
    Full,
}

/// What the hub needs from a node table.
pub trait NodeStore: Send + 'static {
    /// Admits `login` for a node that registers with `key`.
    fn add_preauth(&mut self, key: &str, login: &str);

    /// Registers (or re-registers) the machine, assigning a tailnet IPv4
    /// address on first sight. Keys are reusable, as `headscale preauthkeys
    /// create --reusable`.
    fn register(
        &mut self,
        machine: MachinePublic,
        node_key: NodePublic,
        hostname: &str,
        auth_key: &str,
    ) -> Result<(), RegisterError>;

    /// Whether `key` belongs to a registered node (the relay admits only
    /// these).
    fn has_node_key(&self, key: &NodePublic) -> bool;

    /// Records a node's disco key and endpoints; returns whether anything
    /// changed (so the caller can wake the netmap streams).
    fn update(
        &mut self,
        machine: &MachinePublic,
        disco: DiscoPublic,
        endpoints: Vec<SocketAddr>,
    ) -> bool;

    /// The full netmap for `machine`: itself plus every other node, or
    /// `None` if the machine never registered.
    fn netmap(&self, machine: &MachinePublic, omit_peers: bool) -> Option<MapResponse>;
}

/// Pre-auth keys and registered nodes, held in memory.
#[derive(Debug, Default)]
pub struct MemoryStore {
    /// Pre-auth key to the login name it admits.
    preauth: HashMap<String, String>,
    nodes: Vec<NodeRec>,
}

impl NodeStore for MemoryStore {
    fn add_preauth(&mut self, key: &str, login: &str) {
        self.preauth.insert(key.to_string(), login.to_string());
    }

    fn register(
        &mut self,
        machine: MachinePublic,
        node_key: NodePublic,
        hostname: &str,
        auth_key: &str,
    ) -> Result<(), RegisterError> {
        let login = self
            .preauth
            .get(auth_key)
            .cloned()
            .ok_or(RegisterError::BadAuthKey)?;
        if let Some(i) = self.nodes.iter().position(|n| n.machine == machine) {
            let n = &mut self.nodes[i];
            n.node_key = node_key;
            n.login = login;
            n.hostname = hostname.to_string();
            return Ok(());
        }
        let id = self.nodes.len() as i64 + 1;
        let host = u8::try_from(id).map_err(|_| RegisterError::Full)?;
        self.nodes.push(NodeRec {
            id,
            login,
            hostname: hostname.to_string(),
            machine,
            node_key,
            disco: None,
            endpoints: Vec::new(),
            ip: Ipv4Addr::new(100, 64, 0, host),
        });
        Ok(())
    }

    fn has_node_key(&self, key: &NodePublic) -> bool {
        self.nodes.iter().any(|n| &n.node_key == key)
    }

    fn update(
        &mut self,
        machine: &MachinePublic,
        disco: DiscoPublic,
        endpoints: Vec<SocketAddr>,
    ) -> bool {
        let Some(n) = self.nodes.iter_mut().find(|n| &n.machine == machine) else {
            return false;
        };
        let changed = n.disco != Some(disco) || n.endpoints != endpoints;
        n.disco = Some(disco);
        n.endpoints = endpoints;
        changed
    }

    fn netmap(&self, machine: &MachinePublic, omit_peers: bool) -> Option<MapResponse> {
        let me = self.nodes.iter().find(|n| &n.machine == machine)?;
        let peers = if omit_peers {
            None
        } else {
            Some(
                self.nodes
                    .iter()
                    .filter(|n| n.machine != me.machine)
                    .map(wire_node)
                    .collect(),
            )
        };
        Some(MapResponse {
            node: Some(wire_node(me)),
            peers,
            user_profiles: Some(self.profiles()),
            domain: "mesh".to_string(),
            ..Default::default()
        })
    }
}

impl MemoryStore {
    fn profiles(&self) -> Vec<UserProfile> {
        let mut logins: Vec<&str> = self.nodes.iter().map(|n| n.login.as_str()).collect();
        logins.sort_unstable();
        logins.dedup();
        logins
            .into_iter()
            .map(|l| UserProfile {
                id: user_id(l),
                login_name: l.to_string(),
                display_name: l.to_string(),
                ..Default::default()
            })
            .collect()
    }
}

/// A stable small user id derived from the login name.
fn user_id(login: &str) -> UserID {
    UserID(
        login
            .bytes()
            .fold(1u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b))) as i64
            + 1,
    )
}

fn wire_node(n: &NodeRec) -> Node {
    let addr = IpPrefix {
        addr: IpAddr::V4(n.ip),
        bits: 32,
    };
    Node {
        id: NodeID(n.id),
        stable_id: StableNodeID(format!("n{}", n.id)),
        name: format!("{}.mesh.", n.hostname),
        user: user_id(&n.login),
        key: Some(n.node_key),
        key_expiry: Rfc3339(ZERO_TIME.to_string()),
        machine: Some(n.machine),
        disco_key: n.disco,
        addresses: vec![addr],
        allowed_ips: vec![addr],
        endpoints: n.endpoints.clone(),
        online: Some(true),
        machine_authorized: true,
        home_derp: 1,
        hostinfo: Some(Hostinfo {
            hostname: n.hostname.clone(),
            ..Default::default()
        }),
        created: Rfc3339(ZERO_TIME.to_string()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(b: u8) -> MachinePublic {
        MachinePublic([b; 32])
    }
    fn k(b: u8) -> NodePublic {
        NodePublic([b; 32])
    }

    fn ip_of(s: &MemoryStore, b: u8) -> u8 {
        let node = s.netmap(&m(b), true).unwrap().node.unwrap();
        match node.addresses[0].addr {
            IpAddr::V4(ip) => ip.octets()[3],
            IpAddr::V6(_) => unreachable!("the store hands out IPv4 only"),
        }
    }

    #[test]
    fn register_needs_a_known_key_and_assigns_addresses() {
        let mut s = MemoryStore::default();
        s.add_preauth("k1", "alice");
        assert_eq!(
            s.register(m(1), k(1), "a", "nope").unwrap_err(),
            RegisterError::BadAuthKey
        );
        s.register(m(1), k(1), "a", "k1").unwrap();
        s.register(m(2), k(2), "b", "k1").unwrap();
        assert_eq!(ip_of(&s, 1), 1);
        assert_eq!(ip_of(&s, 2), 2);
        // Registering again keeps the address.
        s.register(m(1), k(9), "a", "k1").unwrap();
        assert_eq!(ip_of(&s, 1), 1);
    }

    #[test]
    fn netmap_lists_the_other_nodes_with_their_disco_keys() {
        let mut s = MemoryStore::default();
        s.add_preauth("k1", "alice");
        s.register(m(1), k(1), "a", "k1").unwrap();
        s.register(m(2), k(2), "b", "k1").unwrap();
        let ep: SocketAddr = "192.0.2.1:41641".parse().unwrap();
        assert!(s.update(&m(2), DiscoPublic([7; 32]), vec![ep]));
        assert!(
            !s.update(&m(2), DiscoPublic([7; 32]), vec![ep]),
            "unchanged"
        );
        let map = s.netmap(&m(1), false).unwrap();
        let peers = map.peers.unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].disco_key, Some(DiscoPublic([7; 32])));
        assert_eq!(peers[0].endpoints, vec![ep]);
        assert!(s.netmap(&m(1), true).unwrap().peers.is_none());
        assert!(s.netmap(&m(9), false).is_none());
    }

    #[test]
    fn the_table_is_full_after_255_nodes() {
        let mut s = MemoryStore::default();
        s.add_preauth("k1", "alice");
        for b in 1..=255u8 {
            s.register(MachinePublic([b; 32]), NodePublic([b; 32]), "n", "k1")
                .unwrap_or_else(|e| panic!("node {b}: {e}"));
        }
        let extra = s.register(MachinePublic([0; 32]), NodePublic([0; 32]), "n", "k1");
        assert_eq!(extra, Err(RegisterError::Full));
    }

    #[test]
    fn a_netmap_node_has_the_wire_shape_stock_clients_need() {
        let mut s = MemoryStore::default();
        s.add_preauth("k1", "alice");
        s.register(m(1), k(1), "a", "k1").unwrap();
        let node = s.netmap(&m(1), true).unwrap().node.unwrap();
        let json = serde_json::to_value(&node).unwrap();
        assert_eq!(json["Name"], "a.mesh.");
        assert_eq!(json["Addresses"][0], "100.64.0.1/32");
        assert_eq!(json["MachineAuthorized"], true);
        assert_eq!(json["HomeDERP"], 1);
        assert_eq!(json["Hostinfo"]["Hostname"], "a");
    }
}
