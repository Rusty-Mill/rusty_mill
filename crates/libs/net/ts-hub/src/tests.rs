//! End to end: the existing `ts-control` client against the hub, over real
//! TCP, Noise and HTTP/2.

use std::net::{IpAddr, SocketAddr};
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
    hub.set_policy(Policy::allow_all());
    hub.add_preauth("k1", "alice", KeyPolicy::REUSABLE).unwrap();
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

/// A store that runs `RACE` right after it builds a netmap: a change that
/// lands between the snapshot and the first frame.
#[derive(Default)]
struct Racy {
    inner: MemoryStore,
}

impl NodeStore for Racy {
    fn add_preauth(
        &mut self,
        key: &str,
        login: &str,
        policy: KeyPolicy,
        now: u64,
    ) -> Result<(), RegisterError> {
        self.inner.add_preauth(key, login, policy, now)
    }
    fn register(
        &mut self,
        machine: MachinePublic,
        node_key: NodePublic,
        hostname: &str,
        auth_key: &str,
        now: u64,
    ) -> Result<(), RegisterError> {
        self.inner
            .register(machine, node_key, hostname, auth_key, now)
    }
    fn remove_preauth(&mut self, key: &str) -> Result<bool, RegisterError> {
        self.inner.remove_preauth(key)
    }
    fn set_file_keys(
        &mut self,
        keys: Vec<(String, String, KeyPolicy)>,
        now: u64,
    ) -> Result<(), RegisterError> {
        self.inner.set_file_keys(keys, now)
    }
    fn revoke(&mut self, machine: &MachinePublic) -> Revocation {
        self.inner.revoke(machine)
    }
    fn has_node_key(&self, key: &NodePublic, now: u64) -> bool {
        self.inner.has_node_key(key, now)
    }
    fn update(&mut self, m: &MachinePublic, d: DiscoPublic, e: Vec<SocketAddr>) -> bool {
        self.inner.update(m, d, e)
    }
    fn netmap(&self, machine: &MachinePublic, omit_peers: bool, now: u64) -> Option<MapResponse> {
        let map = self.inner.netmap(machine, omit_peers, now);
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
    hub.add_preauth("k1", "alice", KeyPolicy::REUSABLE).unwrap();
    let machine = MachinePublic([1; 32]);
    hub.locked(|s| s.register(machine, NodePublic([2; 32]), "a", "k1", 0))
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
    hub.set_policy(Policy::allow_all());
    hub.add_preauth("k1", "alice", KeyPolicy::REUSABLE).unwrap();
    let machine = MachinePublic([1; 32]);
    hub.locked(|s| s.register(machine, NodePublic([2; 32]), "a", "k1", 0))
        .unwrap();
    assert!(hub.netmap(&machine, false).unwrap().derp_map.is_none());
    hub.advertise("hub.example", 3340, 3478);
    let map = hub.netmap(&machine, false).unwrap().derp_map.unwrap();
    let node = &map["Regions"]["1"]["Nodes"][0];
    assert_eq!(node["HostName"], "hub.example");
    assert_eq!(node["DERPPort"], 3340);
    assert_eq!(node["STUNPort"], 3478);
}

#[rusty_tokio::test]
async fn a_reconnect_with_the_same_key_closes_the_old_session() {
    use ts_derp::DerpClient;
    let (_hub, url) = start().await;
    let (ka, kb) = (registered(&url, "a").await, registered(&url, "b").await);
    let mut old = DerpClient::connect(&url, &ka).await.unwrap();
    let new = DerpClient::connect(&url, &ka).await.unwrap();
    let mut b = DerpClient::connect(&url, &kb).await.unwrap();

    let ended = rusty_tokio::time::timeout(Duration::from_secs(5), old.recv())
        .await
        .expect("the replaced session is closed");
    assert!(ended.is_none(), "the replaced session stayed open");

    // The replacement is the live session and still relays.
    new.send(kb.public(), vec![7]).await.unwrap();
    let got = rusty_tokio::time::timeout(Duration::from_secs(5), b.recv())
        .await
        .expect("relay delivers")
        .expect("b stays up");
    assert_eq!(got.peer, ka.public());
}

#[rusty_tokio::test]
async fn http2_streams_per_noise_connection_are_capped() {
    let (_hub, url) = start_with(Limits {
        max_streams_per_connection: 0,
        ..Limits::default()
    })
    .await;
    let p = peer(&url, "a").await;
    // A client with no stream budget either hangs or is refused; it is never served.
    let served = rusty_tokio::time::timeout(
        Duration::from_secs(2),
        p.client.register(p.node.public(), "k1"),
    )
    .await;
    assert!(
        !matches!(served, Ok(Ok(_))),
        "a stream was served over the cap"
    );
}

fn temp_table(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ts-hub-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("nodes.json")
}

#[test]
fn a_restarted_hub_keeps_its_nodes_and_spent_keys() {
    let path = temp_table("restart");
    let _ = std::fs::remove_file(&path);
    let mut store = FileStore::open(&path, None).unwrap();
    store
        .add_preauth("once", "alice", KeyPolicy::default(), 0)
        .unwrap();
    store
        .register(MachinePublic([1; 32]), NodePublic([1; 32]), "a", "once", 0)
        .unwrap();
    drop(store);

    let mut store = FileStore::open(&path, None).unwrap();
    assert!(store.has_node_key(&NodePublic([1; 32]), 0), "node survived");
    assert_eq!(
        store.register(MachinePublic([2; 32]), NodePublic([2; 32]), "b", "once", 0),
        Err(RegisterError::BadAuthKey),
        "the spent key stayed spent"
    );
    assert_eq!(store.revoke(&MachinePublic([1; 32])).saved, Ok(()));
    drop(store);
    let mut store = FileStore::open(&path, None).unwrap();
    assert!(
        !store.has_node_key(&NodePublic([1; 32]), 0),
        "revoke survived"
    );
    store
        .add_preauth("again", "alice", KeyPolicy::REUSABLE, 0)
        .unwrap();
    assert_eq!(
        store.register(MachinePublic([1; 32]), NodePublic([1; 32]), "a", "again", 0),
        Err(RegisterError::Revoked),
        "the ban survived the restart"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "the table holds secrets");
    }
}

