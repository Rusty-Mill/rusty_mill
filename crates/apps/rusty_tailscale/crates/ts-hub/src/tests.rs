//! End to end: the existing `ts-control` client against the hub, over real
//! TCP, Noise and HTTP/2.

use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use rusty_tokio::net::TcpListener;
use ts_control::{ClientError, ControlClient};
use ts_key::{DiscoPrivate, MachinePrivate, NodePrivate};
use ts_types::tailcfg::Hostinfo;

use super::*;

struct Peer {
    client: ControlClient,
    node: NodePrivate,
    disco: DiscoPrivate,
}

async fn start() -> (Arc<Hub>, String) {
    let hub = Hub::new();
    hub.add_preauth("k1", "alice");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    rusty_tokio::spawn(hub.clone().serve(listener));
    (hub, url)
}

async fn peer(url: &str, name: &str) -> Peer {
    let hostinfo = Hostinfo {
        hostname: name.into(),
        os: "linux".into(),
        ..Default::default()
    };
    Peer {
        client: ControlClient::connect(url, MachinePrivate::generate(), hostinfo)
            .await
            .unwrap(),
        node: NodePrivate::generate(),
        disco: DiscoPrivate::generate(),
    }
}

fn ep(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

#[rusty_tokio::test]
async fn the_key_endpoint_matches_the_noise_identity() {
    let (hub, url) = start().await;
    let p = peer(&url, "a").await;
    assert_eq!(p.client.control_key(), hub.public_key());
}

#[rusty_tokio::test]
async fn a_wrong_pre_auth_key_is_refused() {
    let (_hub, url) = start().await;
    let p = peer(&url, "a").await;
    let err = p
        .client
        .register(p.node.public(), "wrong")
        .await
        .unwrap_err();
    assert!(matches!(err, ClientError::RegisterRejected(_)), "{err:?}");
}

#[rusty_tokio::test]
async fn two_nodes_see_each_other_with_disco_keys_and_endpoints() {
    let (_hub, url) = start().await;
    let a = peer(&url, "a").await;
    let b = Arc::new(peer(&url, "b").await);
    for p in [&a, &*b] {
        let resp = p.client.register(p.node.public(), "k1").await.unwrap();
        assert!(resp.machine_authorized);
    }
    b.client
        .update_endpoints(
            b.node.public(),
            b.disco.public(),
            vec![ep("192.0.2.2:41641")],
        )
        .await
        .unwrap();

    // A streaming poll: the first frame already lists b, and a later change
    // of b's endpoints arrives as a new frame.
    let mut frames = 0;
    let b2 = b.clone();
    let result = rusty_tokio::time::timeout(
        Duration::from_secs(10),
        a.client
            .poll_netmap(a.node.public(), a.disco.public(), vec![], |map| {
                let Some(peers) = map.peers else {
                    return ControlFlow::Continue(());
                };
                frames += 1;
                let peer_b = peers.iter().find(|n| n.key == Some(b.node.public()));
                let peer_b = peer_b.expect("b is in the netmap");
                assert_eq!(peer_b.disco_key, Some(b.disco.public()));
                if peer_b.endpoints == vec![ep("192.0.2.3:41641")] {
                    return ControlFlow::Break(());
                }
                assert_eq!(peer_b.endpoints, vec![ep("192.0.2.2:41641")]);
                let b3 = b2.clone();
                rusty_tokio::spawn(async move {
                    let _ = b3
                        .client
                        .update_endpoints(
                            b3.node.public(),
                            b3.disco.public(),
                            vec![ep("192.0.2.3:41641")],
                        )
                        .await;
                });
                ControlFlow::Continue(())
            }),
    )
    .await;
    result.expect("no timeout").expect("poll ends cleanly");
}

/// Registers a node with the hub and returns its node key.
async fn registered(url: &str, name: &str) -> NodePrivate {
    let p = peer(url, name).await;
    p.client.register(p.node.public(), "k1").await.unwrap();
    p.node
}

#[rusty_tokio::test]
async fn two_registered_nodes_move_4_mib_through_the_hub_derp() {
    use ts_derp::DerpClient;
    let (_hub, url) = start().await;
    let (ka, kb) = (registered(&url, "a").await, registered(&url, "b").await);
    let a = DerpClient::connect(&url, &ka).await.unwrap();
    let mut b = DerpClient::connect(&url, &kb).await.unwrap();

    let data: Vec<u8> = (0..4u32 << 20)
        .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
        .collect();
    let sender = a.sender();
    let to = kb.public();
    let outgoing = data.clone();
    rusty_tokio::spawn(async move {
        for chunk in outgoing.chunks(16 << 10) {
            sender.send(to, chunk.to_vec()).await.unwrap();
        }
    });

    let mut got = Vec::with_capacity(data.len());
    let read = rusty_tokio::time::timeout(Duration::from_secs(30), async {
        while got.len() < data.len() {
            let packet = b.recv().await.expect("relay stays up");
            assert_eq!(packet.peer, ka.public());
            got.extend_from_slice(&packet.payload);
        }
    })
    .await;
    read.expect("all 4 MiB arrive");
    assert!(got == data, "bytes differ");
}

#[rusty_tokio::test]
async fn an_unregistered_node_key_is_refused_by_the_derp_relay() {
    use ts_derp::DerpClient;
    let (_hub, url) = start().await;
    let stranger = NodePrivate::generate();
    let result = DerpClient::connect(&url, &stranger).await;
    assert!(result.is_err(), "relay admitted a stranger");
}

#[rusty_tokio::test]
async fn derp_fast_start_skips_the_http_response() {
    use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_hub, url) = start().await;
    let addr = url.trim_start_matches("http://");
    let mut s = rusty_tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "GET /derp HTTP/1.1\r\nHost: {addr}\r\nUpgrade: DERP\r\nConnection: Upgrade\r\nDerp-Fast-Start: 1\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut head = [0u8; 5];
    s.read_exact(&mut head).await.unwrap();
    assert_eq!(head[0], 0x01, "first byte is the ServerKey frame, not HTTP");
}
