//! Spike B pass test: two unmodified `ts-net` nodes register against the
//! hub (not Headscale), find each other in its netmap, and move 4 MiB over
//! the hub's own DERP with matching SHA-256.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use rusty_sha2::{Hash, Sha256};
use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt};
use rusty_tokio::net::TcpListener;
use ts_hub::Hub;
use ts_net::{Node, NodeConfig};

fn size() -> usize {
    std::env::var("MESH_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4 << 20)
}

fn digest(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize()
}

async fn node(url: &str, name: &str) -> Node {
    let dir: PathBuf = std::env::temp_dir().join(format!("ts-hub-{name}-{}", std::process::id()));
    let node = Node::new(NodeConfig {
        control_url: url.to_string(),
        derp_url: None,
        authkey: "k1".into(),
        hostname: name.into(),
        state_dir: dir,
        enable_direct: false,
        stun_server: None,
    })
    .await
    .expect("node registers with the hub");
    node.wait_ip().await.expect("hub assigns an address");
    node
}

#[rusty_tokio::test]
async fn two_unmodified_nodes_move_4_mib_through_the_hub() {
    let hub = Hub::new();
    hub.add_preauth("k1", "alice");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    rusty_tokio::spawn(hub.clone().serve(listener));

    let server = node(&url, "server").await;
    let client = node(&url, "client").await;
    let server_ip = server.tailnet_ip().await.unwrap();

    let data: Vec<u8> = (0..size() as u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
        .collect();
    let want = digest(&data);

    let mut accept = server.bind(7000).await.unwrap();
    let receive = rusty_tokio::spawn(async move {
        let mut stream = accept.accept().await.expect("a connection");
        let mut got = vec![0u8; size()];
        stream.read_exact(&mut got).await.unwrap();
        digest(&got)
    });

    let addr = SocketAddr::new(IpAddr::V4(server_ip), 7000);
    let mut stream = rusty_tokio::time::timeout(Duration::from_secs(30), client.connect(addr))
        .await
        .expect("dial in time")
        .expect("dial succeeds");
    stream.write_all(&data).await.unwrap();
    stream.flush().await.unwrap();

    let got = rusty_tokio::time::timeout(Duration::from_secs(120), receive)
        .await
        .expect("transfer in time")
        .expect("receiver finishes");
    assert_eq!(got, want, "SHA-256 differs");
}
