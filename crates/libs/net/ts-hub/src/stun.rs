//! The hub's STUN responder (RFC 5389 binding requests only): tells a node
//! its server-reflexive UDP address so it can advertise a direct endpoint.

use std::net::{IpAddr, SocketAddr};

use rusty_tokio::net::UdpSocket;

const COOKIE: [u8; 4] = [0x21, 0x12, 0xa4, 0x42];
const HEADER_LEN: usize = 20;
const BINDING_REQUEST: [u8; 2] = [0x00, 0x01];
const BINDING_SUCCESS: [u8; 2] = [0x01, 0x01];
const ATTR_XOR_MAPPED_ADDRESS: [u8; 2] = [0x00, 0x20];

/// The binding success response for `request` as seen from `from`, or
/// `None` when `request` is not a well-formed binding request.
pub fn respond(request: &[u8], from: SocketAddr) -> Option<Vec<u8>> {
    if request.len() < HEADER_LEN || request[..2] != BINDING_REQUEST || request[4..8] != COOKIE {
        return None;
    }
    let declared = usize::from(u16::from_be_bytes([request[2], request[3]]));
    if request.len() != HEADER_LEN + declared {
        return None;
    }
    let mut key = [0u8; 16];
    key[..4].copy_from_slice(&COOKIE);
    key[4..].copy_from_slice(&request[8..20]);

    let (family, addr): (u8, Vec<u8>) = match from.ip() {
        IpAddr::V4(ip) => (0x01, ip.octets().to_vec()),
        IpAddr::V6(ip) => (0x02, ip.octets().to_vec()),
    };
    let mut attr = vec![0, family];
    attr.extend_from_slice(&(from.port() ^ 0x2112).to_be_bytes());
    attr.extend(addr.iter().zip(key).map(|(a, k)| a ^ k));

    let mut out = Vec::with_capacity(HEADER_LEN + 4 + attr.len());
    out.extend_from_slice(&BINDING_SUCCESS);
    out.extend_from_slice(&((4 + attr.len()) as u16).to_be_bytes());
    out.extend_from_slice(&request[4..20]);
    out.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS);
    out.extend_from_slice(&(attr.len() as u16).to_be_bytes());
    out.extend_from_slice(&attr);
    Some(out)
}

/// Answers binding requests on `socket` until it fails. A `ConnectionReset`
/// is logged and skipped: on Windows an ICMP port-unreachable for an earlier reply
/// surfaces as `ConnectionReset` on the next receive, and one bad packet must
/// not stop the responder.
pub async fn serve(socket: UdpSocket) -> std::io::Result<()> {
    let mut buf = [0u8; 1500];
    loop {
        let (n, from) = match socket.recv_from(&mut buf).await {
            Ok(got) => got,
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                rusty_log::debug!("stun: peer unreachable, continuing: {e}");
                continue;
            }
            Err(e) => return Err(e),
        };
        if let Some(reply) = respond(&buf[..n], from) {
            // A lost reply is a lost probe; the client retries.
            let _ = socket.send_to(&reply, from).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ts_stun::{TxId, binding_request, parse_response};

    use super::*;

    #[test]
    fn the_client_parses_what_the_server_sends() {
        for from in ["192.0.2.7:51820", "[2001:db8::7]:4242"] {
            let from: SocketAddr = from.parse().unwrap();
            let tx = TxId::random();
            let reply = respond(&binding_request(tx), from).unwrap();
            assert_eq!(parse_response(&reply, tx).unwrap(), from);
        }
    }

    #[test]
    fn anything_but_a_binding_request_is_ignored() {
        let from: SocketAddr = "192.0.2.7:1".parse().unwrap();
        let good = binding_request(TxId::random());
        assert!(respond(&good[..19], from).is_none());
        let mut wrong_cookie = good;
        wrong_cookie[4] ^= 1;
        assert!(respond(&wrong_cookie, from).is_none());
        let mut wrong_type = good;
        wrong_type[1] = 0x02;
        assert!(respond(&wrong_type, from).is_none());
        let mut bad_len = good;
        bad_len[3] = 8;
        assert!(respond(&bad_len, from).is_none());
    }

    #[rusty_tokio::test]
    async fn a_socket_round_trip_reports_the_senders_address() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server_addr = server.local_addr().unwrap();
        rusty_tokio::spawn(serve(server));
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let tx = TxId::random();
        client
            .send_to(&binding_request(tx), server_addr)
            .await
            .unwrap();
        let mut buf = [0u8; 1500];
        let (n, _) = rusty_tokio::time::timeout(Duration::from_secs(5), client.recv_from(&mut buf))
            .await
            .expect("reply in time")
            .unwrap();
        assert_eq!(
            parse_response(&buf[..n], tx).unwrap(),
            client.local_addr().unwrap()
        );
    }
}
