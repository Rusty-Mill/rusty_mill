//! Interop test: relay a packet between two DERP clients through a real DERP
//! server (Headscale's embedded DERP at http://127.0.0.1:8080/derp, started
//! by `interop/up.sh`). Skips with a message if no relay is reachable.

use std::time::Duration;

use ts_derp::DerpClient;
use ts_key::NodePrivate;

const DERP_URL: &str = "http://127.0.0.1:8080";

async fn relay_reachable() -> bool {
    tokio::net::TcpStream::connect("127.0.0.1:8080")
        .await
        .is_ok()
}

#[tokio::test]
async fn relay_packet_between_two_clients() {
    if !relay_reachable().await {
        eprintln!("SKIPPED: no DERP relay on 127.0.0.1:8080 (run interop/up.sh)");
        return;
    }

    let key_a = NodePrivate::generate();
    let key_b = NodePrivate::generate();
    let pub_a = key_a.public();
    let pub_b = key_b.public();

    let client_a = DerpClient::connect(DERP_URL, &key_a)
        .await
        .expect("client A connects to DERP");
    let mut client_b = DerpClient::connect(DERP_URL, &key_b)
        .await
        .expect("client B connects to DERP");

    // Both learned the same server key from the greeting.
    assert_eq!(client_a.server_key(), client_b.server_key());

    // Give the server a moment to register both clients' presence.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let msg = b"wireguard-over-derp payload".to_vec();
    client_a
        .send(pub_b, msg.clone())
        .await
        .expect("A sends to B");

    let got = tokio::time::timeout(Duration::from_secs(5), client_b.recv())
        .await
        .expect("B receives before timeout")
        .expect("B receives a packet");

    assert_eq!(got.payload, msg, "relayed payload matches");
    assert_eq!(got.peer, pub_a, "source key is A");
}

#[tokio::test]
async fn connect_learns_server_key() {
    if !relay_reachable().await {
        eprintln!("SKIPPED: no DERP relay on 127.0.0.1:8080");
        return;
    }
    let key = NodePrivate::generate();
    let client = DerpClient::connect(DERP_URL, &key)
        .await
        .expect("connect + handshake");
    // Headscale advertises the DERP key in the upgrade header; the greeting
    // frame must carry a non-zero key too.
    assert_ne!(client.server_key(), ts_types::NodePublic([0u8; 32]));
}