#[test]
fn a_corrupt_table_is_an_error_not_an_empty_hub() {
    let path = temp_table("corrupt");
    std::fs::write(&path, b"{not json").unwrap();
    assert!(matches!(
        FileStore::open(&path, None),
        Err(StoreError::Corrupt { .. })
    ));
}

#[rusty_tokio::test]
async fn revoking_a_node_closes_its_relay_session_and_refuses_it_again() {
    use ts_derp::DerpClient;
    let (hub, url) = start().await;
    let machine_key = MachinePrivate::generate();
    let machine = machine_key.public();
    let node = NodePrivate::generate();
    let client = ControlClient::connect(&url, machine_key, Hostinfo::default())
        .await
        .unwrap();
    client.register(node.public(), "k1").await.unwrap();
    let mut relay = DerpClient::connect(&url, &node).await.unwrap();

    assert!(hub.revoke(&machine).unwrap());
    let ended = rusty_tokio::time::timeout(Duration::from_secs(5), relay.recv())
        .await
        .expect("the session is closed");
    assert!(ended.is_none(), "a revoked node kept relaying");
    assert!(DerpClient::connect(&url, &node).await.is_err());
    assert!(!hub.revoke(&machine).unwrap(), "already gone");
}

#[rusty_tokio::test]
async fn one_machine_cannot_hold_more_netmap_streams_than_its_cap() {
    let (_hub, url) = start_with(Limits {
        max_map_streams_per_machine: 1,
        ..Limits::default()
    })
    .await;
    let machine = MachinePrivate::generate().to_bytes();
    let connect = || async {
        ControlClient::connect(
            &url,
            MachinePrivate::from_bytes(machine),
            Hostinfo::default(),
        )
        .await
        .unwrap()
    };
    let (first, second) = (connect().await, connect().await);
    let (node, disco) = (
        NodePrivate::generate().public(),
        DiscoPrivate::generate().public(),
    );
    first.register(node, "k1").await.unwrap();
    // Holds the one stream open.
    rusty_tokio::spawn(async move {
        let _ = first
            .poll_netmap(node, disco, vec![], |_| ControlFlow::Continue(()))
            .await;
    });
    rusty_tokio::time::sleep(Duration::from_millis(300)).await;
    let refused = second
        .poll_netmap(node, disco, vec![], |_| ControlFlow::Break(()))
        .await;
    assert!(refused.is_err(), "a second stream was served over the cap");
}

