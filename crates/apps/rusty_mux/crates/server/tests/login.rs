//! Login mode (C26, Rex delta D9): the front door maps each peer to an
//! account through the map file, relays it to that account's own server
//! (started on first use) with the principal it verified, and a reload
//! closes only the connections it revokes.
//!
//! The front door runs as the test's own user, so it serves only that
//! user's account; `a_root_front_door_starts_the_server_as_the_account`
//! needs root and an account named `claude`, and is ignored elsewhere.

#![cfg(target_os = "linux")]

mod common;

use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{Client, PATIENCE, shell};
use rusty_mux_core::SessionId;
use rusty_mux_proto::{Entry, Features, Message};
use rusty_mux_server::{Login, Reload, serve_forwarded, serve_login};
use rusty_tokio::io::UnixListener;
use rusty_tokio::sync::{mpsc, oneshot};
use rusty_tokio::{Builder, LocalOptions};

const IDENTITY: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::LABELS)
    .union(Features::IDENTITY);

fn me() -> (u32, String) {
    let uid = rusty_libc::process::geteuid();
    (
        uid,
        platform_linux::user_name(uid).expect("this user has a name"),
    )
}

/// A fresh scratch directory: the map, the sockets and the runtime.
fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rusty-mux-login-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).expect("scratch");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    dir
}

fn write_map(dir: &Path, text: &str) -> PathBuf {
    let path = dir.join("accounts");
    std::fs::write(&path, text).expect("map");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    path
}

/// A running front door: its socket, a way to reload it, and its stop.
struct Front {
    socket: PathBuf,
    reload: mpsc::UnboundedSender<Reload>,
    stop: Option<oneshot::Sender<()>>,
    ended: std::sync::mpsc::Receiver<std::io::Result<()>>,
}

impl Front {
    fn start(dir: &Path, name: &str) -> Self {
        let socket = dir.join(name);
        let login = Login {
            map: dir.join("accounts"),
            runtime: dir.join("run"),
            server: PathBuf::from(env!("CARGO_BIN_EXE_rusty-mux-server")),
        };
        let (reload, reloads) = mpsc::unbounded_channel();
        let (stop, stopped) = oneshot::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (ended_tx, ended) = std::sync::mpsc::channel();
        let bind = socket.clone();
        std::thread::spawn(move || {
            let runtime = Builder::new_current_thread()
                .build_local(&LocalOptions::default())
                .expect("runtime");
            let result = runtime.block_on(async move {
                let listener = UnixListener::bind(&bind).expect("bind");
                ready_tx.send(()).expect("ready");
                rusty_tokio::select! {
                    result = serve_login(listener, login, reloads) => result,
                    _ = stopped => Ok(()),
                }
            });
            let _ = ended_tx.send(result);
        });
        ready_rx.recv_timeout(PATIENCE).expect("front door bound");
        Self {
            socket,
            reload,
            stop: Some(stop),
            ended,
        }
    }

    /// Reads the map again; whether it took.
    fn reload(&self) -> std::io::Result<()> {
        let (answer, answered) = oneshot::channel();
        self.reload.send(answer).expect("front door running");
        Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime")
            .block_on(async { rusty_tokio::time::timeout(PATIENCE, answered).await })
            .expect("reload answered in time")
            .expect("reload answered")
    }

    fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.ended
            .recv_timeout(PATIENCE)
            .expect("front door stopped")
            .expect("front door ended cleanly");
    }
}

/// Ends every account server started under `dir`: they outlive the front
/// door by design, so each test stops its own.
struct Reap(PathBuf);

