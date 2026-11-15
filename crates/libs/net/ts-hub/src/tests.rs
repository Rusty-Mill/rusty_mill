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
    start_with(Limits::default()).await
}

async fn start_with(limits: Limits) -> (Arc<Hub>, String) {
    let hub = Hub::with_limits(MemoryStore::default(), limits);
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

/// A store that runs `RACE` right after it builds a netmap: a change that
/// lands between the snapshot and the first frame.
#[derive(Default)]
struct Racy {
    inner: MemoryStore,
}

impl NodeStore for Racy {
    fn add_preauth(&mut self, key: &str, login: &str) {
        self.inner.add_preauth(key, login);
    }
    fn register(
        &mut self,
        machine: MachinePublic,
        node_key: NodePublic,
        hostname: &str,
        auth_key: &str,
    ) -> Result<(), RegisterError> {
        self.inner.register(machine, node_key, hostname, auth_key)
    }
    fn has_node_key(&self, key: &NodePublic) -> bool {
        self.inner.has_node_key(key)
    }
    fn update(&mut self, m: &MachinePublic, d: DiscoPublic, e: Vec<SocketAddr>) -> bool {
        self.inner.update(m, d, e)
    }
    fn netmap(&self, machine: &MachinePublic, omit_peers: bool) -> Option<MapResponse> {
        let map = self.inner.netmap(machine, omit_peers);
        if let Some(hook) = RACE.lock().unwrap().take() {
            hook();
        }
        map
    }
}

static RACE: Mutex<Option<Box<dyn FnOnce() + Send>>> = Mutex::new(None);

#[rusty_tokio::test]
async fn a_change_after_the_first_snapshot_still_wakes_the_stream() {
    let hub = Hub::with_store(Racy::default());
    hub.add_preauth("k1", "alice");
    let machine = MachinePublic([1; 32]);
    hub.locked(|s| s.register(machine, NodePublic([2; 32]), "a", "k1"))
        .unwrap();
    let racing = hub.clone();
    *RACE.lock().unwrap() = Some(Box::new(move || racing.bump()));

    let (_first, mut rx) = hub.snapshot(&machine, false).unwrap();

    let woke = rusty_tokio::time::timeout(Duration::from_secs(5), rx.changed()).await;
    assert!(
        woke.is_ok(),
        "the change was marked seen without being sent"
    );
}

/// Registers a node with the hub and returns its node key.
async fn registered(url: &str, name: &str) -> NodePrivate {
    let p = peer(url, name).await;
    p.client.register(p.node.public(), "k1").await.unwrap();
    p.node
}

fn payload(len: usize) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
        .collect()
}

#[rusty_tokio::test]
async fn two_registered_nodes_move_4_mib_through_the_hub_derp() {
    use ts_derp::DerpClient;
    // The default per-sender rate would drop a 4 MiB blast; this test is
    // about integrity, so the rate is out of the way.
    let (_hub, url) = start_with(Limits {
        derp_bytes_per_sec: 64 << 20,
        derp_burst: 16 << 20,
        ..Limits::default()
    })
    .await;
    let (ka, kb) = (registered(&url, "a").await, registered(&url, "b").await);
    let a = DerpClient::connect(&url, &ka).await.unwrap();
    let mut b = DerpClient::connect(&url, &kb).await.unwrap();

    let data = payload(4 << 20);
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
    let (_hub, url) = start().await;
    let addr = url.trim_start_matches("http://");
    let s = rusty_tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "GET /derp HTTP/1.1\r\nHost: {addr}\r\nUpgrade: DERP\r\nConnection: Upgrade\r\nDerp-Fast-Start: 1\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut head = [0u8; 5];
    s.read_exact(&mut head).await.unwrap();
    assert_eq!(head[0], 0x01, "first byte is the ServerKey frame, not HTTP");
}