#[test]
fn a_write_that_fails_leaves_no_node_admitted_in_memory() {
    let path = temp_table("unwritable");
    let _ = std::fs::remove_file(&path);
    let mut store = FileStore::open(&path, None).unwrap();
    store
        .add_preauth("k", "alice", KeyPolicy::REUSABLE, 0)
        .unwrap();
    // The directory vanishes under the store: every save now fails.
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();

    let refused = store.register(MachinePublic([1; 32]), NodePublic([1; 32]), "a", "k", 0);
    assert!(
        matches!(refused, Err(RegisterError::Storage(_))),
        "{refused:?}"
    );
    assert!(
        !store.has_node_key(&NodePublic([1; 32]), 0),
        "admitted anyway"
    );

    // A revoke still holds in memory, and says it is not durable.
    let revoked = store.revoke(&MachinePublic([2; 32]));
    assert!(matches!(revoked.saved, Err(RegisterError::Storage(_))));
}

#[cfg(unix)]
#[test]
fn a_stale_temp_file_does_not_keep_its_loose_mode() {
    use std::os::unix::fs::PermissionsExt;
    let path = temp_table("stale");
    let _ = std::fs::remove_file(&path);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, b"old").unwrap();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
    let mut store = FileStore::open(&path, None).unwrap();
    store
        .add_preauth("k", "alice", KeyPolicy::REUSABLE, 0)
        .unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0, "the saved table is group/world readable");
}

/// A hub with alice's laptop, bob's laptop and a tagged server `srv`.
fn three_nodes() -> (Arc<Hub>, [MachinePublic; 3]) {
    let hub = Hub::new();
    hub.add_preauth("alice", "alice@x.io", KeyPolicy::REUSABLE)
        .unwrap();
    hub.add_preauth("bob", "bob@x.io", KeyPolicy::REUSABLE)
        .unwrap();
    hub.add_preauth(
        "srv",
        "ops@x.io",
        KeyPolicy {
            tagged: Some("srv".into()),
            ..KeyPolicy::REUSABLE
        },
    )
    .unwrap();
    let machines = [1u8, 2, 3].map(|b| MachinePublic([b; 32]));
    for ((m, name), key) in machines
        .iter()
        .zip(["a", "b", "srv"])
        .zip(["alice", "bob", "srv"])
    {
        hub.locked(|s| s.register(*m, NodePublic([m.0[0]; 32]), name, key, 0))
            .unwrap();
    }
    (hub, machines)
}

fn ip_of_node(map: &MapResponse) -> IpAddr {
    map.node.as_ref().unwrap().addresses[0].addr
}

fn peer_names(map: &MapResponse) -> Vec<String> {
    let mut names: Vec<String> = map.peers.iter().flatten().map(|p| p.name.clone()).collect();
    names.sort();
    names
}

#[test]
fn a_hub_with_no_policy_shows_no_peers_and_admits_nothing() {
    let (hub, [a, ..]) = three_nodes();
    let map = hub.netmap(&a, false).unwrap();
    assert!(peer_names(&map).is_empty());
    assert_eq!(
        map.packet_filter,
        Some(Vec::new()),
        "an empty filter drops all"
    );
}

#[test]
fn a_rule_shows_only_the_peers_it_names_and_admits_only_them() {
    let (hub, [a, b, srv]) = three_nodes();
    hub.set_policy(Policy::parse("ts:alice@x.io -> ts-node:srv.mesh").unwrap());
    let (map_a, map_b, map_s) = (
        hub.netmap(&a, false).unwrap(),
        hub.netmap(&b, false).unwrap(),
        hub.netmap(&srv, false).unwrap(),
    );
    assert_eq!(peer_names(&map_a), ["srv.mesh."]);
    assert!(peer_names(&map_b).is_empty(), "bob has no rule with anyone");
    assert_eq!(peer_names(&map_s), ["a.mesh."]);

    // The server's filter lets alice in and nobody else; alice's lets nothing in.
    let (alice_ip, bob_ip, srv_ip) = (ip_of_node(&map_a), ip_of_node(&map_b), ip_of_node(&map_s));
    let at_srv = ts_filter::Filter::new(map_s.packet_filter.as_deref().unwrap());
    assert!(
        at_srv.allows(alice_ip, srv_ip, 6, 7000),
        "alice reaches srv"
    );
    assert!(!at_srv.allows(bob_ip, srv_ip, 6, 7000), "bob does not");
    let at_alice = ts_filter::Filter::new(map_a.packet_filter.as_deref().unwrap());
    assert!(
        !at_alice.allows(srv_ip, alice_ip, 6, 7000),
        "the rule is one way"
    );
}

