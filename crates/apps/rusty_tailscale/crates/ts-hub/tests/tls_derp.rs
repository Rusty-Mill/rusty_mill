//! Two clients on the hub's TLS DERP port, talking both ways at once like
//! stock `tailscaled` does: A sends numbered packets, B echoes each one.

use std::time::Duration;

use crypto_box::aead::{Aead, AeadCore, OsRng};
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use rusty_tls::{AsyncTlsStream, TlsAcceptor, TrustPolicy};
use rusty_tokio::io::{AsyncWriteExt, SplitReadHalf, SplitWriteHalf};
use rusty_tokio::net::{TcpListener, TcpStream};
use ts_control::ControlClient;
use ts_derp::frame::{self, FrameType, MAX_FRAME_SIZE};
use ts_hub::Hub;
use ts_key::{MachinePrivate, NodePrivate};
use ts_types::NodePublic;
use ts_types::tailcfg::Hostinfo;

const CERT: &str = include_str!("data/cert.pem");
const KEY: &str = include_str!("data/key.pem");
const PACKETS: usize = 2000;
const SIZE: usize = 1200;
const WINDOW: usize = 50;

fn der(pem: &str, label: &str) -> Vec<u8> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    assert!(pem.contains(label));
    rusty_base64::decode_standard(&body).unwrap()
}

async fn register(url: &str, name: &str) -> NodePrivate {
    let hostinfo = Hostinfo {
        hostname: name.into(),
        ..Default::default()
    };
    let client = ControlClient::connect(url, MachinePrivate::generate(), hostinfo)
        .await
        .unwrap();
    let node = NodePrivate::generate();
    client.register(node.public(), "k1").await.unwrap();
    node
}

type Tls = AsyncTlsStream<TcpStream>;

async fn connect(
    addr: std::net::SocketAddr,
    node: &NodePrivate,
    server: &NodePublic,
) -> (SplitReadHalf<Tls>, SplitWriteHalf<Tls>) {
    let pin = rusty_tls::fingerprint_sha256(&der(CERT, "CERTIFICATE")).unwrap();
    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut tls =
        AsyncTlsStream::new(tcp, "hub", &TrustPolicy::PinnedFingerprints(vec![pin])).unwrap();
    tls.write_all(b"GET /derp HTTP/1.1\r\nHost: hub\r\nUpgrade: DERP\r\nConnection: Upgrade\r\nDerp-Fast-Start: 1\r\n\r\n")
        .await
        .unwrap();
    let (mut rd, mut wr) = rusty_tokio::io::split(tls);
    let (t, greeting) = frame::read_frame(&mut rd, 1024).await.unwrap();
    assert_eq!(t, FrameType::ServerKey);
    assert_eq!(&greeting[8..40], &server.0);
    let salsa = SalsaBox::new(
        &PublicKey::from(server.0),
        &SecretKey::from(node.to_bytes()),
    );
    let nonce = SalsaBox::generate_nonce(&mut OsRng);
    let boxed = salsa.encrypt(&nonce, &br#"{"version":2}"#[..]).unwrap();
    let mut info = node.public().0.to_vec();
    info.extend_from_slice(&nonce);
    info.extend_from_slice(&boxed);
    frame::write_frame(&mut wr, FrameType::ClientInfo, &info)
        .await
        .unwrap();
    let (t, _) = frame::read_frame(&mut rd, 1024).await.unwrap();
    assert_eq!(t, FrameType::ServerInfo);
    (rd, wr)
}

#[rusty_tokio::test]
async fn both_ways_over_tls_derp_without_stalling() {
    let hub = Hub::new();
    hub.add_preauth("k1", "alice");
    let control = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", control.local_addr().unwrap());
    rusty_tokio::spawn(hub.clone().serve(control));
    let tls = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tls_addr = tls.local_addr().unwrap();
    let acceptor =
        TlsAcceptor::new(vec![der(CERT, "CERTIFICATE")], der(KEY, "PRIVATE KEY")).unwrap();
    rusty_tokio::spawn(hub.clone().serve_derp_tls(tls, acceptor));

    let (a, b) = (register(&url, "a").await, register(&url, "b").await);
    let server = hub.derp_public_key();
    let (mut a_rd, mut a_wr) = connect(tls_addr, &a, &server).await;
    let (mut b_rd, mut b_wr) = connect(tls_addr, &b, &server).await;

    // B echoes every packet back to A.
    let a_pub = a.public();
    rusty_tokio::spawn(async move {
        while let Ok((t, p)) = frame::read_frame(&mut b_rd, MAX_FRAME_SIZE).await {
            if t != FrameType::RecvPacket {
                continue;
            }
            let mut out = a_pub.0.to_vec();
            out.extend_from_slice(&p[32..]);
            if frame::write_frame(&mut b_wr, FrameType::SendPacket, &out)
                .await
                .is_err()
            {
                return;
            }
        }
    });
    // At most WINDOW packets in flight: the relay drops on a full queue, as
    // Go's does, so an unpaced blast would lose packets by design.
    let b_pub = b.public();
    let mut echoed = 0;
    let mut sent = 0;
    let run = rusty_tokio::time::timeout(Duration::from_secs(20), async {
        while echoed < PACKETS {
            while sent < PACKETS && sent - echoed < WINDOW {
                let mut out = b_pub.0.to_vec();
                out.extend_from_slice(&[sent as u8; SIZE]);
                frame::write_frame(&mut a_wr, FrameType::SendPacket, &out)
                    .await
                    .unwrap();
                sent += 1;
            }
            let (t, _) = frame::read_frame(&mut a_rd, MAX_FRAME_SIZE).await.unwrap();
            if t == FrameType::RecvPacket {
                echoed += 1;
            }
        }
    })
    .await;
    assert!(run.is_ok(), "stalled after {echoed} of {PACKETS} echoes");
}
