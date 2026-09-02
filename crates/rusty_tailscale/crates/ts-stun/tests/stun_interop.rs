//! Socket-level round-trip: the client sends an RFC-5389 binding request to
//! a STUN responder over a real UDP socket and parses the XOR-MAPPED-ADDRESS
//! back. (Public STUN servers aren't reachable from this environment — UDP
//! egress is blocked — so the responder runs locally; the wire bytes are
//! RFC-standard and encoded independently of the parser under test.)

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;
use ts_stun::{TxId, binding_request, parse_response};

/// Encodes a STUN binding success response with the source in
/// XOR-MAPPED-ADDRESS, the way a real server (Go `stun.Response`) does.
fn success_response(tx: TxId, src: SocketAddr) -> Vec<u8> {
    const COOKIE: [u8; 4] = [0x21, 0x12, 0xa4, 0x42];
    let (fam, addr): (u8, Vec<u8>) = match src.ip() {
        IpAddr::V4(v4) => (0x01, v4.octets().to_vec()),
        IpAddr::V6(v6) => (0x02, v6.octets().to_vec()),
    };
    let attr_len = 4 + addr.len();
    let mut b = vec![0x01, 0x01];
    b.extend_from_slice(&(attr_len as u16 + 4).to_be_bytes());
    b.extend_from_slice(&COOKIE);
    b.extend_from_slice(&tx.0);
    b.extend_from_slice(&0x0020u16.to_be_bytes());
    b.extend_from_slice(&(attr_len as u16).to_be_bytes());
    b.push(0);
    b.push(fam);
    let xport = src.port() ^ u16::from_be_bytes([COOKIE[0], COOKIE[1]]);
    b.extend_from_slice(&xport.to_be_bytes());
    let mut key = [0u8; 16];
    key[..4].copy_from_slice(&COOKIE);
    key[4..].copy_from_slice(&tx.0);
    for (i, byte) in addr.iter().enumerate() {
        b.push(byte ^ key[i]);
    }
    b
}

#[tokio::test]
async fn binding_request_round_trip_over_udp() {
    let server = UdpSocket::bind("127.0.0.1:0").await.expect("bind server");
    let server_addr = server.local_addr().unwrap();

    // Responder: echo the client's public address per STUN.
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        let (n, from) = server.recv_from(&mut buf).await.unwrap();
        // Transaction ID lives at bytes 8..20 of the request.
        let mut tx = [0u8; 12];
        tx.copy_from_slice(&buf[8..20.min(n)]);
        let resp = success_response(TxId(tx), from);
        server.send_to(&resp, from).await.unwrap();
    });

    let client = UdpSocket::bind("127.0.0.1:0").await.expect("bind client");
    let client_port = client.local_addr().unwrap().port();
    let tx = TxId::random();
    client
        .send_to(&binding_request(tx), server_addr)
        .await
        .unwrap();

    let mut buf = [0u8; 512];
    let n = tokio::time::timeout(Duration::from_secs(3), client.recv(&mut buf))
        .await
        .expect("response in time")
        .expect("recv");

    let reflexive = parse_response(&buf[..n], tx).expect("parse reflexive addr");
    assert_eq!(reflexive.ip(), IpAddr::from([127, 0, 0, 1]));
    assert_eq!(reflexive.port(), client_port, "server saw our source port");
}