#[test]
fn a_wildcard_source_admits_everyone_the_hub_shows() {
    let (hub, [a, b, srv]) = three_nodes();
    hub.set_policy(Policy::parse("* -> ts-node:srv.mesh").unwrap());
    let map_s = hub.netmap(&srv, false).unwrap();
    assert_eq!(peer_names(&map_s), ["a.mesh.", "b.mesh."]);
    let at_srv = ts_filter::Filter::new(map_s.packet_filter.as_deref().unwrap());
    let srv_ip = ip_of_node(&map_s);
    for m in [a, b] {
        assert!(at_srv.allows(ip_of_node(&hub.netmap(&m, false).unwrap()), srv_ip, 6, 1));
    }
    // Laptops do not see each other.
    assert_eq!(peer_names(&hub.netmap(&a, false).unwrap()), ["srv.mesh."]);
}

#[test]
fn omitting_peers_still_computes_the_filter_from_them() {
    let (hub, [_, _, srv]) = three_nodes();
    hub.set_policy(Policy::parse("ts:alice@x.io -> ts-node:srv.mesh").unwrap());
    let map = hub.netmap(&srv, true).unwrap();
    assert!(map.peers.is_none());
    assert_eq!(map.packet_filter.unwrap().len(), 1);
}

#[test]
fn principals_follow_the_mux_rule_tagged_node_by_name_else_login() {
    let (hub, [a, _, srv]) = three_nodes();
    hub.set_policy(Policy::allow_all());
    for (machine, want) in [(a, "ts:alice@x.io"), (srv, "ts-node:srv.mesh")] {
        let map = hub.netmap(&machine, false).unwrap();
        let profiles = map.user_profiles.clone().unwrap();
        assert_eq!(
            principal_of(map.node.as_ref().unwrap(), &profiles).as_deref(),
            Some(want)
        );
    }
}

#[rusty_tokio::test]
async fn changing_the_policy_wakes_the_netmap_streams() {
    let (hub, _) = three_nodes();
    let mut rx = hub.changes.clone();
    rx.borrow_and_update();
    hub.set_policy(Policy::allow_all());
    let woke = rusty_tokio::time::timeout(Duration::from_secs(5), rx.changed()).await;
    assert!(woke.is_ok(), "streams were not told about the new rules");
}

#[test]
fn a_removed_pre_auth_key_registers_no_one_but_keeps_its_nodes() {
    let (hub, [a, ..]) = three_nodes();
    assert_eq!(hub.remove_preauth("alice"), Ok(true));
    assert_eq!(hub.remove_preauth("alice"), Ok(false));
    let again =
        hub.locked(|s| s.register(MachinePublic([9; 32]), NodePublic([9; 32]), "x", "alice", 0));
    assert_eq!(again, Err(RegisterError::BadAuthKey));
    assert!(
        hub.locked(|s| s.has_node_key(&NodePublic([1; 32]), 0)),
        "its node stays"
    );
    assert_eq!(hub.revoke(&a), Ok(true));
}

// ---- TLS ----

use rusty_tls::{AsyncTlsStream, TrustPolicy};
use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A throwaway certificate for `localhost`: its PEMs and its DER.
fn localhost_cert() -> (String, String, Vec<u8>) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    (cert.pem(), key_pair.serialize_pem(), cert.der().to_vec())
}

async fn start_tls(limits: Limits) -> (Arc<Hub>, SocketAddr, Vec<u8>) {
    let (cert, key, der) = localhost_cert();
    let acceptor = tls_acceptor(&cert, &key).unwrap();
    let hub = Hub::with_limits(MemoryStore::default(), limits);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    rusty_tokio::spawn(hub.clone().serve_tls(listener, acceptor));
    (hub, addr, der)
}

async fn tls_connect(addr: SocketAddr, root: &[u8]) -> AsyncTlsStream<rusty_tokio::net::TcpStream> {
    let tcp = rusty_tokio::net::TcpStream::connect(addr).await.unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![root.to_vec().into()]);
    AsyncTlsStream::new(tcp, "localhost", &policy).unwrap()
}

