//! Spike A (throwaway, feature `mesh`): serve mux over an embedded tailnet
//! node. No `tailscaled`, no root, no TUN: `ts-net` registers with the
//! control server, binds the mux port on a userspace stack, and a peer's
//! identity comes from the node's own netmap instead of a LocalAPI call.
//! Admission is the shared-mode allowlist, as for `--tailnet`.

use std::cell::RefCell;
use std::io;
use std::net::IpAddr;
use std::path::PathBuf;
use std::rc::Rc;

use rusty_mux_core::{Principal, Transport};
use rusty_tokio::io::split;
use ts_types::Status;

use crate::passed::Descriptors;
use crate::shared::{Allow, Guests};
use crate::tailnet::principal_named;
use crate::{conn, state};

impl Descriptors for rusty_tokio::io::SplitReadHalf<ts_net::TcpStream> {}

/// How the embedded node joins the mesh.
pub struct MeshConfig {
    /// Control server base URL (Headscale, or a hub).
    pub control_url: String,
    /// Pre-auth key; never logged.
    pub authkey: String,
    /// Node name advertised to the control server.
    pub hostname: String,
    /// Where the node keeps its keys.
    pub state_dir: PathBuf,
    /// Mux port on the tailnet address.
    pub port: u16,
}

/// The mesh side of a shared server: the node and its mux listener.
pub struct Mesh {
    node: ts_net::Node,
    listener: ts_net::TcpListener,
    pub(crate) allow: Rc<RefCell<Allow>>,
}

impl Mesh {
    /// Registers, waits for the tailnet address and binds the mux port.
    pub async fn start(config: MeshConfig) -> io::Result<(Self, std::net::Ipv4Addr)> {
        let other = |e: ts_net::Error| io::Error::other(format!("mesh: {e}"));
        let node = ts_net::Node::new(ts_net::NodeConfig {
            control_url: config.control_url,
            derp_url: None,
            authkey: config.authkey,
            hostname: config.hostname,
            state_dir: config.state_dir,
            enable_direct: true,
            stun_server: None,
        })
        .await
        .map_err(other)?;
        let ip = node
            .wait_ip()
            .await
            .ok_or_else(|| io::Error::other("mesh: no tailnet address from the control server"))?;
        let listener = node.bind(config.port).await.map_err(other)?;
        Ok((
            Self {
                node,
                listener,
                allow: Rc::default(),
            },
            ip,
        ))
    }
}

/// The principal of the tailnet peer at `ip`, from the node's netmap: a
/// tagged node is `ts-node:NAME`, any other `ts:LOGIN`. A peer not in the
/// netmap is refused.
pub fn principal_in(status: &Status, ip: IpAddr) -> Result<Principal, String> {
    let peer = status
        .peer
        .values()
        .find(|p| p.tailscale_ips.contains(&ip))
        .ok_or("the peer is not in the netmap")?;
    let login = status
        .user
        .get(&peer.user_id)
        .map(|u| u.login_name.as_str());
    principal_named(&peer.tags, &peer.dns_name, login)
}

/// Serves mesh peers: each is named from the netmap before it says anything
/// and served only if the list in force names it. A guest like any other: no
/// descriptors (C75), no UDP.
pub(crate) async fn accept(mut mesh: Mesh, state: state::Shared, guests: Guests) -> io::Result<()> {
    while let Some(stream) = mesh.listener.accept().await {
        let peer = stream.peer_addr();
        let Some(status) = mesh.node.status().await else {
            eprintln!("rusty-mux-server: mesh: no status, closing {peer}");
            continue;
        };
        let principal = match principal_in(&status, peer.ip()) {
            Ok(p) if mesh.allow.borrow().names(&p) => p,
            Ok(p) => {
                eprintln!("rusty-mux-server: mesh: refusing {}", p.id());
                continue;
            }
            Err(err) => {
                eprintln!("rusty-mux-server: mesh: {peer}: {err}");
                continue;
            }
        };
        let (state, guests, who) = (state.clone(), guests.clone(), principal.clone());
        rusty_tokio::task::spawn_local(async move {
            let (rd, wr) = split(stream);
            conn::run(
                state,
                rd,
                wr,
                principal,
                Transport::Socket,
                move |_, cancel| guests.add(who, cancel),
            )
            .await;
        });
    }
    Err(io::Error::other("mesh: the node's listener closed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ts_types::{NodePublic, PeerStatus, UserID, UserProfile};

    const KEY: &str = "nodekey:0000000000000000000000000000000000000000000000000000000000000001";

    fn status(tags: &[&str], login: &str) -> Status {
        let mut status = Status::default();
        let peer = PeerStatus {
            user_id: UserID(7),
            dns_name: "box.tail.net.".into(),
            tailscale_ips: vec!["100.64.0.2".parse().unwrap()],
            tags: tags.iter().map(|t| t.to_string()).collect(),
            ..PeerStatus::default()
        };
        status.peer.insert(KEY.parse::<NodePublic>().unwrap(), peer);
        status.user.insert(
            UserID(7),
            UserProfile {
                id: UserID(7),
                login_name: login.into(),
                ..UserProfile::default()
            },
        );
        status
    }

    #[test]
    fn a_user_node_is_its_login_and_a_tagged_node_its_name() {
        let ip: IpAddr = "100.64.0.2".parse().unwrap();
        let user = principal_in(&status(&[], "ann@example.com"), ip).unwrap();
        assert_eq!(user.id(), "ts:ann@example.com");
        let tagged = principal_in(&status(&["tag:ci"], "tagged-devices"), ip).unwrap();
        assert_eq!(tagged.id(), "ts-node:box.tail.net");
    }

    #[test]
    fn an_address_outside_the_netmap_or_a_nameless_user_is_refused() {
        let other: IpAddr = "100.64.0.9".parse().unwrap();
        assert!(principal_in(&status(&[], "ann"), other).is_err());
        let ip: IpAddr = "100.64.0.2".parse().unwrap();
        assert!(principal_in(&status(&[], ""), ip).is_err());
    }
}
