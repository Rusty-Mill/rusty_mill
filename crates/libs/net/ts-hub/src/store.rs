//! The hub's node table: who registered, their keys and endpoints, and the
//! netmap built from them. Pure data and an explicit clock (`now`, Unix
//! seconds), no I/O. [`NodeStore`] is the seam a persistent table plugs into;
//! [`MemoryStore`] is the in-memory one and [`FileStore`](crate::FileStore)
//! saves it to disk.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use serde::{Deserialize, Serialize};
use ts_types::tailcfg::{Hostinfo, MapResponse, Node, NodeID};
use ts_types::{
    DiscoPublic, IpPrefix, MachinePublic, NodePublic, Rfc3339, StableNodeID, UserID, UserProfile,
};

/// The all-zero time Tailscale uses for "never expires".
const ZERO_TIME: &str = "0001-01-01T00:00:00Z";
/// The login of the user every tagged node belongs to.
pub(crate) const TAGGED_LOGIN: &str = "tagged-devices";
/// The one tag the hub gives tagged nodes.
const TAG: &str = "tag:mesh";
/// Longest DNS label.
const MAX_LABEL: usize = 63;

/// How a pre-auth key may be used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyPolicy {
    /// Seconds from creation until the key stops working; `None` never.
    pub ttl: Option<u64>,
    /// Whether more than one machine may register with the key.
    pub reusable: bool,
    /// A server's name, for a tagged key: the one machine that registers with
    /// it is `NAME.mesh`, principal `ts-node:NAME.mesh`, and nobody else can
    /// take the name. `None` makes a person's device, principal `ts:LOGIN`.
    pub tagged: Option<String>,
}

impl KeyPolicy {
    /// A key any number of machines may use, forever.
    pub const REUSABLE: Self = Self {
        ttl: None,
        reusable: true,
        tagged: None,
    };
}

/// One registered node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRec {
    pub id: i64,
    pub login: String,
    pub hostname: String,
    pub machine: MachinePublic,
    pub node_key: NodePublic,
    /// Re-sent with the node's next map poll, so not saved.
    #[serde(skip)]
    pub disco: Option<DiscoPublic>,
    #[serde(skip)]
    pub endpoints: Vec<SocketAddr>,
    pub ip: Ipv4Addr,
    /// Registered with a tagged key.
    #[serde(default)]
    pub tagged: bool,
    /// Unix seconds after which the node must register again; `None` never.
    #[serde(default)]
    pub expires: Option<u64>,
}

/// Why a store operation was refused.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    #[error("unknown or empty pre-auth key")]
    BadAuthKey,
    #[error("pre-auth key has expired")]
    ExpiredAuthKey,
    #[error("this machine was revoked")]
    Revoked,
    #[error("invalid pre-auth key: {0}")]
    InvalidKey(String),
    #[error("the name `{0}` is taken")]
    NameTaken(String),
    #[error("the node table is full")]
    Full,
    #[error("could not save the node table: {0}")]
    Storage(String),
}