#[rusty_tokio::test]
async fn the_key_endpoint_is_served_over_verified_tls() {
    let (hub, addr, der) = start_tls(Limits::default()).await;
    let mut tls = tls_connect(addr, &der).await;
    tls.write_all(b"GET /key HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let mut body = Vec::new();
    tls.read_to_end(&mut body).await.ok();
    let text = String::from_utf8_lossy(&body);
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(text.contains(&format!("{}", serde_json::json!(hub.public_key()))));
}

#[rusty_tokio::test]
async fn derp_fast_start_runs_over_tls() {
    let (_hub, addr, der) = start_tls(Limits::default()).await;
    let mut tls = tls_connect(addr, &der).await;
    tls.write_all(
        b"GET /derp HTTP/1.1\r\nHost: localhost\r\nUpgrade: DERP\r\nConnection: Upgrade\r\nDerp-Fast-Start: 1\r\n\r\n",
    )
    .await
    .unwrap();
    let mut head = [0u8; 5];
    tls.read_exact(&mut head).await.unwrap();
    assert_eq!(head[0], 0x01, "first byte is the ServerKey frame, not HTTP");
}

#[rusty_tokio::test]
async fn a_client_that_does_not_trust_the_certificate_is_refused() {
    let (_hub, addr, _der) = start_tls(Limits::default()).await;
    let (_, _, other) = localhost_cert();
    let mut tls = tls_connect(addr, &other).await;
    let sent = tls.write_all(b"GET /key HTTP/1.1\r\n\r\n").await;
    let mut sink = Vec::new();
    let read = tls.read_to_end(&mut sink).await;
    assert!(sent.is_err() || read.is_err(), "handshake should fail");
}

#[rusty_tokio::test]
async fn plain_http_on_the_tls_port_gets_no_answer() {
    let (_hub, addr, _der) = start_tls(Limits::default()).await;
    let mut s = rusty_tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(b"GET /key HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let mut got = Vec::new();
    let _ = rusty_tokio::time::timeout(Duration::from_secs(2), s.read_to_end(&mut got)).await;
    assert!(
        !String::from_utf8_lossy(&got).contains("legacyPublicKey"),
        "the key was served in the clear"
    );
}

#[rusty_tokio::test]
async fn a_tls_handshake_that_stalls_is_closed_at_the_handshake_timeout() {
    let limits = Limits {
        handshake_timeout: Duration::from_millis(200),
        ..Limits::default()
    };
    let (_hub, addr, _der) = start_tls(limits).await;
    let mut s = rusty_tokio::net::TcpStream::connect(addr).await.unwrap();
    assert!(closes_within(&mut s, Duration::from_secs(3)).await);
}

#[test]
fn a_bad_certificate_or_key_is_a_typed_error_without_file_content() {
    let (cert, key, _) = localhost_cert();
    assert!(tls_acceptor(&cert, &key).is_ok());
    assert!(matches!(
        tls_acceptor("", &key),
        Err(TlsConfigError::NoCertificate)
    ));
    assert!(matches!(
        tls_acceptor(&cert, ""),
        Err(TlsConfigError::NoPkcs8Key)
    ));
    assert!(matches!(
        tls_acceptor(&key, &key),
        Err(TlsConfigError::NoCertificate)
    ));
    let broken = "-----BEGIN CERTIFICATE-----\nAAAA\n";
    assert!(matches!(
        tls_acceptor(broken, &key),
        Err(TlsConfigError::Pem {
            file: "certificate",
            ..
        })
    ));
    // A bad character in the key is named by file, never echoed.
    let bad_key = "-----BEGIN PRIVATE KEY-----\nS3CR*T\n-----END PRIVATE KEY-----\n";
    let e = tls_acceptor(&cert, bad_key).err().unwrap();
    assert!(matches!(e, TlsConfigError::Pem { file: "key", .. }));
    assert!(!format!("{e} {e:?}").contains("S3CR"));
}

#[rusty_tokio::test]
async fn a_stalled_tls_handshake_holds_a_connection_slot() {
    let limits = Limits {
        max_connections: 1,
        ..Limits::default()
    };
    let (_hub, addr, _der) = start_tls(limits).await;
    let _stalled = rusty_tokio::net::TcpStream::connect(addr).await.unwrap();
    rusty_tokio::time::sleep(Duration::from_millis(100)).await;
    let mut second = rusty_tokio::net::TcpStream::connect(addr).await.unwrap();
    assert!(closes_within(&mut second, Duration::from_secs(2)).await);
}

// ---- keys file ----

#[test]
fn a_keys_file_reads_login_reusable_and_name() {
    let keys = parse_keys("# c\nk1 alice reusable\n\nk2 ci name=build1 # server\n").unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0], ("k1".into(), "alice".into(), KeyPolicy::REUSABLE));
    assert_eq!(keys[1].2.tagged.as_deref(), Some("build1"));
    assert!(!keys[1].2.reusable);
}