impl Drop for Reap {
    fn drop(&mut self) {
        let needle = self.0.join("run").into_os_string().into_encoded_bytes();
        let Ok(procs) = std::fs::read_dir("/proc") else {
            return;
        };
        for entry in procs.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<i32>().ok())
            else {
                continue;
            };
            let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
            if cmdline
                .windows(needle.len())
                .any(|w| w == needle.as_slice())
            {
                let _ = rusty_libc::process::kill(pid, rusty_libc::signal::SIGKILL);
            }
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn whoami(client: &mut Client) -> (String, String) {
    let answer = client.answer(|req| Message::Whoami { req });
    match answer.as_slice() {
        [
            Message::Entry {
                entry: Entry::Client {
                    principal: Some(p), ..
                },
                ..
            },
            ..,
        ] => (p.id().to_owned(), p.user().to_owned()),
        other => panic!("expected the caller's entry, got {other:?}"),
    }
}

fn sessions(client: &mut Client) -> Vec<SessionId> {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .filter_map(|m| match m {
            Message::Entry {
                entry: Entry::Session { session, .. },
                ..
            } => Some(session),
            _ => None,
        })
        .collect()
}

/// Connects and says Hello, and reports whether the connection was closed
/// without a Welcome.
fn closed_unheard(socket: &Path) -> bool {
    let mut stream = UnixStream::connect(socket).expect("connect");
    stream.set_read_timeout(Some(PATIENCE)).expect("timeout");
    let hello = Message::Hello {
        features: IDENTITY,
        resume: None,
        takeover: None,
    };
    // The front door may have closed already, before reading it.
    let _ = stream.write_all(&rusty_mux_proto::encode(&hello).expect("encode"));
    ended(&mut stream)
}

/// Whether the peer closed `stream`: end of stream, or a reset when it
/// closed with our bytes unread.
fn ended(stream: &mut UnixStream) -> bool {
    match stream.read(&mut [0u8; 64]) {
        Ok(0) => true,
        Err(e) => e.kind() == std::io::ErrorKind::ConnectionReset,
        Ok(_) => false,
    }
}

/// The front door as its own process, as an administrator runs it.
struct FrontProcess {
    socket: PathBuf,
    child: std::process::Child,
}

impl FrontProcess {
    fn start(dir: &Path, name: &str) -> Self {
        let socket = dir.join(name);
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_rusty-mux-server"))
            .arg("--login")
            .arg(dir.join("accounts"))
            .arg("--runtime")
            .arg(dir.join("run"))
            .arg(&socket)
            .spawn()
            .expect("front door");
        let deadline = std::time::Instant::now() + PATIENCE;
        while UnixStream::connect(&socket).is_err() {
            assert!(
                std::time::Instant::now() < deadline,
                "front door never listened"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Self { socket, child }
    }

    fn signal(&self, signal: i32) {
        let pid = i32::try_from(self.child.id()).expect("pid");
        rusty_libc::process::kill(pid, signal).expect("signal");
    }
}

impl Drop for FrontProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn login_mode_maps_a_peer_to_its_account_and_forwards_who_it_is() {
    let dir = scratch();
    let _reap = Reap(dir.clone());
    let (uid, user) = me();
    write_map(&dir, &format!("# only me\nuid:{uid} {user}\n"));
    let front = Front::start(&dir, "login.sock");

    let mut first = Client::connect_with(&front.socket, IDENTITY, None);
    assert_eq!(whoami(&mut first), (format!("uid:{uid}"), user.clone()));
    // The account's server is its own, in a directory only it can open.
    let account_dir = dir.join("run").join(&user);
    let meta = std::fs::symlink_metadata(&account_dir).expect("account dir");
    assert!(meta.is_dir());
    assert_eq!((meta.uid(), meta.mode() & 0o777), (uid, 0o700));
    assert!(account_dir.join("server.sock").exists());

    // A second connection reaches the same server, and its sessions.
    let session = first.request(|req| Message::CreateSession {
        req,
        name: "work".into(),
    });
    let mut second = Client::connect_with(&front.socket, IDENTITY, None);
    let listed = sessions(&mut second);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(Some(listed[0].uuid()), session);
    front.stop();
}

#[test]
fn a_map_with_no_rule_for_the_peer_lets_it_in_nowhere() {
    let dir = scratch();
    let _reap = Reap(dir.clone());
    let (uid, user) = me();
    // Empty, as shipped; then a rule for someone else; then root through `*`.
    for map in [
        "",
        &format!("uid:{} {user}\n", uid.wrapping_add(1)),
        "* root\n",
    ] {
        if map == "* root\n" && uid != 0 {
            continue;
        }
        write_map(&dir, map);
        let front = Front::start(&dir, "login.sock");
        assert!(closed_unheard(&front.socket), "{map:?} let the peer in");
        front.stop();
        let _ = std::fs::remove_file(dir.join("login.sock"));
    }
    assert!(
        !dir.join("run").join(&user).exists(),
        "a server started for nobody"
    );
}

#[test]
fn the_account_server_outlives_the_front_door() {
    let dir = scratch();
    let _reap = Reap(dir.clone());
    let (uid, user) = me();
    write_map(&dir, &format!("uid:{uid} {user}\n"));
    let front = FrontProcess::start(&dir, "login.sock");
    let mut client = Client::connect_with(&front.socket, IDENTITY, None);
    let session = client.request(|req| Message::CreateSession {
        req,
        name: "work".into(),
    });
    drop(front);
    // The relay went with the front door ...
    client.wait_closed();
    // ... but a new front door reaches the same server and session.
    let again = FrontProcess::start(&dir, "login-2.sock");
    let mut client = Client::connect_with(&again.socket, IDENTITY, None);
    let listed = sessions(&mut client);
    assert_eq!(
        listed.iter().map(|s| Some(s.uuid())).collect::<Vec<_>>(),
        vec![session]
    );
}

/// SIGHUP is the front door's reload, as `ExecReload` sends it.
#[test]
fn a_hangup_reloads_the_map() {
    let dir = scratch();
    let _reap = Reap(dir.clone());
    let (uid, user) = me();
    write_map(&dir, &format!("uid:{uid} {user}\n"));
    let front = FrontProcess::start(&dir, "login.sock");
    let mut client = Client::connect_with(&front.socket, IDENTITY, None);
    write_map(&dir, "");
    front.signal(rusty_libc::signal::SIGHUP);
    client.wait_closed();
    assert!(closed_unheard(&front.socket));
}

#[test]
fn a_reload_closes_only_the_connections_it_revokes() {
    let dir = scratch();
    let _reap = Reap(dir.clone());
    let (uid, user) = me();
    write_map(&dir, &format!("uid:{uid} {user}\n"));
    let front = Front::start(&dir, "login.sock");
    let mut client = Client::connect_with(&front.socket, IDENTITY, None);

    // The same account by another rule keeps the connection.
    write_map(&dir, &format!("user:{user} =\n"));
    front.reload().expect("a good map");
    assert_eq!(whoami(&mut client).0, format!("uid:{uid}"));

    // A bad map is refused whole, and the old one stays.
    write_map(&dir, &format!("user:{user} =\nnonsense\n"));
    assert!(front.reload().is_err());
    assert_eq!(whoami(&mut client).0, format!("uid:{uid}"));
    let path = write_map(&dir, &format!("user:{user} =\n"));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).expect("chmod");
    assert!(front.reload().is_err(), "a map anyone may write was taken");
    assert_eq!(whoami(&mut client).0, format!("uid:{uid}"));

    // A map without the peer closes it, and lets nobody new in.
    write_map(&dir, "");
    front.reload().expect("an empty map");
    client.wait_closed();
    assert!(closed_unheard(&front.socket));
    front.stop();
}

#[test]
fn a_bad_map_stops_the_front_door_before_it_serves() {
    let dir = scratch();
    let _reap = Reap(dir.clone());
    write_map(&dir, "uid:1 two words here\n");
    let front = Front::start(&dir, "login.sock");
    let ended = front.ended.recv_timeout(PATIENCE).expect("ended");
    assert!(ended.is_err(), "a bad map was served");
}

#[test]
fn a_runtime_directory_others_can_write_stops_the_front_door() {
    let dir = scratch();
    let _reap = Reap(dir.clone());
    let (uid, user) = me();
    write_map(&dir, &format!("uid:{uid} {user}\n"));
    std::fs::create_dir(dir.join("run")).expect("run");
    std::fs::set_permissions(dir.join("run"), std::fs::Permissions::from_mode(0o777))
        .expect("chmod");
    let front = Front::start(&dir, "login.sock");
    let ended = front.ended.recv_timeout(PATIENCE).expect("ended");
    assert!(ended.is_err(), "a world-writable runtime was used");
}

/// Its parent too: another user could rename `run` away and put their own
/// tree in its place, between the check and the front door's use of it.
#[test]
fn a_runtime_directory_in_a_parent_others_can_write_stops_the_front_door() {
    let dir = scratch();
    let _reap = Reap(dir.clone());
    let (uid, user) = me();
    write_map(&dir, &format!("uid:{uid} {user}\n"));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).expect("chmod");
    let front = Front::start(&dir, "login.sock");
    let ended = front.ended.recv_timeout(PATIENCE).expect("ended");
    assert!(
        ended.is_err(),
        "a runtime under a world-writable parent was used"
    );
}

/// An account server trusts a preface only from the forwarder's uid, and
/// closes a forwarder's connection whose preface is missing or bad.
#[test]
fn an_account_server_needs_the_preface_from_the_forwarder() {
    let dir = scratch();
    let _reap = Reap(dir.clone());
    let path = dir.join("server.sock");
    start_forwarded(&path, me().0);
    // A Hello where the preface belongs, and a preface of another version.
    for bytes in [
        &b"\0\0\0\x10hello, no preface"[..],
        b"RMXFWD2\0\0\x05uid:0\0\0",
    ] {
        let mut stream = UnixStream::connect(&path).expect("connect");
        stream.set_read_timeout(Some(PATIENCE)).expect("timeout");
        stream.write_all(bytes).expect("write");
        assert!(ended(&mut stream), "{bytes:?} served");
    }
}

/// An account server behind a front door of `forwarder`'s uid, on `path`.
fn start_forwarded(path: &Path, forwarder: u32) {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let bind = path.to_owned();
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime");
        runtime.block_on(async move {
            let listener = UnixListener::bind(&bind).expect("bind");
            ready_tx.send(()).expect("ready");
            serve_forwarded(listener, shell(), forwarder, std::future::pending()).await
        })
    });
    ready_rx.recv_timeout(PATIENCE).expect("bound");
}