/// What the hub needs from a node table.
pub trait NodeStore: Send + 'static {
    /// Admits `login` for a node that registers with `key`, under `policy`.
    fn add_preauth(
        &mut self,
        key: &str,
        login: &str,
        policy: KeyPolicy,
        now: u64,
    ) -> Result<(), RegisterError>;

    /// Makes `keys` the whole set of file keys, all or nothing: file keys not
    /// listed are removed, listed ones are added (one already present with
    /// the same settings is kept, spent mark and all). Keys added with
    /// [`add_preauth`](Self::add_preauth) are left alone. The mark is saved
    /// with the key, so a hub that restarts after the file dropped a key
    /// still drops it. If any key is refused, nothing changes.
    fn set_file_keys(
        &mut self,
        keys: Vec<(String, String, KeyPolicy)>,
        now: u64,
    ) -> Result<(), RegisterError>;

    /// Registers (or re-registers) the machine, assigning a tailnet IPv4
    /// address on first sight. The hostname is made a unique DNS label. A
    /// single-use key is spent by its first machine and refused to others.
    fn register(
        &mut self,
        machine: MachinePublic,
        node_key: NodePublic,
        hostname: &str,
        auth_key: &str,
        now: u64,
    ) -> Result<(), RegisterError>;

    /// Forgets a pre-auth key, so nobody can register with it again. Nodes
    /// already registered with it stay; revoke their machines to remove
    /// them. `false` if there was no such key.
    fn remove_preauth(&mut self, key: &str) -> Result<bool, RegisterError>;

    /// Removes the machine from the table and bans it: it cannot register
    /// again, whatever key it holds. The outcome carries the node key so the
    /// caller can close the relay session even when saving failed (the
    /// revoke stands in memory regardless).
    fn revoke(&mut self, machine: &MachinePublic) -> Revocation;

    /// Whether `key` belongs to a registered, unexpired node (the relay
    /// admits only these).
    fn has_node_key(&self, key: &NodePublic, now: u64) -> bool;

    /// Records a node's disco key and endpoints; returns whether anything
    /// changed (so the caller can wake the netmap streams).
    fn update(
        &mut self,
        machine: &MachinePublic,
        disco: DiscoPublic,
        endpoints: Vec<SocketAddr>,
    ) -> bool;

    /// The full netmap for `machine`: itself plus every other unexpired
    /// node, or `None` if the machine is not registered or has expired.
    fn netmap(&self, machine: &MachinePublic, omit_peers: bool, now: u64) -> Option<MapResponse>;

    /// The table as saved bytes, for a standby to copy. A store that cannot
    /// be copied says so and the hub refuses to replicate.
    fn export(&self) -> Result<Vec<u8>, RegisterError> {
        Err(RegisterError::Storage(
            "this store cannot be replicated".into(),
        ))
    }
}

/// What [`NodeStore::revoke`] did.
#[derive(Debug, PartialEq, Eq)]
pub struct Revocation {
    /// The revoked node's key, if the machine was registered.
    pub node_key: Option<NodePublic>,
    /// Whether the new table was saved.
    pub saved: Result<(), RegisterError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PreauthRec {
    login: String,
    expires: Option<u64>,
    reusable: bool,
    #[serde(default)]
    tagged: Option<String>,
    /// The machine that spent a single-use key.
    used_by: Option<MachinePublic>,
    /// Put here by [`NodeStore::set_file_keys`]: the keys file owns it, so it
    /// goes when the file drops it, including after a restart.
    #[serde(default)]
    from_file: bool,
}

/// Pre-auth keys and registered nodes, held in memory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryStore {
    preauth: HashMap<String, PreauthRec>,
    nodes: Vec<NodeRec>,
    /// Machines that may never register again.
    #[serde(default)]
    revoked: HashSet<MachinePublic>,
    /// Ids are never reused, even after a revoke.
    next_id: i64,
    /// How long a registration lasts; set by the operator, not saved.
    #[serde(skip)]
    node_ttl: Option<u64>,
}

impl MemoryStore {
    /// Registrations expire `secs` seconds after they are made.
    pub fn with_node_ttl(mut self, secs: u64) -> Self {
        self.node_ttl = Some(secs);
        self
    }

