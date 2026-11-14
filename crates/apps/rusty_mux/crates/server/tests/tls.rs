//! The TLS listener (#506, D236, D237): a peer with the right token is a
//! guest, and one with a wrong token, or none, is closed without the
//! protocol ever starting. The certificate is a throwaway self-signed one for
//! `localhost` (its key is a test key and protects nothing).

#![cfg(target_os = "linux")]

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rusty_mux_proto::{Features, Message, decode, encode};
use rusty_mux_script::token::{self, Tokens};
use rusty_mux_server::{Config, Sharing, Tls, serve_shared};
use rusty_tls::{AsyncTlsStream, TrustPolicy};
use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt, TcpListener, TcpStream, UnixListener};
use rusty_tokio::sync::{mpsc, oneshot};
use rusty_tokio::{Builder, LocalOptions};

const CERT: &str = "MIIBlTCCATugAwIBAgIUNVCXzQ2FEC2YXUZSvYW3emOili0wCgYIKoZIzj0EAwIwFDESMBAGA1UEAwwJbG9jYWxob3N0MCAXDTI2MTExMzIxMDUwMVoYDzIxMjYxMDIwMjEwNTAxWjAUMRIwEAYDVQQDDAlsb2NhbGhvc3QwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAASGBIhUN8Mhly8WdTpSksdLzUi0wz+clGPZsfzZyD7dFD9T7CvJQK3hp5wfwXMx41/9AwO1zAKomyfSZ3t/4nLEo2kwZzAdBgNVHQ4EFgQUdz4TCJsOC9W3ESAwqTBkw2vpuLUwHwYDVR0jBBgwFoAUdz4TCJsOC9W3ESAwqTBkw2vpuLUwDwYDVR0TAQH/BAUwAwEB/zAUBgNVHREEDTALgglsb2NhbGhvc3QwCgYIKoZIzj0EAwIDSAAwRQIhAM/uQE1htwBDUiTWpEzxPvFjWH7CUGDzIOpg3S+ILq5SAiA/2pdwmEPSIk0accKOYLRpcxeOQGWPugOzlJHK3rV74w==";
const KEY: &str = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg+i39n8x4u+BcjKalH5jKPPeqGm3LgDG6fHidpE1ifRihRANCAASGBIhUN8Mhly8WdTpSksdLzUi0wz+clGPZsfzZyD7dFD9T7CvJQK3hp5wfwXMx41/9AwO1zAKomyfSZ3t/4nLE";

const TOKEN: [u8; token::LEN] = [7; token::LEN];
const PATIENCE: Duration = Duration::from_secs(20);

fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rusty-mux-tls-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).expect("scratch");
    dir
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    path
}

struct Server {
    addr: SocketAddr,
    stop: oneshot::Sender<()>,
    reload: mpsc::UnboundedSender<rusty_mux_server::Reload>,
}

/// A shared server with a TLS listener whose allowlist is `allow` and whose
/// token file says the token `laptop` is [`TOKEN`].
fn start(dir: &Path, allow: &str) -> Server {
    let pem = |label: &str, body: &str| {
        format!("-----BEGIN {label}-----\n{body}\n-----END {label}-----\n")
    };
    let cert = write(dir, "cert.pem", &pem("CERTIFICATE", CERT));
    let key = write(dir, "key.pem", &pem("PRIVATE KEY", KEY));
    let mut tokens = Tokens::default();
    tokens.create("laptop", &TOKEN).expect("token");
    let tokens = write(dir, "tokens", &tokens.text());
    let allow = write(dir, "allow", allow);
    let socket = dir.join("s.sock");
    let (reload, reloads) = mpsc::unbounded_channel();
    let (stop, stopped) = oneshot::channel::<()>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime");
        runtime.block_on(async move {
            let unix = UnixListener::bind(&socket).expect("bind");
            let tcp = TcpListener::bind("127.0.0.1:0").await.expect("bind tcp");
            ready_tx
                .send(tcp.local_addr().expect("addr"))
                .expect("ready");
            let tls = Tls::new(tcp, &cert, &key, tokens).expect("tls");
            let config = Config {
                program: "/bin/sh".into(),
                args: Vec::new(),
                cwd: "/".into(),
                console_host: rusty_mux_server::ConsoleHost::default(),
                state_dir: None,
                grapheme_clustering: false,
                config_file: None,
            };
            let sharing = Sharing {
                allow,
                tailnet: None,
                tls: Some(tls),
            };
            let _ = serve_shared(unix, config, sharing, reloads, async {
                let _ = stopped.await;
            })
            .await;
        });
    });
    Server {
        addr: ready_rx.recv_timeout(PATIENCE).expect("bound"),
        stop,
        reload,
    }
}

