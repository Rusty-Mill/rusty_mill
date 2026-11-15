#![cfg(target_os = "linux")]
//! Two `ts-net` nodes join an in-process `ts-hub` and move 4 MiB over the
//! tailnet with `write_all` + `shutdown` + `read_to_end`; the bytes must match.
//! This is the hub's end-to-end proof for the userspace stack (card #1466).

use std::net::SocketAddr;
use std::path::PathBuf;

use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt};
use rusty_tokio::net::TcpListener;
use ts_hub::{Hub, KeyPolicy, Policy};
use ts_net::{Node, NodeConfig};

const KEY: &str = "tskey-hub-transfer";
const PORT: u16 = 7000;
const LEN: usize = 4 << 20;

fn state_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ts-net-hub-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

async fn join(url: &str, name: &str) -> (Node, std::net::Ipv4Addr) {
    let node = Node::new(NodeConfig {
        control_url: url.to_owned(),
        derp_url: None,
        authkey: KEY.to_owned(),
        hostname: name.to_owned(),
        state_dir: state_dir(name),
        enable_direct: false,
        stun_server: None,
    })
    .await
    .expect("node joins the hub");
    let ip = node.wait_ip().await.expect("hub assigns an address");
    (node, ip)
}

/// A pattern that shows a dropped, repeated or reordered chunk.
fn payload() -> Vec<u8> {
    (0..LEN)
        .map(|i| (i.wrapping_mul(31) ^ (i >> 8)) as u8)
        .collect()
}

#[rusty_tokio::test]
async fn nodes_transfer_four_mebibytes_through_the_hub() {
    let hub = Hub::new();
    hub.add_preauth(KEY, "test@example.com", KeyPolicy::REUSABLE)
        .expect("key");
    hub.set_policy(Policy::allow_all());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind hub");
    let addr: SocketAddr = listener.local_addr().expect("addr");
    hub.advertise("127.0.0.1", addr.port(), 0);
    rusty_tokio::spawn(hub.serve(listener));
    let url = format!("http://{addr}");

    let (a, a_ip) = join(&url, "alpha").await;
    let (b, _) = join(&url, "beta").await;
    let mut listener = a.bind(PORT).await.expect("bind on alpha");

    let want = payload();
    let sender = rusty_tokio::spawn({
        let want = want.clone();
        async move {
            let mut s = b.connect((a_ip, PORT).into()).await.expect("connect");
            s.write_all(&want).await.expect("write");
            s.shutdown().await.expect("shutdown");
            s // keep the stream (and the node) alive until the reader is done
        }
    });

    let mut conn = listener.accept().await.expect("accept");
    let mut got = Vec::with_capacity(LEN);
    rusty_tokio::time::timeout(
        std::time::Duration::from_secs(60),
        conn.read_to_end(&mut got),
    )
    .await
    .expect("read_to_end finished (EOF seen)")
    .expect("read");
    assert_eq!(got.len(), want.len(), "length");
    assert!(got == want, "bytes differ");
    drop(sender.await);
}