    pub(crate) fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }

    pub(crate) fn from_json(
        bytes: &[u8],
        node_ttl: Option<u64>,
    ) -> Result<Self, serde_json::Error> {
        let mut store: Self = serde_json::from_slice(bytes)?;
        store.node_ttl = node_ttl;
        Ok(store)
    }

    /// The login and server name (if tagged) `key` admits `machine` as, if the key may be used now.
    fn check_key(
        &self,
        key: &str,
        machine: &MachinePublic,
        now: u64,
    ) -> Result<(String, Option<String>), RegisterError> {
        if self.revoked.contains(machine) {
            return Err(RegisterError::Revoked);
        }
        let rec = self.preauth.get(key).ok_or(RegisterError::BadAuthKey)?;
        if rec.expires.is_some_and(|t| now >= t) {
            return Err(RegisterError::ExpiredAuthKey);
        }
        match rec.used_by {
            Some(spent_by) if !rec.reusable && spent_by != *machine => {
                Err(RegisterError::BadAuthKey)
            }
            _ => Ok((rec.login.clone(), rec.tagged.clone())),
        }
    }

    /// The lowest tailnet host id nobody holds.
    fn free_host(&self) -> Option<u8> {
        let taken: HashSet<u8> = self.nodes.iter().map(|n| n.ip.octets()[3]).collect();
        (1..=u8::MAX).find(|h| !taken.contains(h))
    }

    /// Refuses a key whose login or server name could not be written in a
    /// policy rule, or would collide with another identity.
    fn check_new_key(&self, login: &str, policy: &KeyPolicy) -> Result<(), RegisterError> {
        let bad = |why: &str| Err(RegisterError::InvalidKey(why.to_string()));
        if login.is_empty() || login.contains(|c: char| c.is_whitespace() || c == '#') {
            return bad("a login must be non-empty, with no whitespace or `#`");
        }
        if login == TAGGED_LOGIN {
            return bad("`tagged-devices` is reserved for tagged nodes");
        }
        // Two logins that hash to one wire user id would share a principal.
        let id = user_id(login);
        if self
            .preauth
            .values()
            .any(|k| k.login != login && user_id(&k.login) == id)
        {
            return bad("the login collides with another login's user id");
        }
        match &policy.tagged {
            Some(name) if sanitize_label(name) != *name => {
                bad("a server name must be a lowercase DNS label")
            }
            _ => Ok(()),
        }
    }

    /// The server name `name` for `machine`, unless another machine holds it.
    fn claim_name(&self, name: &str, machine: &MachinePublic) -> Result<String, RegisterError> {
        if self
            .nodes
            .iter()
            .any(|n| n.hostname == name && n.machine != *machine)
        {
            return Err(RegisterError::NameTaken(name.to_string()));
        }
        Ok(name.to_string())
    }

    /// `want` as a DNS label no other machine's node holds.
    fn unique_label(&self, want: &str, machine: &MachinePublic) -> String {
        let base = sanitize_label(want);
        let mut held: HashSet<&str> = self
            .nodes
            .iter()
            .filter(|n| n.machine != *machine)
            .map(|n| n.hostname.as_str())
            .collect();
        // A name a tagged key reserves is never handed to a device.
        held.extend(self.preauth.values().filter_map(|k| k.tagged.as_deref()));
        if !held.contains(base.as_str()) {
            return base;
        }
        (1u32..)
            .map(|i| format!("{}-{i}", &base[..base.len().min(MAX_LABEL - 4)]))
            .find(|c| !held.contains(c.as_str()))
            .unwrap_or(base)
    }
}

/// Lowercase letters, digits and inner hyphens, at most 63 long; `node` if
/// nothing usable remains.
fn sanitize_label(raw: &str) -> String {
    let mapped: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(MAX_LABEL)
        .collect();
    let trimmed = mapped.trim_matches('-');
    if trimmed.is_empty() {
        "node".to_string()
    } else {
        trimmed.to_string()
    }
}

fn live(n: &NodeRec, now: u64) -> bool {
    n.expires.is_none_or(|t| now < t)
}

impl NodeStore for MemoryStore {
    fn add_preauth(
        &mut self,
        key: &str,
        login: &str,
        policy: KeyPolicy,
        now: u64,
    ) -> Result<(), RegisterError> {
        // A key that is already here keeps its spent mark: re-adding it (a
        // restart reading the same keys file) must not make it usable again.
        if let Some(have) = self.preauth.get(key) {
            let same = have.login == login
                && have.reusable == policy.reusable
                && have.tagged == policy.tagged;
            return if same {
                Ok(())
            } else {
                Err(RegisterError::InvalidKey(
                    "the key exists with other settings; remove it first".into(),
                ))
            };
        }
        self.check_new_key(login, &policy)?;
        self.preauth.insert(
            key.to_string(),
            PreauthRec {
                login: login.to_string(),
                expires: policy.ttl.map(|t| now.saturating_add(t)),
                reusable: policy.reusable,
                tagged: policy.tagged,
                used_by: None,
                from_file: false,
            },
        );
        Ok(())
    }

