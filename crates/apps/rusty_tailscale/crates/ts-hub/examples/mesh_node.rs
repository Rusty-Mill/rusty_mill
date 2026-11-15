//! Spike C probe: one `ts-net` node that either receives or sends a paced
//! byte stream and reports which path the peer is on.
//!
//! `mesh_node serve <hub url> <stun host:port> <name> <port> <total bytes>`
//! `mesh_node send  <hub url> <stun host:port> <name> <peer ip> <port> <total bytes>`
//!
//! The sender writes 32 KiB every 50 ms and prints `path=` each second:
//! `direct <addr>` once the peer's current endpoint is a direct address,
//! `relay` before that. Both sides print the SHA-256 of what they moved.

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use rusty_sha2::{Hash, Sha256};
use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt};
use ts_net::{Node, NodeConfig};

const CHUNK: usize = 32 << 10;
const PACE: Duration = Duration::from_millis(50);

fn pattern(offset: usize, len: usize) -> Vec<u8> {
    (offset..offset + len)
        .map(|i| (i as u32).wrapping_mul(2654435761).to_be_bytes()[0])
        .collect()
}

fn hex(d: [u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

async fn start(url: &str, stun: &str, name: &str) -> Node {
    let state_dir = std::env::temp_dir().join(format!("mesh-node-{name}-{}", std::process::id()));
    let node = Node::new(NodeConfig {
        control_url: url.to_string(),
        derp_url: None,
        authkey: "spikekey".into(),
        hostname: name.into(),
        state_dir,
        enable_direct: true,
        stun_server: Some(stun.to_string()),
    })
    .await
    .expect("register with the hub");
    node.wait_ip().await.expect("tailnet address");
    node
}

async fn path(node: &Node) -> String {
    let Some(status) = node.status().await else {
        return "unknown".into();
    };
    match status.peer.values().next() {
        Some(p) if !p.cur_addr.is_empty() => format!("direct {}", p.cur_addr),
        Some(_) => "relay".into(),
        None => "no peer yet".into(),
    }
}

#[rusty_tokio::main(flavor = "current_thread")]
async fn main() {
    if let Ok(filter) = std::env::var("MESH_LOG") {
        rusty_log::tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    }
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["serve", url, stun, name, port, total] => {
            let (port, total): (u16, usize) = (port.parse().unwrap(), total.parse().unwrap());
            let node = start(url, stun, name).await;
            let mut listener = node.bind(port).await.expect("bind");
            println!("READY {}", node.tailnet_ip().await.unwrap());
            let mut stream = listener.accept().await.expect("a connection");
            let mut hash = Sha256::new();
            let mut got = 0;
            let mut buf = vec![0u8; CHUNK];
            let mut last = Instant::now();
            while got < total {
                let n = stream.read(&mut buf).await.expect("read");
                if n == 0 {
                    break;
                }
                hash.update(&buf[..n]);
                got += n;
                if last.elapsed() >= Duration::from_secs(1) {
                    println!("recv {got} path={}", path(&node).await);
                    last = Instant::now();
                }
            }
            println!("DONE recv {got} sha256={}", hex(hash.finalize()));
        }
        ["send", url, stun, name, peer, port, total] => {
            let (peer, port, total): (IpAddr, u16, usize) = (
                peer.parse().unwrap(),
                port.parse().unwrap(),
                total.parse().unwrap(),
            );
            let node = start(url, stun, name).await;
            let mut stream = node
                .connect(SocketAddr::new(peer, port))
                .await
                .expect("dial");
            let mut hash = Sha256::new();
            let mut sent = 0;
            let mut last = Instant::now();
            while sent < total {
                let chunk = pattern(sent, CHUNK.min(total - sent));
                hash.update(&chunk);
                stream.write_all(&chunk).await.expect("write");
                sent += chunk.len();
                if last.elapsed() >= Duration::from_secs(1) {
                    println!("sent {sent} path={}", path(&node).await);
                    last = Instant::now();
                }
                rusty_tokio::time::sleep(PACE).await;
            }
            println!("DONE sent {sent} sha256={}", hex(hash.finalize()));
            // Closing drops unsent bytes (ts-net), so let the stack drain.
            rusty_tokio::time::sleep(Duration::from_secs(10)).await;
        }
        _ => eprintln!("usage: see the file header"),
    }
}
