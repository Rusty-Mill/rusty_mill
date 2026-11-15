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

#[test]
fn every_netmap_carries_a_filter_that_admits_all_traffic() {
    let hub = Hub::new();
    hub.add_preauth("k1", "alice");
    let machine = MachinePublic([1; 32]);
    hub.locked(|s| s.register(machine, NodePublic([2; 32]), "a", "k1"))
        .unwrap();
    let map = hub.netmap(&machine, false).unwrap();
    let rules = map.packet_filter.expect("a filter is present");
    assert_eq!(rules[0].src_ips, ["*"]);
    assert_eq!(rules[0].dst_ports[0].ports.last, u16::MAX);
}