    fn register(
        &mut self,
        machine: MachinePublic,
        node_key: NodePublic,
        hostname: &str,
        auth_key: &str,
        now: u64,
    ) -> Result<(), RegisterError> {
        // A node that already holds this machine and node key may register
        // again with no key (a restarted client does). Nothing changes: the
        // address, login and expiry stay, so this never extends a node's life.
        if auth_key.is_empty()
            && !self.revoked.contains(&machine)
            && self
                .nodes
                .iter()
                .any(|n| n.machine == machine && n.node_key == node_key && live(n, now))
        {
            return Ok(());
        }
        let (login, tagged) = self.check_key(auth_key, &machine, now)?;
        let hostname = match &tagged {
            Some(name) => self.claim_name(name, &machine)?,
            None => self.unique_label(hostname, &machine),
        };
        let tagged = tagged.is_some();
        let expires = self.node_ttl.map(|t| now.saturating_add(t));
        if let Some(n) = self.nodes.iter_mut().find(|n| n.machine == machine) {
            n.node_key = node_key;
            n.login = login;
            n.tagged = tagged;
            n.hostname = hostname;
            n.expires = expires;
        } else {
            let host = self.free_host().ok_or(RegisterError::Full)?;
            self.next_id += 1;
            self.nodes.push(NodeRec {
                id: self.next_id,
                login,
                hostname,
                machine,
                node_key,
                disco: None,
                endpoints: Vec::new(),
                ip: Ipv4Addr::new(100, 64, 0, host),
                tagged,
                expires,
            });
        }
        if let Some(rec) = self.preauth.get_mut(auth_key) {
            rec.used_by = Some(machine);
        }
        Ok(())
    }

    fn remove_preauth(&mut self, key: &str) -> Result<bool, RegisterError> {
        Ok(self.preauth.remove(key).is_some())
    }

    fn set_file_keys(
        &mut self,
        keys: Vec<(String, String, KeyPolicy)>,
        now: u64,
    ) -> Result<(), RegisterError> {
        let before = self.preauth.clone();
        let listed: HashSet<&str> = keys.iter().map(|(k, ..)| k.as_str()).collect();
        self.preauth
            .retain(|k, rec| !rec.from_file || listed.contains(k.as_str()));
        for (key, login, policy) in &keys {
            let added = self.add_preauth(key, login, policy.clone(), now);
            if let Err(e) = added {
                self.preauth = before;
                return Err(e);
            }
            if let Some(rec) = self.preauth.get_mut(key) {
                rec.from_file = true;
            }
        }
        Ok(())
    }

    fn revoke(&mut self, machine: &MachinePublic) -> Revocation {
        self.revoked.insert(*machine);
        let node_key = self
            .nodes
            .iter()
            .position(|n| n.machine == *machine)
            .map(|i| self.nodes.remove(i).node_key);
        Revocation {
            node_key,
            saved: Ok(()),
        }
    }