/// Connects over TLS and sends `first` (a token, or whatever the test
/// wants), then a Hello; returns the first reply, or `None` if the server
/// closed the connection instead.
fn try_in(addr: SocketAddr, first: &[u8]) -> Option<Message> {
    let runtime = Builder::new_current_thread()
        .build_local(&LocalOptions::default())
        .expect("runtime");
    runtime.block_on(async {
        rusty_tokio::time::timeout(PATIENCE, async {
            let tcp = TcpStream::connect(addr).await.ok()?;
            let mut tls =
                AsyncTlsStream::new(tcp, "localhost", &TrustPolicy::DangerNoVerification).ok()?;
            tls.write_all(first).await.ok()?;
            let hello = Message::Hello {
                version: None,
                features: Features::CHECKPOINT_V6,
                resume: None,
                takeover: None,
            };
            tls.write_all(&encode(&hello).expect("encode")).await.ok()?;
            let mut buf = Vec::new();
            loop {
                if let Some((message, _)) = decode(&buf).ok().flatten() {
                    return Some(message);
                }
                let mut chunk = [0u8; 4096];
                match tls.read(&mut chunk).await {
                    Ok(0) | Err(_) => return None,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
        })
        .await
        .expect("answered or closed in time")
    })
}

/// Connects with a valid token, calls `revoke` (at the door when `early`,
/// else once the server has answered), and tells whether the server then
/// closed the connection. Whatever it still said before closing is ignored.
fn closed_after_revoke(addr: SocketAddr, early: bool, revoke: impl FnOnce()) -> bool {
    let runtime = Builder::new_current_thread()
        .build_local(&LocalOptions::default())
        .expect("runtime");
    runtime.block_on(async {
        rusty_tokio::time::timeout(PATIENCE, async {
            let tcp = TcpStream::connect(addr).await.ok()?;
            let mut tls =
                AsyncTlsStream::new(tcp, "localhost", &TrustPolicy::DangerNoVerification).ok()?;
            tls.write_all(&TOKEN).await.ok()?;
            let hello = Message::Hello {
                version: None,
                features: Features::CHECKPOINT_V6,
                resume: None,
                takeover: None,
            };
            let mut revoke = Some(revoke);
            if early {
                // Let the server read the token and wait for Hello.
                rusty_tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                (revoke.take()?)();
            }
            tls.write_all(&encode(&hello).expect("encode")).await.ok()?;
            loop {
                let mut chunk = [0u8; 4096];
                match tls.read(&mut chunk).await {
                    Ok(0) | Err(_) => return Some(()),
                    Ok(_) => {
                        if let Some(revoke) = revoke.take() {
                            revoke();
                        }
                    }
                }
            }
        })
        .await
        .is_ok()
    })
}

impl Server {
    /// Empties the token file and reloads, as `token revoke` and SIGHUP do.
    fn revoke_all(&self, dir: &Path) {
        write(dir, "tokens", "");
        let (answer, answered) = oneshot::channel();
        self.reload.send(answer).expect("running");
        // The answer is on a channel only an async task can await; give the
        // reload a moment instead.
        drop(answered);
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    fn finish(self, dir: &Path) {
        let _ = self.stop.send(());
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_listed_token_is_served_and_a_wrong_or_missing_one_is_closed() {
    let dir = scratch();
    let server = start(&dir, "token:laptop\n");
    assert!(matches!(
        try_in(server.addr, &TOKEN),
        Some(Message::Welcome { .. })
    ));
    assert!(
        try_in(server.addr, &[8; token::LEN]).is_none(),
        "a wrong token was served"
    );
    assert!(try_in(server.addr, &[]).is_none(), "no token was served");
    server.finish(&dir);
}

#[test]
fn a_token_the_allowlist_does_not_name_is_closed() {
    let dir = scratch();
    let server = start(&dir, "# nobody\n");
    assert!(
        try_in(server.addr, &TOKEN).is_none(),
        "an unlisted token was served"
    );
    server.finish(&dir);
}

#[test]
fn a_reload_that_revokes_a_token_stops_it_being_served() {
    let dir = scratch();
    let server = start(&dir, "token:laptop\n");
    assert!(try_in(server.addr, &TOKEN).is_some());
    write(&dir, "tokens", "");
    let (answer, answered) = oneshot::channel();
    server.reload.send(answer).expect("running");
    Builder::new_current_thread()
        .build_local(&LocalOptions::default())
        .expect("runtime")
        .block_on(async { rusty_tokio::time::timeout(PATIENCE, answered).await })
        .expect("answered in time")
        .expect("answered")
        .expect("the tokens took");
    assert!(
        try_in(server.addr, &TOKEN).is_none(),
        "a revoked token was served"
    );
    server.finish(&dir);
}

#[test]
fn a_token_revoked_while_the_peer_waits_at_hello_is_not_served() {
    let dir = scratch();
    let server = start(&dir, "token:laptop\n");
    let closed = closed_after_revoke(server.addr, true, || server.revoke_all(&dir));
    assert!(closed, "a token revoked at Hello kept its seat");
    server.finish(&dir);
}

#[test]
fn a_reload_closes_a_served_connection_of_a_revoked_token() {
    let dir = scratch();
    let server = start(&dir, "token:laptop\n");
    let closed = closed_after_revoke(server.addr, false, || server.revoke_all(&dir));
    assert!(closed, "a revoked connection stayed open");
    server.finish(&dir);
}