#[test]
fn keys_file_errors_name_the_line_not_the_key() {
    for (text, line) in [("k1\n", 1), ("k1 a\nk2 b nope\n", 2), ("k1 a\nk1 b\n", 2)] {
        let e = parse_keys(text).unwrap_err();
        assert_eq!(e.line, line, "{text}");
        assert!(!e.to_string().contains("k1"), "{e}");
    }
}

#[test]
fn reloading_keys_adds_new_drops_removed_and_keeps_spent_marks() {
    let hub = Hub::new();
    hub.load_keys("k1 alice\nk2 bob reusable\n").unwrap();
    hub.add_preauth("manual", "carol", KeyPolicy::REUSABLE)
        .unwrap();
    let m1 = MachinePublic([1; 32]);
    let m2 = MachinePublic([2; 32]);
    let node = |b: u8| NodePublic([b; 32]);
    hub.locked(|s| s.register(m1, node(1), "a", "k1", 0))
        .unwrap();
    // Reading the same file again must not unspend k1 for another machine.
    hub.load_keys("k1 alice\nk3 dave\n").unwrap();
    assert!(
        hub.locked(|s| s.register(m2, node(2), "b", "k1", 0))
            .is_err()
    );
    assert!(
        hub.locked(|s| s.register(m2, node(2), "b", "k2", 0))
            .is_err()
    );
    hub.locked(|s| s.register(m2, node(2), "b", "k3", 0))
        .unwrap();
    hub.locked(|s| s.register(MachinePublic([3; 32]), node(3), "c", "manual", 0))
        .unwrap();
}

#[test]
fn a_keys_file_that_does_not_parse_changes_nothing() {
    let hub = Hub::new();
    hub.load_keys("k1 alice\n").unwrap();
    assert!(hub.load_keys("k2 bob\nbroken\n").is_err());
    let m = MachinePublic([1; 32]);
    let n = NodePublic([1; 32]);
    assert!(hub.locked(|s| s.register(m, n, "a", "k2", 0)).is_err());
    hub.locked(|s| s.register(m, n, "a", "k1", 0)).unwrap();
}

#[test]
fn re_adding_a_key_with_other_settings_is_refused() {
    let hub = Hub::new();
    hub.add_preauth("k", "alice", KeyPolicy::default()).unwrap();
    assert!(matches!(
        hub.add_preauth("k", "alice", KeyPolicy::REUSABLE),
        Err(RegisterError::InvalidKey(_))
    ));
}

#[test]
fn a_key_dropped_while_the_hub_was_down_is_refused_after_the_restart() {
    let path = temp_table("dropped");
    let m = |b| MachinePublic([b; 32]);
    let n = |b| NodePublic([b; 32]);
    {
        let hub = Hub::with_store(FileStore::open(&path, None).unwrap());
        hub.load_keys("k1 alice reusable\nk2 bob reusable\n")
            .unwrap();
        hub.locked(|s| s.register(m(1), n(1), "a", "k1", 0))
            .unwrap();
    }
    let hub = Hub::with_store(FileStore::open(&path, None).unwrap());
    hub.load_keys("k2 bob reusable\n").unwrap();
    assert!(
        hub.locked(|s| s.register(m(2), n(2), "b", "k1", 0))
            .is_err()
    );
    hub.locked(|s| s.register(m(2), n(2), "b", "k2", 0))
        .unwrap();
    std::fs::remove_file(&path).ok();
}