    fn has_node_key(&self, key: &NodePublic, now: u64) -> bool {
        self.nodes
            .iter()
            .any(|n| &n.node_key == key && live(n, now))
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

    fn export(&self) -> Result<Vec<u8>, RegisterError> {
        self.to_json()
            .map_err(|e| RegisterError::Storage(e.to_string()))
    }

    fn netmap(&self, machine: &MachinePublic, omit_peers: bool, now: u64) -> Option<MapResponse> {
        let me = self
            .nodes
            .iter()
            .find(|n| &n.machine == machine && live(n, now))?;
        let peers = if omit_peers {
            None
        } else {
            Some(
                self.nodes
                    .iter()
                    .filter(|n| n.machine != me.machine && live(n, now))
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
        let mut logins: Vec<&str> = self.nodes.iter().map(wire_login).collect();
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

/// The login a node's wire user carries: tagged nodes belong to the special
/// tagged-devices user, as in Tailscale.
fn wire_login(n: &NodeRec) -> &str {
    if n.tagged { TAGGED_LOGIN } else { &n.login }
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
        user: user_id(wire_login(n)),
        tags: if n.tagged {
            vec![TAG.to_string()]
        } else {
            Vec::new()
        },
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
        let node = s.netmap(&m(b), true, 0).unwrap().node.unwrap();
        match node.addresses[0].addr {
            IpAddr::V4(ip) => ip.octets()[3],
            IpAddr::V6(_) => unreachable!("the store hands out IPv4 only"),
        }
    }

    #[test]
    fn register_needs_a_known_key_and_assigns_addresses() {
        let mut s = MemoryStore::default();
        s.add_preauth("k1", "alice", KeyPolicy::REUSABLE, 0)
            .unwrap();
        assert_eq!(
            s.register(m(1), k(1), "a", "nope", 0).unwrap_err(),
            RegisterError::BadAuthKey
        );
        s.register(m(1), k(1), "a", "k1", 0).unwrap();
        s.register(m(2), k(2), "b", "k1", 0).unwrap();
        assert_eq!(ip_of(&s, 1), 1);
        assert_eq!(ip_of(&s, 2), 2);
        // Registering again keeps the address.
        s.register(m(1), k(9), "a", "k1", 0).unwrap();
        assert_eq!(ip_of(&s, 1), 1);
    }

    #[test]
    fn a_known_node_registers_again_without_a_key() {
        let mut s = MemoryStore::default().with_node_ttl(100);
        s.add_preauth("k1", "alice", KeyPolicy::REUSABLE, 0)
            .unwrap();
        s.register(m(1), k(1), "a", "k1", 0).unwrap();
        // Same machine, same node key, no pre-auth key: accepted, nothing moves.
        s.register(m(1), k(1), "a", "", 50).unwrap();
        assert_eq!(ip_of(&s, 1), 1);
        assert_eq!(s.nodes[0].expires, Some(100), "expiry is not extended");
        // A new node key, or an unknown machine, still needs a key.
        assert_eq!(
            s.register(m(1), k(2), "a", "", 50).unwrap_err(),
            RegisterError::BadAuthKey
        );
        assert_eq!(
            s.register(m(2), k(1), "b", "", 50).unwrap_err(),
            RegisterError::BadAuthKey
        );
        // An expired node must present a key again.
        assert_eq!(
            s.register(m(1), k(1), "a", "", 100).unwrap_err(),
            RegisterError::BadAuthKey
        );
        // A revoked machine never comes back.
        s.revoke(&m(1));
        assert_eq!(
            s.register(m(1), k(1), "a", "", 10).unwrap_err(),
            RegisterError::Revoked
        );
    }

    #[test]
    fn netmap_lists_the_other_nodes_with_their_disco_keys() {
        let mut s = MemoryStore::default();
        s.add_preauth("k1", "alice", KeyPolicy::REUSABLE, 0)
            .unwrap();
        s.register(m(1), k(1), "a", "k1", 0).unwrap();
        s.register(m(2), k(2), "b", "k1", 0).unwrap();
        let ep: SocketAddr = "192.0.2.1:41641".parse().unwrap();
        assert!(s.update(&m(2), DiscoPublic([7; 32]), vec![ep]));
        assert!(
            !s.update(&m(2), DiscoPublic([7; 32]), vec![ep]),
            "unchanged"
        );
        let map = s.netmap(&m(1), false, 0).unwrap();
        let peers = map.peers.unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].disco_key, Some(DiscoPublic([7; 32])));
        assert_eq!(peers[0].endpoints, vec![ep]);
        assert!(s.netmap(&m(1), true, 0).unwrap().peers.is_none());
        assert!(s.netmap(&m(9), false, 0).is_none());
    }

    #[test]
    fn the_table_is_full_after_255_nodes() {
        let mut s = MemoryStore::default();
        s.add_preauth("k1", "alice", KeyPolicy::REUSABLE, 0)
            .unwrap();
        for b in 1..=255u8 {
            s.register(MachinePublic([b; 32]), NodePublic([b; 32]), "n", "k1", 0)
                .unwrap_or_else(|e| panic!("node {b}: {e}"));
        }
        let extra = s.register(MachinePublic([0; 32]), NodePublic([0; 32]), "n", "k1", 0);
        assert_eq!(extra, Err(RegisterError::Full));
    }

    #[test]
    fn a_netmap_node_has_the_wire_shape_stock_clients_need() {
        let mut s = MemoryStore::default();
        s.add_preauth("k1", "alice", KeyPolicy::REUSABLE, 0)
            .unwrap();
        s.register(m(1), k(1), "a", "k1", 0).unwrap();
        let node = s.netmap(&m(1), true, 0).unwrap().node.unwrap();
        let json = serde_json::to_value(&node).unwrap();
        assert_eq!(json["Name"], "a.mesh.");
        assert_eq!(json["Addresses"][0], "100.64.0.1/32");
        assert_eq!(json["MachineAuthorized"], true);
        assert_eq!(json["HomeDERP"], 1);
        assert_eq!(json["Hostinfo"]["Hostname"], "a");
    }

    fn keyed(s: &mut MemoryStore, policy: KeyPolicy) {
        s.add_preauth("k", "alice", policy, 100).unwrap();
    }

    #[test]
    fn a_single_use_key_is_spent_by_its_first_machine() {
        let mut s = MemoryStore::default();
        keyed(&mut s, KeyPolicy::default());
        s.register(m(1), k(1), "a", "k", 100).unwrap();
        assert_eq!(
            s.register(m(2), k(2), "b", "k", 100),
            Err(RegisterError::BadAuthKey)
        );
        // The same machine may register again (a rotated node key).
        s.register(m(1), k(9), "a", "k", 100).unwrap();
        assert!(s.has_node_key(&k(9), 100));
    }

    #[test]
    fn a_key_stops_working_at_its_expiry() {
        let mut s = MemoryStore::default();
        keyed(
            &mut s,
            KeyPolicy {
                ttl: Some(60),
                reusable: true,
                tagged: None,
            },
        );
        s.register(m(1), k(1), "a", "k", 159).unwrap();
        assert_eq!(
            s.register(m(2), k(2), "b", "k", 160),
            Err(RegisterError::ExpiredAuthKey)
        );
    }

    #[test]
    fn an_expired_node_leaves_the_netmaps_and_the_relay() {
        let mut s = MemoryStore::default().with_node_ttl(60);
        keyed(&mut s, KeyPolicy::REUSABLE);
        s.register(m(1), k(1), "a", "k", 100).unwrap();
        s.register(m(2), k(2), "b", "k", 130).unwrap();
        assert!(s.has_node_key(&k(1), 159));
        assert!(!s.has_node_key(&k(1), 160));
        assert!(s.netmap(&m(1), false, 160).is_none());
        let seen = s.netmap(&m(2), false, 160).unwrap().peers.unwrap();
        assert!(seen.is_empty(), "an expired peer is not listed");
    }

    #[test]
    fn a_revoked_node_is_gone_and_its_address_is_reused_but_not_its_id() {
        let mut s = MemoryStore::default();
        keyed(&mut s, KeyPolicy::REUSABLE);
        s.register(m(1), k(1), "a", "k", 0).unwrap();
        s.register(m(2), k(2), "b", "k", 0).unwrap();
        assert_eq!(s.revoke(&m(1)).node_key, Some(k(1)));
        assert_eq!(s.revoke(&m(1)).node_key, None);
        assert!(!s.has_node_key(&k(1), 0));
        s.register(m(3), k(3), "c", "k", 0).unwrap();
        assert_eq!(ip_of(&s, 3), 1, "the lowest free host");
        let node = s.netmap(&m(3), true, 0).unwrap().node.unwrap();
        assert_eq!(node.id.0, 3, "ids are never reused");
    }

    #[test]
    fn hostnames_become_unique_dns_labels() {
        assert_eq!(sanitize_label("My Laptop_01!"), "my-laptop-01");
        assert_eq!(sanitize_label("--"), "node");
        assert_eq!(sanitize_label(&"x".repeat(80)).len(), 63);
        let mut s = MemoryStore::default();
        keyed(&mut s, KeyPolicy::REUSABLE);
        for b in 1..=3u8 {
            s.register(m(b), k(b), "Dev Box", "k", 0).unwrap();
        }
        let names: Vec<String> = [1u8, 2, 3]
            .iter()
            .map(|b| s.netmap(&m(*b), true, 0).unwrap().node.unwrap().name)
            .collect();
        assert_eq!(
            names,
            ["dev-box.mesh.", "dev-box-1.mesh.", "dev-box-2.mesh."]
        );
        // Registering again keeps a machine's own name.
        s.register(m(1), k(1), "Dev Box", "k", 0).unwrap();
        assert_eq!(
            s.netmap(&m(1), true, 0).unwrap().node.unwrap().name,
            "dev-box.mesh."
        );
    }

    #[test]
    fn a_revoked_machine_cannot_register_again_with_any_key() {
        let mut s = MemoryStore::default();
        keyed(&mut s, KeyPolicy::REUSABLE);
        s.register(m(1), k(1), "a", "k", 0).unwrap();
        s.revoke(&m(1));
        assert_eq!(
            s.register(m(1), k(1), "a", "k", 0),
            Err(RegisterError::Revoked)
        );
        assert!(!s.has_node_key(&k(1), 0));
        // Other machines are unaffected.
        s.register(m(2), k(2), "b", "k", 0).unwrap();
    }

    fn tagged(name: &str) -> KeyPolicy {
        KeyPolicy {
            tagged: Some(name.to_string()),
            ..KeyPolicy::REUSABLE
        }
    }

    #[test]
    fn a_tagged_key_fixes_the_servers_name_and_refuses_a_second_holder() {
        let mut s = MemoryStore::default();
        s.add_preauth("t", "ops", tagged("mux"), 0).unwrap();
        // The node's own hostname is ignored.
        s.register(m(1), k(1), "whatever", "t", 0).unwrap();
        let node = s.netmap(&m(1), true, 0).unwrap().node.unwrap();
        assert_eq!(node.name, "mux.mesh.");
        assert_eq!(node.tags, ["tag:mesh"]);
        // Another machine with the same key cannot take the name...
        assert_eq!(
            s.register(m(2), k(2), "mux", "t", 0),
            Err(RegisterError::NameTaken("mux".into()))
        );
        // ...but the holder may register again (a rotated node key).
        s.register(m(1), k(9), "mux", "t", 0).unwrap();
        // Once revoked, the name is free for the key's next holder.
        s.revoke(&m(1));
        s.register(m(2), k(2), "mux", "t", 0).unwrap();
    }

    #[test]
    fn a_device_cannot_squat_a_name_a_tagged_key_reserves() {
        let mut s = MemoryStore::default();
        s.add_preauth("t", "ops", tagged("mux"), 0).unwrap();
        s.add_preauth("d", "alice", KeyPolicy::REUSABLE, 0).unwrap();
        // The device registers first and asks for the server's name.
        s.register(m(1), k(1), "mux", "d", 0).unwrap();
        let device = s.netmap(&m(1), true, 0).unwrap().node.unwrap();
        assert_eq!(device.name, "mux-1.mesh.");
        assert!(device.tags.is_empty());
        // The real server still gets its name.
        s.register(m(2), k(2), "x", "t", 0).unwrap();
        assert_eq!(
            s.netmap(&m(2), true, 0).unwrap().node.unwrap().name,
            "mux.mesh."
        );
    }

    #[test]
    fn keys_that_could_not_be_written_in_a_rule_or_that_collide_are_refused() {
        let mut s = MemoryStore::default();
        let refused = |s: &mut MemoryStore, login: &str, policy: KeyPolicy| {
            matches!(
                s.add_preauth("k", login, policy, 0),
                Err(RegisterError::InvalidKey(_))
            )
        };
        for login in ["", "a b", "a#b", "tagged-devices"] {
            assert!(refused(&mut s, login, KeyPolicy::REUSABLE), "{login:?}");
        }
        for name in ["", "Mux", "a b", "a.b"] {
            assert!(refused(&mut s, "ops", tagged(name)), "{name:?}");
        }
        // "Aa" and "BB" hash to the same user id (31 * 65 + 97 == 31 * 66 + 66).
        s.add_preauth("1", "Aa", KeyPolicy::REUSABLE, 0).unwrap();
        assert!(
            refused(&mut s, "BB", KeyPolicy::REUSABLE),
            "colliding login"
        );
        // The same login may have more than one key.
        s.add_preauth("2", "Aa", KeyPolicy::REUSABLE, 0).unwrap();
    }

    #[test]
    fn re_registering_under_a_tagged_key_cannot_rename_the_node() {
        let mut s = MemoryStore::default();
        s.add_preauth("t", "ops", tagged("mux"), 0).unwrap();
        s.register(m(1), k(1), "mux", "t", 0).unwrap();
        s.register(m(1), k(2), "renamed", "t", 0).unwrap();
        let node = s.netmap(&m(1), true, 0).unwrap().node.unwrap();
        assert_eq!(node.name, "mux.mesh.");
        assert_eq!(node.key, Some(k(2)), "the new node key took effect");
    }

    #[test]
    fn a_table_saved_before_tagged_keys_still_loads() {
        let old = br#"{"preauth":{"k":{"login":"alice","expires":null,"reusable":true,"used_by":null}},
            "nodes":[{"id":1,"login":"alice","hostname":"a","machine":"mkey:0101010101010101010101010101010101010101010101010101010101010101",
            "node_key":"nodekey:0202020202020202020202020202020202020202020202020202020202020202","ip":"100.64.0.1"}],
            "next_id":1}"#;
        let mut s = MemoryStore::from_json(old, None).expect("an old table loads");
        assert!(s.has_node_key(&NodePublic([2; 32]), 0));
        s.register(m(2), k(3), "b", "k", 0).unwrap();
    }
}
