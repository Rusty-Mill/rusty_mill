//! The tailscale status event (C27): a server with a tailnet listener asks
//! `tailscaled` where its node stands and tells the clients that asked for
//! the event, when it changes.

#![cfg(target_os = "linux")]

mod common;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use common::{Client, PATIENCE, bridge};
use rusty_mux_core::{Event, TailscaleState};
use rusty_mux_proto::{Features, Message};
use rusty_mux_server::{Config, Reload, Sharing, Tailnet, serve_shared, tailnet::Whois};
use rusty_tokio::io::UnixListener;
use rusty_tokio::sync::{mpsc, oneshot};
use rusty_tokio::{Builder, LocalOptions};

const WATCHER: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::TAILSCALE_STATUS);
const PLAIN: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT);

fn scratch() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rusty-mux-tsstatus-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).expect("scratch");
    dir
}

/// What the fake `tailscaled` says of every peer.
const CAROL: &str =
    r#"{"Node":{"Name":"lap.t.ts.net."},"UserProfile":{"LoginName":"carol@example.com"}}"#;

/// A `tailscaled` that names every peer carol and answers every status request with the body in
/// `body`, once there is one: the test says what the node is doing, and
/// when.
fn fake_tailscaled(dir: &Path, body: Arc<Mutex<Option<String>>>) -> PathBuf {
    let path = dir.join("tailscaled.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).expect("tailscaled");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (mut stream, body) = (stream, body.clone());
            std::thread::spawn(move || {
                let mut seen = Vec::new();
                let mut buf = [0u8; 1024];
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => seen.extend_from_slice(&buf[..n]),
                    }
                }
                let body = if seen.starts_with(b"GET /localapi/v0/whois") {
                    CAROL.to_owned()
                } else {
                    loop {
                        if let Some(body) = body.lock().expect("lock").clone() {
                            break body;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            });
        }
    });
    path
}

/// A shared server on `socket` with a tailnet listener (loopback: a test
/// cannot bind a tailnet address) asking the `tailscaled` at `whois`.
fn start(dir: &Path, whois: PathBuf) -> (PathBuf, std::net::SocketAddr, oneshot::Sender<()>) {
    let socket = dir.join("shared.sock");
    let allow = dir.join("allow");
    std::fs::write(&allow, "ts:carol@example.com\n").expect("allow");
    std::fs::set_permissions(&allow, std::os::unix::fs::PermissionsExt::from_mode(0o644))
        .expect("chmod");
    let (stop, stopped) = oneshot::channel::<()>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::net::SocketAddr>();
    let bind = socket.clone();
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime");
        let _ = runtime.block_on(async move {
            let listener = UnixListener::bind(&bind).expect("bind");
            let tcp = rusty_tokio::io::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("tailnet");
            let addr = tcp.local_addr().expect("address");
            let tailnet = Tailnet::unchecked(tcp, Whois::new(whois))
                .status_every(std::time::Duration::from_millis(50));
            ready_tx.send(addr).expect("ready");
            let (_reload, reloads) = mpsc::unbounded_channel::<Reload>();
            serve_shared(
                listener,
                Config {
                    program: "/bin/sh".into(),
                    args: Vec::new(),
                    cwd: "/".into(),
                    console_host: rusty_mux_server::ConsoleHost::default(),
                    state_dir: None,
                    grapheme_clustering: false,
                    config_file: None,
                },
                Sharing {
                    allow,
                    tailnet: Some(tailnet),
                },
                reloads,
                async {
                    let _ = stopped.await;
                },
            )
            .await
        });
    });
    let addr = ready_rx.recv_timeout(PATIENCE).expect("bound");
    (socket, addr, stop)
}

fn status(state: &str, name: &str) -> String {
    format!(
        r#"{{"BackendState":"{state}","TailscaleIPs":["100.64.0.1"],"Self":{{"DNSName":"{name}"}}}}"#
    )
}