/// Root's front door starts the account's server as the account: its uid,
/// its directory, and the principal of who connected (root here).
#[test]
#[ignore = "needs root and an account named claude"]
fn a_root_front_door_starts_the_server_as_the_account() {
    let account = platform_linux::account("claude").expect("an account named claude");
    assert_eq!(rusty_libc::process::geteuid(), 0, "run as root");
    let dir = scratch();
    let _reap = Reap(dir.clone());
    write_map(&dir, "uid:0 claude\n");
    let front = Front::start(&dir, "login.sock");
    let mut client = Client::connect_with(&front.socket, IDENTITY, None);
    assert_eq!(whoami(&mut client), ("uid:0".to_owned(), "root".to_owned()));
    let account_dir = dir.join("run").join("claude");
    let meta = std::fs::symlink_metadata(&account_dir).expect("account dir");
    assert_eq!(
        (meta.uid(), meta.gid(), meta.mode() & 0o777),
        (account.uid, account.gid, 0o700)
    );
    let (pid, server) = server_uid(&account_dir.join("server.sock")).expect("the account's server");
    assert_eq!(server, account.uid);
    // No descriptor of the front door's (here, the test's own stdio)
    // reaches the account: its first three are /dev/null.
    for fd in 0..3 {
        let target = std::fs::read_link(format!("/proc/{pid}/fd/{fd}")).expect("fd");
        assert_eq!(target, Path::new("/dev/null"), "fd {fd}");
    }
    front.stop();
}