#[test]
fn a_keys_file_with_a_refused_key_changes_nothing() {
    let path = temp_table("atomic");
    let hub = Hub::with_store(FileStore::open(&path, None).unwrap());
    hub.load_keys("k1 alice reusable\n").unwrap();
    // k2 is fine, the second line's login is reserved: the load is refused whole.
    let e = hub.load_keys("k2 bob reusable\nk3 tagged-devices\n");
    assert!(matches!(e, Err(LoadKeysError::Refused(_))));
    let m = |b| MachinePublic([b; 32]);
    assert!(
        hub.locked(|s| s.register(m(2), NodePublic([2; 32]), "b", "k2", 0))
            .is_err()
    );
    hub.locked(|s| s.register(m(1), NodePublic([1; 32]), "a", "k1", 0))
        .unwrap();
    // A key dropped by a refused load is still there, also after a restart.
    let e = hub.load_keys("k3 tagged-devices\n");
    assert!(e.is_err());
    drop(hub);
    let hub = Hub::with_store(FileStore::open(&path, None).unwrap());
    hub.locked(|s| s.register(m(1), NodePublic([1; 32]), "a", "k1", 0))
        .unwrap();
    std::fs::remove_file(&path).ok();
}

async fn join_as(
    url: &str,
    key: MachinePrivate,
    name: &str,
    auth: &str,
) -> Result<NodePrivate, ClientError> {
    let hostinfo = Hostinfo {
        hostname: name.into(),
        os: "linux".into(),
        ..Default::default()
    };
    let client = ControlClient::connect(url, key, hostinfo).await.unwrap();
    let node = NodePrivate::generate();
    client.register(node.public(), auth).await?;
    Ok(node)
}