/// The next status event `client` sees: its state and node name.
fn next_status(client: &mut Client) -> (TailscaleState, Option<String>) {
    loop {
        let seen = client.events.len();
        client.recv();
        for (_, event) in &client.events[seen..] {
            if let Event::TailscaleStatusChanged { state, node } = event {
                return (*state, node.clone());
            }
        }
    }
}

#[test]
fn a_change_of_the_nodes_state_reaches_the_tailnet_clients_that_asked() {
    let dir = scratch();
    // The node is running when the peers arrive: the whois lookup needs
    // its status for the node's own addresses.
    let body = Arc::new(Mutex::new(Some(status("Running", "box.t.ts.net."))));
    let (_, addr, stop) = start(&dir, fake_tailscaled(&dir, body.clone()));
    let door = bridge(&dir, addr);
    let mut watcher = Client::connect_with(&door, WATCHER, None);
    let mut plain = Client::connect_with(&door, PLAIN, None);
    assert!(watcher.features.contains(Features::TAILSCALE_STATUS));
    assert!(!plain.features.contains(Features::TAILSCALE_STATUS));

    // A change is an event, with the node's name sans its dot; a state that
    // stays the same is not another.
    *body.lock().unwrap() = Some(status("NeedsLogin", "box.t.ts.net."));
    assert_eq!(
        next_status(&mut watcher),
        (TailscaleState::NeedsLogin, Some("box.t.ts.net".into()))
    );
    std::thread::sleep(std::time::Duration::from_millis(300));
    *body.lock().unwrap() = Some(status("Stopped", "box.t.ts.net."));
    assert_eq!(
        next_status(&mut watcher),
        (TailscaleState::Stopped, Some("box.t.ts.net".into()))
    );
    let events = watcher
        .events
        .iter()
        .filter(|(_, e)| matches!(e, Event::TailscaleStatusChanged { .. }))
        .count();
    assert_eq!(events, 2, "an unchanged state was sent again");

    // A client without the feature never sees the kind.
    let listed = plain.answer(|req| Message::List { req });
    assert!(!listed.iter().any(|m| matches!(m, Message::Failed { .. })));
    assert!(
        !plain
            .events
            .iter()
            .any(|(_, e)| matches!(e, Event::TailscaleStatusChanged { .. })),
        "a client without the feature got the event"
    );
    let _ = stop.send(());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A local connection with the feature is not told where the tailnet node
/// stands (D265), whatever the node does.
#[test]
fn a_local_client_is_not_told_where_the_node_stands() {
    let dir = scratch();
    let body = Arc::new(Mutex::new(None));
    let (socket, _, stop) = start(&dir, fake_tailscaled(&dir, body.clone()));
    let mut watcher = Client::connect_with(&socket, WATCHER, None);
    assert!(watcher.features.contains(Features::TAILSCALE_STATUS));
    // Each state is polled many times over the wait (every 50 ms).
    for state in ["Running", "NeedsLogin"] {
        *body.lock().unwrap() = Some(status(state, "box.t.ts.net."));
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    let listed = watcher.answer(|req| Message::List { req });
    assert!(!listed.iter().any(|m| matches!(m, Message::Failed { .. })));
    assert!(
        !watcher
            .events
            .iter()
            .any(|(_, e)| matches!(e, Event::TailscaleStatusChanged { .. })),
        "a local client got the event"
    );
    let _ = stop.send(());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_backend_state_this_does_not_name_is_other() {
    for (text, state) in [
        ("Running", TailscaleState::Running),
        ("NeedsLogin", TailscaleState::NeedsLogin),
        ("NeedsMachineAuth", TailscaleState::NeedsLogin),
        ("Stopped", TailscaleState::Stopped),
        ("Starting", TailscaleState::Other),
        ("NoState", TailscaleState::Other),
        ("SomethingNew", TailscaleState::Other),
    ] {
        assert_eq!(TailscaleState::from_backend(text), state, "{text}");
    }
}