/// The pid and real uid of the process whose command line names `socket`.
fn server_uid(socket: &Path) -> Option<(u32, u32)> {
    let needle = socket.as_os_str().as_encoded_bytes();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
        if !cmdline.windows(needle.len()).any(|w| w == needle) {
            continue;
        }
        let status = std::fs::read_to_string(entry.path().join("status")).ok()?;
        let line = status.lines().find(|l| l.starts_with("Uid:"))?;
        let pid = entry.file_name().to_str()?.parse().ok()?;
        return Some((pid, line.split_whitespace().nth(1)?.parse().ok()?));
    }
    None
}

/// The account owns its directory, so it can swap its socket for a link to
/// another user's server. Root's front door must not follow it there.
#[test]
#[ignore = "needs root and an account named claude"]
fn a_root_front_door_refuses_a_socket_the_account_does_not_serve() {
    platform_linux::account("claude").expect("an account named claude");
    assert_eq!(rusty_libc::process::geteuid(), 0, "run as root");
    let dir = scratch();
    let _reap = Reap(dir.clone());
    write_map(&dir, "uid:0 claude\n");
    // Another account's server behind the same front door, which a link
    // in claude's directory points at: it would take the preface.
    let roots = dir.join("roots.sock");
    start_forwarded(&roots, 0);
    let account_dir = dir.join("run").join("claude");
    let front = Front::start(&dir, "login.sock");
    // The first connection makes the directory and claude's server.
    drop(Client::connect_with(&front.socket, IDENTITY, None));
    std::fs::remove_file(account_dir.join("server.sock")).expect("unlink");
    std::os::unix::fs::symlink(&roots, account_dir.join("server.sock")).expect("link");
    assert!(closed_unheard(&front.socket), "relayed to root's server");
    front.stop();
}