#[rusty_tokio::test]
async fn a_standby_promoted_after_the_active_dies_keeps_the_key_the_nodes_and_the_revokes() {
    let root = std::env::temp_dir().join(format!("ts-hub-failover-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    let (active_dir, standby_dir) = (root.join("active"), root.join("standby"));
    std::fs::create_dir_all(&active_dir).unwrap();
    let secret = "0123456789abcdef0123456789abcdef";

    let identity = ts_key::NodeState::load_or_generate(&active_dir)
        .unwrap()
        .machine;
    let store = FileStore::open(active_dir.join("nodes.json"), None).unwrap();
    let active = Hub::with_identity(store, Limits::default(), identity);
    active.set_policy(Policy::allow_all());
    active
        .add_preauth("k1", "alice", KeyPolicy::REUSABLE)
        .unwrap();
    active.enable_replication(secret).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let serving = rusty_tokio::spawn(active.clone().serve(listener));

    let (key_a, key_b) = (MachinePrivate::generate(), MachinePrivate::generate());
    let node_a = join_as(&url, key_a.clone(), "a", "k1").await.unwrap();
    join_as(&url, key_b.clone(), "b", "k1").await.unwrap();
    assert!(active.revoke(&key_b.public()).unwrap());

    // The wrong secret gets nothing.
    let trust = ts_dial::Trust::System;
    let refused = replica::fetch(&url, "wrong-wrong-wrong-wrong", None, &trust).await;
    assert!(
        matches!(refused, Err(replica::ReplicaError::Status(401))),
        "{refused:?}"
    );

    let follower = {
        let (url, dir) = (url.clone(), standby_dir.clone());
        rusty_tokio::spawn(async move {
            replica::follow(&url, secret, &dir, &ts_dial::Trust::System).await
        })
    };
    let want: serde_json::Value =
        serde_json::from_slice(&active.locked(|s| s.export()).unwrap()).unwrap();
    let copied = async {
        loop {
            let have = std::fs::read(standby_dir.join("nodes.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
            if have.as_ref() == Some(&want) {
                return;
            }
            rusty_tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    rusty_tokio::time::timeout(Duration::from_secs(10), copied)
        .await
        .expect("the standby copies the table");

    // The active dies; the standby's hub opens the copy.
    serving.abort();
    follower.abort();
    let identity = ts_key::NodeState::load_or_generate(&standby_dir)
        .unwrap()
        .machine;
    let store = FileStore::open(standby_dir.join("nodes.json"), None).unwrap();
    let promoted = Hub::with_identity(store, Limits::default(), identity);
    promoted.set_policy(Policy::allow_all());
    assert_eq!(
        promoted.public_key(),
        active.public_key(),
        "nodes pin this key"
    );
    assert!(promoted.locked(|s| s.has_node_key(&node_a.public(), unix_now())));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    rusty_tokio::spawn(promoted.clone().serve(listener));

    join_as(&url, MachinePrivate::generate(), "c", "k1")
        .await
        .expect("a new node joins");
    let revoked = join_as(&url, key_b, "b", "k1").await;
    assert!(
        matches!(revoked, Err(ClientError::RegisterRejected(_))),
        "revoked stays revoked: {revoked:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[rusty_tokio::test]
async fn replication_is_off_until_enabled_and_needs_a_long_enough_secret() {
    let (hub, url) = start().await;
    let off = replica::fetch(&url, "0123456789abcdef", None, &ts_dial::Trust::System).await;
    assert!(
        matches!(off, Err(replica::ReplicaError::Status(404))),
        "{off:?}"
    );
    assert!(matches!(
        hub.enable_replication("short"),
        Err(replica::ReplicaError::WeakSecret)
    ));
}

async fn replicating_hub() -> (Arc<Hub>, String) {
    let hub = Hub::new();
    hub.enable_replication("0123456789abcdef0123456789abcdef")
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    rusty_tokio::spawn(hub.clone().serve(listener));
    (hub, url)
}

const SECRET: &str = "0123456789abcdef0123456789abcdef";

#[rusty_tokio::test]
async fn a_table_over_one_mib_is_copied() {
    let (hub, url) = replicating_hub().await;
    for i in 0..10_000 {
        hub.add_preauth(
            &format!("key-{i:08}-padding-padding"),
            "alice",
            KeyPolicy::REUSABLE,
        )
        .unwrap();
    }
    let snapshot = replica::fetch(&url, SECRET, None, &ts_dial::Trust::System)
        .await
        .expect("a big table is not refused");
    assert!(serde_json::to_vec(&snapshot.table).unwrap().len() > 1 << 20);
}

#[rusty_tokio::test]
async fn a_restarted_active_is_copied_even_when_its_counter_matches() {
    let (_, url) = replicating_hub().await;
    let trust = ts_dial::Trust::System;
    let held = replica::fetch(&url, SECRET, None, &trust).await.unwrap();

    // The same run with the same version has nothing new: the request waits.
    let waits = rusty_tokio::time::timeout(
        Duration::from_millis(400),
        replica::fetch(&url, SECRET, Some((held.epoch, held.version)), &trust),
    )
    .await;
    assert!(waits.is_err(), "nothing changed, so the active waits");

    // A different run (a restart) with the same version answers at once.
    let restarted = rusty_tokio::time::timeout(
        Duration::from_secs(5),
        replica::fetch(&url, SECRET, Some((held.epoch + 1, held.version)), &trust),
    )
    .await;
    assert!(restarted.expect("answers at once").is_ok());
}

#[rusty_tokio::test]
async fn only_a_refused_connection_counts_as_a_hub_being_down() {
    let (_hub, url) = replicating_hub().await;
    assert!(
        replica::is_down(&url).await.is_err(),
        "it accepts connections"
    );

    let gone = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", gone.local_addr().unwrap());
    drop(gone);
    assert!(
        replica::is_down(&url).await.is_ok(),
        "nothing listens there"
    );
    assert!(
        replica::is_down("ftp://x").await.is_err(),
        "a bad URL is not 'down'"
    );
}

#[rusty_tokio::test]
async fn every_netmap_lists_the_advertised_standbys() {
    let (hub, url) = start().await;
    hub.add_preauth("k1", "alice", KeyPolicy::REUSABLE).unwrap();
    hub.set_policy(Policy::allow_all());
    let a = peer(&url, "a").await;
    a.client.register(a.node.public(), "k1").await.unwrap();

    let standbys = vec!["http://10.0.0.2:8443".to_string()];
    hub.advertise_standbys(standbys.clone());
    let mut got = None;
    rusty_tokio::time::timeout(
        Duration::from_secs(10),
        a.client
            .poll_netmap(a.node.public(), a.disco.public(), vec![], |map| {
                if map.peers.is_none() {
                    return ControlFlow::Continue(());
                }
                got = map.standby_urls;
                ControlFlow::Break(())
            }),
    )
    .await
    .expect("a netmap arrives")
    .unwrap();
    assert_eq!(got, Some(standbys));
}
