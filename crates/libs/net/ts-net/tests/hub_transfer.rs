#![cfg(target_os = "linux")]
//! Two `ts-net` nodes join an in-process `ts-hub` and move bytes over the
//! tailnet with `write_all` + `shutdown` + `read_to_end`; the bytes must
//! match. Once over plain HTTP, once with the control and DERP traffic inside
//! verified TLS (the hub's own certificate is the only trust anchor).

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt};
use rusty_tokio::net::TcpListener;
use ts_hub::{Hub, KeyPolicy, Policy, tls_acceptor};
use ts_net::{Node, NodeConfig, Trust};

const KEY: &str = "tskey-hub-transfer";
const PORT: u16 = 7000;

fn state_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ts-net-hub-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A running hub: its base URL and what a node must trust to reach it.
struct Running {
    url: String,
    trust: Trust,
    _hub: Arc<Hub>,
}

async fn start_hub(tls: bool) -> Running {
    let hub = Hub::new();
    hub.add_preauth(KEY, "test@example.com", KeyPolicy::REUSABLE)
        .expect("key");
    hub.set_policy(Policy::allow_all());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind hub");
    let addr: SocketAddr = listener.local_addr().expect("addr");
    hub.advertise("127.0.0.1", addr.port(), 0);
    if !tls {
        rusty_tokio::spawn(hub.clone().serve(listener));
        return Running {
            url: format!("http://{addr}"),
            trust: Trust::System,
            _hub: hub,
        };
    }
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("cert");
    let acceptor = tls_acceptor(&cert.pem(), &key_pair.serialize_pem()).expect("acceptor");
    rusty_tokio::spawn(hub.clone().serve_tls(listener, acceptor));
    Running {
        url: format!("https://localhost:{}", addr.port()),
        trust: Trust::PinnedAnchors(vec![cert.der().to_vec().into()]),
        _hub: hub,
    }
}

async fn join(hub: &Running, name: &str) -> (Node, Ipv4Addr) {
    let node = Node::new(NodeConfig {
        control_url: hub.url.clone(),
        derp_url: None,
        authkey: KEY.to_owned(),
        hostname: name.to_owned(),
        state_dir: state_dir(name),
        enable_direct: false,
        stun_server: None,
        trust: hub.trust.clone(),
    })
    .await
    .expect("node joins the hub");
    let ip = node.wait_ip().await.expect("hub assigns an address");
    (node, ip)
}

/// A pattern that shows a dropped, repeated or reordered chunk.
fn payload(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i.wrapping_mul(31) ^ (i >> 8)) as u8)
        .collect()
}

async fn transfer(tls: bool, len: usize) {
    let hub = start_hub(tls).await;
    let (a, a_ip) = join(&hub, if tls { "alpha-tls" } else { "alpha" }).await;
    let (b, _) = join(&hub, if tls { "beta-tls" } else { "beta" }).await;
    let mut listener = a.bind(PORT).await.expect("bind on alpha");

    let want = payload(len);
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
    let mut got = Vec::with_capacity(len);
    rusty_tokio::time::timeout(Duration::from_secs(60), conn.read_to_end(&mut got))
        .await
        .expect("read_to_end finished (EOF seen)")
        .expect("read");
    assert_eq!(got.len(), want.len(), "length");
    assert!(got == want, "bytes differ");
    drop(sender.await);
}

#[rusty_tokio::test]
async fn nodes_transfer_four_mebibytes_through_the_hub() {
    transfer(false, 4 << 20).await;
}

#[rusty_tokio::test]
async fn nodes_transfer_through_a_hub_behind_verified_tls() {
    transfer(true, 1 << 20).await;
}

#[rusty_tokio::test]
async fn a_node_that_does_not_trust_the_hub_certificate_cannot_join() {
    let hub = start_hub(true).await;
    let other = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("cert");
    let joined = rusty_tokio::time::timeout(
        Duration::from_secs(20),
        Node::new(NodeConfig {
            control_url: hub.url.clone(),
            derp_url: None,
            authkey: KEY.to_owned(),
            hostname: "stranger".to_owned(),
            state_dir: state_dir("stranger"),
            enable_direct: false,
            stun_server: None,
            trust: Trust::PinnedAnchors(vec![other.cert.der().to_vec().into()]),
        }),
    )
    .await
    .expect("refusal comes back, not a hang");
    assert!(
        joined.is_err(),
        "a certificate we do not trust must be refused"
    );
}
