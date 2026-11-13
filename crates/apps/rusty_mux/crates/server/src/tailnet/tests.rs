use super::*;
use rusty_tokio::{Builder, LocalOptions};

fn principal_from(json: &str) -> Result<Principal, String> {
    principal_of(&serde_json::from_str(json).unwrap())
}

#[test]
fn a_users_node_is_its_login() {
    let p = principal_from(
        r#"{"Node":{"Name":"lap.t.ts.net."},"UserProfile":{"LoginName":"nano@example.com"}}"#,
    )
    .unwrap();
    assert_eq!((p.id(), p.user()), ("ts:nano@example.com", ""));
}

#[test]
fn a_tagged_node_is_its_name_whatever_its_user() {
    let p = principal_from(
        r#"{"Node":{"Name":"srv.t.ts.net.","Tags":["tag:server"]},"UserProfile":{"LoginName":"tagged-devices"}}"#,
    )
    .unwrap();
    assert_eq!((p.id(), p.user()), ("ts-node:srv.t.ts.net", ""));
}

#[test]
fn an_unknown_or_userless_peer_is_refused() {
    assert!(principal_from("{}").is_err());
    assert!(principal_from(r#"{"Node":{"Name":"x."}}"#).is_err());
    assert!(principal_from(r#"{"Node":{"Name":"x."},"UserProfile":{"LoginName":""}}"#).is_err());
}

#[test]
fn a_shared_or_nameless_identity_is_refused() {
    // Every tagged node's login is the same, and an empty name is shared too.
    assert!(
        principal_from(r#"{"Node":{"Name":"x."},"UserProfile":{"LoginName":"tagged-devices"}}"#)
            .is_err()
    );
    assert!(principal_from(r#"{"Node":{"Name":".","Tags":["tag:a"]}}"#).is_err());
    assert!(principal_from(r#"{"Node":{"Name":"","Tags":["tag:a"]}}"#).is_err());
}

#[test]
fn a_tailnet_name_never_reaches_the_user_field() {
    // `deploy` must not be read as the local account `deploy`.
    let p = principal_from(r#"{"Node":{"Name":"deploy.","Tags":["tag:a"]}}"#).unwrap();
    assert_eq!(p.user(), "");
}

#[test]
fn an_ipv6_peer_is_escaped_in_the_query() {
    let peer: SocketAddr = "[fd7a:115c::1]:4000".parse().unwrap();
    assert_eq!(encode_addr(peer), "%5Bfd7a:115c::1%5D:4000");
    let zoned: SocketAddr = "[fe80::1%2]:4000".parse().unwrap();
    assert_eq!(encode_addr(zoned), "%5Bfe80::1%252%5D:4000");
}

/// Answers one request on a fresh socket with `status` and `body`, and
/// hands back the request line it saw.
fn fake_localapi(
    name: &str,
    status: &str,
    body: &str,
) -> (PathBuf, std::thread::JoinHandle<String>) {
    let path =
        std::env::temp_dir().join(format!("rusty-mux-ts-{}-{name}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{body}",
        body.len()
    );
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut s, _) = listener.accept().unwrap();
        let mut seen = Vec::new();
        let mut buf = [0u8; 1024];
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = s.read(&mut buf).unwrap();
            seen.extend_from_slice(&buf[..n]);
        }
        s.write_all(reply.as_bytes()).unwrap();
        String::from_utf8_lossy(&seen)
            .lines()
            .next()
            .unwrap()
            .to_owned()
    });
    (path, server)
}

fn block_on<T>(f: impl std::future::Future<Output = T>) -> T {
    Builder::new_current_thread()
        .build_local(&LocalOptions::default())
        .unwrap()
        .block_on(f)
}

#[test]
fn asks_tailscaled_and_maps_the_answer() {
    let (path, seen) = fake_localapi(
        "ok",
        "200 OK",
        r#"{"Node":{"Name":"lap.t.ts.net."},"UserProfile":{"LoginName":"nano@example.com"}}"#,
    );
    let peer: SocketAddr = "100.64.0.7:5000".parse().unwrap();
    let p = block_on(Whois::new(&path).principal(peer)).unwrap();
    assert_eq!(p.id(), "ts:nano@example.com");
    assert_eq!(
        seen.join().unwrap(),
        "GET /localapi/v0/whois?addr=100.64.0.7:5000 HTTP/1.1"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_refusal_from_tailscaled_carries_its_reason() {
    let (path, seen) = fake_localapi("refused", "404 Not Found", "peer not found");
    let peer: SocketAddr = "100.64.0.9:5000".parse().unwrap();
    let err = block_on(Whois::new(&path).principal(peer)).unwrap_err();
    assert!(err.to_string().contains("peer not found"), "{err}");
    seen.join().unwrap();
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_missing_socket_names_the_path() {
    let path = std::env::temp_dir().join("rusty-mux-ts-absent.sock");
    let err = block_on(Whois::new(&path).principal("100.64.0.1:1".parse().unwrap())).unwrap_err();
    assert!(
        err.to_string().contains("rusty-mux-ts-absent.sock"),
        "{err}"
    );
}

#[test]
fn a_stalled_tailscaled_times_out() {
    let path = std::env::temp_dir().join(format!("rusty-mux-ts-{}-stall.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    // Accepts and says nothing until the client gives up.
    let held = std::thread::spawn(move || {
        let (s, _) = listener.accept().unwrap();
        std::thread::sleep(DEADLINE + std::time::Duration::from_secs(1));
        drop(s);
    });
    let err = block_on(Whois::new(&path).principal("100.64.0.3:1".parse().unwrap())).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");
    held.join().unwrap();
    let _ = std::fs::remove_file(path);
}

#[test]
fn only_tailnet_addresses_are_tailnet_addresses() {
    let yes = [
        "100.64.0.1",
        "100.127.255.254",
        "fd7a:115c:a1e0::1",
        "fd7a:115c:a1e0:ab12::9",
    ];
    let no = [
        "100.63.255.255",
        "100.128.0.1",
        "127.0.0.1",
        "0.0.0.0",
        "192.168.1.1",
        "::1",
        "::",
        "fd7a:115c:a1e1::1",
        "fe80::1",
    ];
    for ip in yes {
        assert!(is_tailnet_addr(ip.parse().unwrap()), "{ip}");
    }
    for ip in no {
        assert!(!is_tailnet_addr(ip.parse().unwrap()), "{ip}");
    }
}
