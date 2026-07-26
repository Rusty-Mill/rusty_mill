#![cfg(target_os = "linux")]

//! Embeddable node: a userspace TCP/IP stack (smoltcp) on the tailnet, with
//! **no TUN device and no root**.
//!
//! `ts-engine` decrypts inbound WireGuard payloads to bare IP packets; instead
//! of handing them to the OS via a TUN device (Phase 4), a [`Node`] feeds them
//! to an in-process smoltcp stack and encapsulates whatever smoltcp emits.
//! Applications then [`bind`](Node::bind) a TCP port on the tailnet IP and
//! serve connections as ordinary [`tokio::io`] streams — a plain `cargo run`
//! becomes a tailnet service.

mod device;
mod stack;

use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use ts_engine::{Engine, EngineConfig, EngineHandle, StackIo};
use ts_key::NodeState;

pub use stack::TcpStream;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Engine(#[from] ts_engine::EngineError),
    #[error("identity error: {0}")]
    Identity(#[from] ts_key::StateError),
    #[error("node stack stopped")]
    StackGone,
}

/// Configuration for a [`Node`].
pub struct NodeConfig {
    /// Control base URL, e.g. `http://127.0.0.1:8080`.
    pub control_url: String,
    /// DERP relay base URL; defaults to `control_url` when `None`.
    pub derp_url: Option<String>,
    /// Preauth key to register with.
    pub authkey: String,
    /// Node hostname advertised to the control server.
    pub hostname: String,
    /// Identity/state directory (keys are generated on first run).
    pub state_dir: std::path::PathBuf,
    /// Enable direct-path discovery (magicsock/disco).
    pub enable_direct: bool,
    /// Optional `host:port` STUN server for reflexive-endpoint discovery.
    pub stun_server: Option<String>,
}

/// A running embeddable tailnet node with a userspace TCP/IP stack.
#[derive(Clone)]
pub struct Node {
    engine: EngineHandle,
    net: mpsc::UnboundedSender<stack::Request>,
}

impl Node {
    /// Registers with the control server, brings up the data plane, and starts
    /// the userspace network stack. Returns once the node is registered; the
    /// tailnet IP arrives shortly after (await [`wait_ip`](Node::wait_ip)).
    pub async fn new(config: NodeConfig) -> Result<Node, Error> {
        let state = NodeState::load_or_generate(&config.state_dir)?;

        // Wire the engine's decrypted-packet path to our stack.
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();

        let engine_config = EngineConfig {
            derp_url: config
                .derp_url
                .unwrap_or_else(|| config.control_url.clone()),
            control_url: config.control_url,
            authkey: config.authkey,
            hostname: config.hostname,
            tun_name: None,
            magic_dns_hosts: None,
            enable_direct: config.enable_direct,
            stun_server: config.stun_server,
            stack_io: Some(StackIo {
                inbound: inbound_tx,
                outbound: outbound_rx,
            }),
        };

        let engine = Engine::start(engine_config, state).await?;
        let net = stack::spawn(inbound_rx, outbound_tx);
        Ok(Node { engine, net })
    }

    /// Waits until the control server assigns our tailnet IPv4 address and
    /// returns it, telling the stack to bind to it. Times out to `None` after
    /// ~30 s.
    pub async fn wait_ip(&self) -> Option<Ipv4Addr> {
        for _ in 0..300 {
            if let Some(ip) = self.tailnet_ip().await {
                let _ = self.net.send(stack::Request::SetAddr(ip));
                return Some(ip);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    /// The current tailnet IPv4 address, if the netmap has arrived.
    pub async fn tailnet_ip(&self) -> Option<Ipv4Addr> {
        let status = self.engine.status().await?;
        status.self_.and_then(|s| {
            s.tailscale_ips.iter().find_map(|ip| match ip {
                std::net::IpAddr::V4(v4) => Some(*v4),
                std::net::IpAddr::V6(_) => None,
            })
        })
    }

    /// A snapshot of tailnet status (self + peers), LocalAPI-compatible.
    pub async fn status(&self) -> Option<ts_types::Status> {
        self.engine.status().await
    }

    /// The underlying engine handle (for `ping`, etc.).
    pub fn engine(&self) -> &EngineHandle {
        &self.engine
    }

    /// Binds a TCP port on the tailnet and returns a listener. Ensure the
    /// tailnet IP is assigned first (await [`wait_ip`](Node::wait_ip)).
    pub async fn bind(&self, port: u16) -> Result<TcpListener, Error> {
        let (reply, rx) = oneshot::channel();
        self.net
            .send(stack::Request::Bind { port, reply })
            .map_err(|_| Error::StackGone)?;
        let accept_rx = rx.await.map_err(|_| Error::StackGone)?;
        Ok(TcpListener { accept_rx })
    }
}

/// A bound TCP listener on the tailnet, yielding [`TcpStream`]s.
pub struct TcpListener {
    accept_rx: mpsc::Receiver<TcpStream>,
}

impl TcpListener {
    /// Waits for the next inbound connection. Returns `None` if the node's
    /// stack has stopped.
    pub async fn accept(&mut self) -> Option<TcpStream> {
        self.accept_rx.recv().await
    }
}