/// Reads until the peer closes; false if it is still open after `wait`.
async fn closes_within(s: &mut rusty_tokio::net::TcpStream, wait: Duration) -> bool {
    use rusty_tokio::io::AsyncReadExt;
    let mut sink = [0u8; 256];
    let drained = rusty_tokio::time::timeout(wait, async {
        loop {
            match s.read(&mut sink).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
    .await;
    drained.is_ok()
}

#[rusty_tokio::test]
async fn a_connection_that_sends_no_request_is_closed_at_the_head_timeout() {
    let (_hub, url) = start_with(Limits {
        head_timeout: Duration::from_millis(200),
        ..Limits::default()
    })
    .await;
    let mut s = rusty_tokio::net::TcpStream::connect(url.trim_start_matches("http://"))
        .await
        .unwrap();
    assert!(closes_within(&mut s, Duration::from_secs(5)).await);
}

#[rusty_tokio::test]
async fn a_derp_handshake_that_stalls_is_closed_at_the_handshake_timeout() {
    let (_hub, url) = start_with(Limits {
        handshake_timeout: Duration::from_millis(200),
        ..Limits::default()
    })
    .await;
    let addr = url.trim_start_matches("http://");
    let mut s = rusty_tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "GET /derp HTTP/1.1\r\nHost: {addr}\r\nUpgrade: DERP\r\nDerp-Fast-Start: 1\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    // The ServerKey frame arrives; the ClientInfo never does.
    assert!(closes_within(&mut s, Duration::from_secs(5)).await);
}

#[rusty_tokio::test]
async fn connections_over_the_cap_are_closed() {
    let (_hub, url) = start_with(Limits {
        max_connections: 1,
        ..Limits::default()
    })
    .await;
    let addr = url.trim_start_matches("http://");
    let _held = rusty_tokio::net::TcpStream::connect(addr).await.unwrap();
    // Let the hub accept the first connection before the second arrives.
    rusty_tokio::time::sleep(Duration::from_millis(200)).await;
    let mut second = rusty_tokio::net::TcpStream::connect(addr).await.unwrap();
    assert!(closes_within(&mut second, Duration::from_secs(5)).await);
}

#[rusty_tokio::test]
async fn netmap_streams_over_the_cap_are_refused() {
    let (_hub, url) = start_with(Limits {
        max_map_streams: 0,
        ..Limits::default()
    })
    .await;
    let a = peer(&url, "a").await;
    a.client.register(a.node.public(), "k1").await.unwrap();
    let polled = a
        .client
        .poll_netmap(a.node.public(), a.disco.public(), vec![], |_| {
            ControlFlow::Break(())
        })
        .await;
    assert!(polled.is_err(), "a stream was served over the cap");
}

#[rusty_tokio::test]
async fn a_second_derp_node_is_refused_at_the_session_cap() {
    use ts_derp::DerpClient;
    let (_hub, url) = start_with(Limits {
        max_derp_sessions: 1,
        ..Limits::default()
    })
    .await;
    let (ka, kb) = (registered(&url, "a").await, registered(&url, "b").await);
    let _first = DerpClient::connect(&url, &ka).await.unwrap();
    let refused = async {
        let mut second = DerpClient::connect(&url, &kb).await?;
        Ok::<_, ts_derp::DerpError>(second.recv().await)
    };
    let outcome = rusty_tokio::time::timeout(Duration::from_secs(5), refused)
        .await
        .expect("the relay closes the refused session");
    assert!(
        matches!(outcome, Err(_) | Ok(None)),
        "second session stayed up"
    );
}

#[rusty_tokio::test]
async fn a_sender_over_its_rate_loses_packets_not_the_session() {
    use ts_derp::DerpClient;
    // Room for exactly four 16 KiB packets, and effectively no refill.
    let (_hub, url) = start_with(Limits {
        derp_bytes_per_sec: 1,
        derp_burst: 70_000,
        ..Limits::default()
    })
    .await;
    let (ka, kb) = (registered(&url, "a").await, registered(&url, "b").await);
    let a = DerpClient::connect(&url, &ka).await.unwrap();
    let mut b = DerpClient::connect(&url, &kb).await.unwrap();
    for _ in 0..10 {
        a.send(kb.public(), payload(16 << 10)).await.unwrap();
    }
    let mut got = 0;
    while rusty_tokio::time::timeout(Duration::from_millis(500), b.recv())
        .await
        .is_ok_and(|p| p.is_some())
    {
        got += 1;
    }
    assert_eq!(got, 4, "the burst, then drops");
    // The session is still usable after the drops.
    a.send(kb.public(), vec![1]).await.unwrap();
}

#[test]
fn an_advertised_region_rides_in_every_netmap() {
    let hub = Hub::new();
    hub.add_preauth("k1", "alice");
    let machine = MachinePublic([1; 32]);
    hub.locked(|s| s.register(machine, NodePublic([2; 32]), "a", "k1"))
        .unwrap();
    assert!(hub.netmap(&machine, false).unwrap().derp_map.is_none());
    hub.advertise("hub.example", 3340, 3478);
    let map = hub.netmap(&machine, false).unwrap().derp_map.unwrap();
    let node = &map["Regions"]["1"]["Nodes"][0];
    assert_eq!(node["HostName"], "hub.example");
    assert_eq!(node["DERPPort"], 3340);
    assert_eq!(node["STUNPort"], 3478);
}
