//! The launcher end to end: the real `ts-daemon hub` binary, two nodes
//! joined by pre-auth key, and a transfer through its DERP relay that must
//! arrive byte for byte.
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use ts_control::ControlClient;
use ts_derp::DerpClient;
use ts_key::{MachinePrivate, NodePrivate};
use ts_types::tailcfg::Hostinfo;

/// A running hub and the directory that holds its files.
struct Running {
    child: Child,
    dir: PathBuf,
    url: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ts-hub-it-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_private(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn start(dir: PathBuf, keys: &str) -> Running {
    start_with(dir, keys, &[])
}

/// Like [`start`], with more flags for the hub.
fn start_with(dir: PathBuf, keys: &str, extra: &[&std::ffi::OsStr]) -> Running {
    write_private(&dir.join("keys"), keys);
    std::fs::write(dir.join("policy"), "* -> *\n").unwrap();
    let mut args: Vec<std::ffi::OsString> = ["hub", "--listen", "127.0.0.1:0", "--state-dir"]
        .map(Into::into)
        .into();
    args.push(dir.join("state").into());
    args.extend(["--keys".into(), dir.join("keys").into()]);
    args.extend(["--policy".into(), dir.join("policy").into()]);
    args.extend(extra.iter().map(Into::into));
    spawn_hub(dir, &args)
}

/// Runs `ts-daemon args...` and waits for the hub to report its address.
fn spawn_hub(dir: PathBuf, args: &[std::ffi::OsString]) -> Running {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ts-daemon"))
        .args(args)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    // The log shares stdout, so keep draining it after the address line.
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(addr) = line.strip_prefix("hub: listening on ") {
                tx.send(addr.to_string()).ok();
            }
        }
    });
    let addr = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the hub reports its address");
    Running {
        child,
        dir,
        url: format!("http://{addr}"),
    }
}

async fn join(url: &str, name: &str, key: &str) -> Result<NodePrivate, String> {
    let hostinfo = Hostinfo {
        hostname: name.into(),
        os: "linux".into(),
        ..Default::default()
    };
    let client = ControlClient::connect(url, MachinePrivate::generate(), hostinfo)
        .await
        .map_err(|e| e.to_string())?;
    let node = NodePrivate::generate();
    client
        .register(node.public(), key)
        .await
        .map_err(|e| e.to_string())?;
    Ok(node)
}

#[rusty_tokio::test]
async fn two_nodes_join_and_move_a_megabyte_through_the_launched_hub() {
    let hub = start(temp_dir("transfer"), "ka alice reusable\n");
    let (ka, kb) = (
        join(&hub.url, "a", "ka").await.unwrap(),
        join(&hub.url, "b", "ka").await.unwrap(),
    );
    let a = DerpClient::connect(&hub.url, &ka).await.unwrap();
    let mut b = DerpClient::connect(&hub.url, &kb).await.unwrap();

    let data: Vec<u8> = (0..1u32 << 20)
        .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
        .collect();
    let (sender, to, outgoing) = (a.sender(), kb.public(), data.clone());
    rusty_tokio::spawn(async move {
        for chunk in outgoing.chunks(16 << 10) {
            sender.send(to, chunk.to_vec()).await.unwrap();
        }
    });
    let mut got = Vec::with_capacity(data.len());
    rusty_tokio::time::timeout(Duration::from_secs(30), async {
        while got.len() < data.len() {
            got.extend_from_slice(&b.recv().await.expect("relay stays up").payload);
        }
    })
    .await
    .expect("the whole megabyte arrives");
    assert!(got == data, "bytes differ");
}

#[rusty_tokio::test]
async fn sighup_adds_and_drops_keys_without_a_restart() {
    let hub = start(temp_dir("reload"), "k1 alice reusable\n");
    join(&hub.url, "a", "k1").await.unwrap();
    assert!(
        join(&hub.url, "b", "k2").await.is_err(),
        "k2 is not listed yet"
    );

    write_private(&hub.dir.join("keys"), "k2 bob reusable\n");
    let pid = hub.child.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-HUP", &pid])
            .status()
            .unwrap()
            .success()
    );

    let mut joined = false;
    for _ in 0..50 {
        if join(&hub.url, "b", "k2").await.is_ok() {
            joined = true;
            break;
        }
        rusty_tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(joined, "k2 works after SIGHUP");
    assert!(join(&hub.url, "c", "k1").await.is_err(), "k1 was dropped");
}

#[test]
fn a_keys_file_others_can_read_stops_the_hub_before_it_serves() {
    let dir = temp_dir("perm");
    std::fs::write(dir.join("keys"), "k alice\n").unwrap();
    std::fs::set_permissions(dir.join("keys"), std::fs::Permissions::from_mode(0o644)).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ts-daemon"))
        .args(["hub", "--listen", "127.0.0.1:0", "--state-dir"])
        .arg(dir.join("state"))
        .arg("--keys")
        .arg(dir.join("keys"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("listening"));
}

#[test]
fn a_bad_certificate_stops_the_hub_before_it_serves() {
    let dir = temp_dir("tls");
    std::fs::write(dir.join("cert"), "not a certificate\n").unwrap();
    std::fs::write(dir.join("key"), "not a key\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ts-daemon"))
        .args(["hub", "--listen", "127.0.0.1:0", "--state-dir"])
        .arg(dir.join("state"))
        .arg("--tls-cert")
        .arg(dir.join("cert"))
        .arg("--tls-key")
        .arg(dir.join("key"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("listening"));
}

#[test]
fn a_tls_ca_file_without_a_certificate_stops_the_daemon_before_it_dials() {
    let dir = temp_dir("ca");
    std::fs::write(dir.join("ca.pem"), "not a certificate\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ts-daemon"))
        .args([
            "--login-server",
            "https://localhost:1",
            "--authkey",
            "k",
            "--state-dir",
        ])
        .arg(dir.join("state"))
        .arg("--tls-ca")
        .arg(dir.join("ca.pem"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--tls-ca"));
}

#[test]
fn a_tls_ca_file_with_plain_http_urls_stops_the_daemon_before_it_dials() {
    let dir = temp_dir("cahttp");
    std::fs::write(dir.join("ca.pem"), "not read\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ts-daemon"))
        .args([
            "--login-server",
            "http://localhost:1",
            "--authkey",
            "k",
            "--state-dir",
        ])
        .arg(dir.join("state"))
        .arg("--tls-ca")
        .arg(dir.join("ca.pem"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("https://"));
}

#[rusty_tokio::test]
async fn a_keys_file_that_does_not_parse_leaves_the_old_keys_working_after_sighup() {
    let hub = start(temp_dir("badreload"), "k1 alice reusable\n");
    write_private(&hub.dir.join("keys"), "k2 bob reusable\nbroken\n");
    let pid = hub.child.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-HUP", &pid])
            .status()
            .unwrap()
            .success()
    );
    rusty_tokio::time::sleep(Duration::from_millis(500)).await;
    join(&hub.url, "a", "k1").await.unwrap();
    assert!(join(&hub.url, "b", "k2").await.is_err(), "k2 was applied");
}

async fn control_key(url: &str) -> ts_types::MachinePublic {
    let hostinfo = Hostinfo::default();
    ControlClient::connect(url, MachinePrivate::generate(), hostinfo)
        .await
        .unwrap()
        .control_key()
}

#[rusty_tokio::test]
async fn a_standby_promoted_after_the_active_is_killed_serves_the_same_hub() {
    use std::ffi::OsStr;
    let dir = temp_dir("failover");
    let secret = dir.join("secret");
    write_private(&secret, "0123456789abcdef0123456789abcdef\n");
    std::fs::create_dir_all(dir.join("active")).unwrap();
    let mut active = start_with(
        dir.join("active"),
        "ka alice reusable\n",
        &[OsStr::new("--replica-secret"), secret.as_os_str()],
    );
    let active_key = control_key(&active.url).await;
    join(&active.url, "a", "ka").await.unwrap();

    let standby_state = dir.join("standby");
    let mut standby = Command::new(env!("CARGO_BIN_EXE_ts-daemon"))
        .args(["hub", "--standby", &active.url, "--replica-secret"])
        .arg(&secret)
        .arg("--state-dir")
        .arg(&standby_state)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let copy = standby_state.join("nodes.json");
    let mut copied = false;
    for _ in 0..100 {
        let text = std::fs::read_to_string(&copy).unwrap_or_default();
        if text.contains("\"hostname\": \"a\"") {
            copied = true;
            break;
        }
        rusty_tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(copied, "the standby copies the node table");

    // Promotion is refused while the active hub answers.
    let refused = Command::new(env!("CARGO_BIN_EXE_ts-daemon"))
        .args([
            "hub",
            "--listen",
            "127.0.0.1:0",
            "--promote",
            &active.url,
            "--state-dir",
        ])
        .arg(&standby_state)
        .output()
        .unwrap();
    assert!(!refused.status.success());

    // Kill both, then promote the copy.
    active.child.kill().unwrap();
    active.child.wait().unwrap();
    standby.kill().unwrap();
    standby.wait().unwrap();
    std::fs::copy(active.dir.join("keys"), dir.join("keys")).unwrap();
    std::fs::copy(active.dir.join("policy"), dir.join("policy")).unwrap();
    let promoted = Command::new(env!("CARGO_BIN_EXE_ts-daemon"))
        .args([
            "hub",
            "--listen",
            "127.0.0.1:0",
            "--promote",
            &active.url,
            "--state-dir",
        ])
        .arg(&standby_state)
        .arg("--keys")
        .arg(dir.join("keys"))
        .arg("--policy")
        .arg(dir.join("policy"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut promoted = Running {
        child: promoted,
        dir: dir.clone(),
        url: String::new(),
    };
    let stdout = promoted.child.stdout.take().unwrap();
    let addr = BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .find_map(|l| l.strip_prefix("hub: listening on ").map(str::to_string))
        .expect("the promoted hub reports its address");
    promoted.url = format!("http://{addr}");

    assert_eq!(
        control_key(&promoted.url).await,
        active_key,
        "nodes pin this key"
    );
    join(&promoted.url, "c", "ka")
        .await
        .expect("a new node joins the promoted hub");
}

/// Two real nodes stream through the active hub; it is killed mid-transfer and
/// the standby is promoted on the address the nodes were told about. The
/// stream must finish byte for byte, and a new node must join afterwards.
#[rusty_tokio::test]
async fn nodes_ride_out_the_active_hub_dying_and_a_standby_taking_over() {
    use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt};
    use std::ffi::{OsStr, OsString};

    const PORT: u16 = 7000;
    const LEN: usize = 2 << 20;
    let dir = temp_dir("takeover");
    let secret = dir.join("secret");
    write_private(&secret, "0123456789abcdef0123456789abcdef\n");
    let standby_addr = {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        probe.local_addr().unwrap()
    };
    let standby_url = format!("http://{standby_addr}");

    std::fs::create_dir_all(dir.join("active")).unwrap();
    let mut active = start_with(
        dir.join("active"),
        "ka alice reusable\n",
        &[
            OsStr::new("--replica-secret"),
            secret.as_os_str(),
            OsStr::new("--advertise-standby"),
            OsStr::new(&standby_url),
        ],
    );
    let node = |name: &str, url: &str, key: &str| {
        let cfg = ts_net::NodeConfig {
            control_url: url.to_owned(),
            derp_url: None,
            authkey: key.into(),
            hostname: name.to_owned(),
            state_dir: dir.join(name),
            enable_direct: false,
            stun_server: None,
            trust: ts_net::Trust::System,
        };
        async move { ts_net::Node::new(cfg).await.expect("node joins") }
    };
    let a = node("a", &active.url, "ka").await;
    let b = node("b", &active.url, "ka").await;
    let a_ip = a.wait_ip().await.expect("a has an address");
    b.wait_ip().await.expect("b has an address");
    let mut listener = a.bind(PORT).await.expect("bind");

    let want: Vec<u8> = (0..LEN)
        .map(|i| (i.wrapping_mul(31) ^ (i >> 8)) as u8)
        .collect();
    let sender = rusty_tokio::spawn({
        let want = want.clone();
        async move {
            let mut s = b.connect((a_ip, PORT).into()).await.expect("connect");
            for chunk in want.chunks(32 << 10) {
                s.write_all(chunk).await.expect("write");
                rusty_tokio::time::sleep(Duration::from_millis(20)).await;
            }
            s.shutdown().await.expect("shutdown");
            (s, b)
        }
    });

    let standby_state = dir.join("standby");
    let standby = Command::new(env!("CARGO_BIN_EXE_ts-daemon"))
        .args(["hub", "--standby", &active.url, "--replica-secret"])
        .arg(&secret)
        .arg("--state-dir")
        .arg(&standby_state)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut standby = Running {
        child: standby,
        dir: dir.clone(),
        url: String::new(),
    };

    let mut conn = listener.accept().await.expect("accept");
    let mut got = Vec::with_capacity(LEN);
    let mut buf = vec![0u8; 64 << 10];
    let mut promoted = None;
    let finished = rusty_tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let n = conn.read(&mut buf).await.expect("read");
            if n == 0 {
                return;
            }
            got.extend_from_slice(&buf[..n]);
            if promoted.is_none() && got.len() > LEN / 4 {
                // The copy must have arrived before the active dies.
                let copy = standby_state.join("nodes.json");
                assert!(
                    std::fs::read_to_string(&copy).is_ok_and(|t| t.contains("\"hostname\": \"b\"")),
                    "the standby has the node table"
                );
                active.child.kill().unwrap();
                active.child.wait().unwrap();
                standby.child.kill().unwrap();
                standby.child.wait().unwrap();
                // The key the nodes joined with is gone from the standby's list
                // (rotated out): they are known machines and must not need it.
                write_private(&dir.join("keys"), "kz alice reusable\n");
                std::fs::copy(active.dir.join("policy"), dir.join("policy")).unwrap();
                let args: Vec<OsString> = [
                    "hub".into(),
                    "--listen".into(),
                    standby_addr.to_string().into(),
                    "--promote".into(),
                    active.url.clone().into(),
                    "--state-dir".into(),
                    standby_state.clone().into(),
                    "--keys".into(),
                    dir.join("keys").into(),
                    "--policy".into(),
                    dir.join("policy").into(),
                ]
                .into();
                promoted = Some(spawn_hub(dir.clone(), &args));
            }
        }
    })
    .await;
    assert!(finished.is_ok(), "the stream finishes after the takeover");
    assert_eq!(got.len(), want.len(), "length");
    assert!(got == want, "bytes differ");
    let _kept = sender.await; // keeps b's node alive

    let c = node("c", &standby_url, "kz").await;
    let c_ip = c
        .wait_ip()
        .await
        .expect("a new node joins the promoted hub");

    // The old nodes hear of it only if their control link followed the
    // promotion (the stream above rode on what they already knew).
    let mut listener = c.bind(PORT + 1).await.expect("bind on c");
    let reached = rusty_tokio::time::timeout(Duration::from_secs(30), async {
        let mut s = a.connect((c_ip, PORT + 1).into()).await.expect("connect");
        s.write_all(b"hello").await.expect("write");
        let mut conn = listener.accept().await.expect("accept");
        let mut buf = [0u8; 5];
        conn.read_exact(&mut buf).await.expect("read");
        buf
    })
    .await
    .expect("an old node learns of the new one through the promoted hub");
    assert_eq!(&reached, b"hello");
}

/// A different hub on an advertised standby address is not the hub the node
/// joined: it must never see the node's pre-auth key.
#[rusty_tokio::test]
async fn a_node_will_not_follow_a_hub_with_a_different_key() {
    use std::ffi::OsStr;

    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let other_addr = probe.local_addr().unwrap();
    drop(probe);
    let other_url = format!("http://{other_addr}");

    let mut active = start_with(
        temp_dir("imposter-active"),
        "ka alice reusable\n",
        &[OsStr::new("--advertise-standby"), OsStr::new(&other_url)],
    );
    let dir = temp_dir("imposter-node");
    let node = ts_net::Node::new(ts_net::NodeConfig {
        control_url: active.url.clone(),
        derp_url: None,
        authkey: "ka".into(),
        hostname: "wanderer".into(),
        state_dir: dir.join("node"),
        enable_direct: false,
        stun_server: None,
        trust: ts_net::Trust::System,
    })
    .await
    .expect("node joins");
    node.wait_ip().await.expect("address");

    // A stranger: its own key, same pre-auth key, on the advertised address.
    let args: Vec<std::ffi::OsString> = [
        "hub".into(),
        "--listen".into(),
        other_addr.to_string().into(),
        "--state-dir".into(),
        dir.join("other").into(),
        "--keys".into(),
        dir.join("keys").into(),
    ]
    .into();
    write_private(&dir.join("keys"), "ka mallory reusable\n");
    let _other = spawn_hub(dir.clone(), &args);
    active.child.kill().unwrap();
    active.child.wait().unwrap();

    rusty_tokio::time::sleep(Duration::from_secs(5)).await;
    let table = std::fs::read_to_string(dir.join("other").join("nodes.json")).unwrap_or_default();
    assert!(
        !table.contains("wanderer"),
        "the node registered with a hub it never joined: {table}"
    );
}

/// Rebind a dead hub's address even when its accepted connections are in
/// TIME_WAIT. The hub also sets SO_REUSEADDR; no live listener is shared.
fn retry_probe(addr: &str) -> std::net::TcpListener {
    let socket = rusty_tokio::io::TcpSocket::new_v4().expect("probe socket");
    socket
        .set_reuseaddr(true)
        .expect("reuse the dead hub's address");
    socket
        .bind(addr.parse().expect("hub address"))
        .expect("bind probe");
    let probe = socket
        .listen(128)
        .expect("listen")
        .into_std()
        .expect("probe listener");
    probe.set_nonblocking(true).expect("nonblocking probe");
    probe
}

#[rusty_tokio::test]
async fn retry_probe_rebinds_after_the_server_closes_an_accepted_connection() {
    use std::io::Read;

    let listener = rusty_tokio::io::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener")
        .into_std()
        .expect("std listener");
    let addr = listener.local_addr().unwrap();
    let mut client = std::net::TcpStream::connect(addr).expect("connect");
    let (mut server, _) = listener.accept().expect("accept");
    // Complete both halves of a server-first close before rebinding, so
    // the server's local address is in TIME_WAIT rather than racing FIN.
    server
        .shutdown(std::net::Shutdown::Write)
        .expect("server FIN");
    assert_eq!(client.read(&mut [0]).expect("server EOF"), 0);
    client
        .shutdown(std::net::Shutdown::Write)
        .expect("client FIN");
    assert_eq!(server.read(&mut [0]).expect("client EOF"), 0);
    drop(server);
    drop(client);
    drop(listener);
    let probe = retry_probe(&addr.to_string());
    assert_eq!(probe.local_addr().unwrap(), addr);
    assert_eq!(
        probe.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

/// A dropped node must not leave retry loops behind when its hub dies.
#[rusty_tokio::test]
async fn a_dropped_node_stops_retrying_when_its_hub_dies() {
    let mut hub = start(temp_dir("dropped"), "ka alice reusable\n");
    let addr = hub.url.trim_start_matches("http://").to_owned();
    let node = ts_net::Node::new(ts_net::NodeConfig {
        control_url: hub.url.clone(),
        derp_url: None,
        authkey: "ka".into(),
        hostname: "leaver".into(),
        state_dir: hub.dir.join("node"),
        enable_direct: false,
        stun_server: None,
        trust: ts_net::Trust::System,
    })
    .await
    .expect("node joins");
    node.wait_ip().await.expect("address");
    drop(node);
    hub.child.kill().unwrap();
    hub.child.wait().unwrap();

    // Whatever still dials the old address shows up here.
    let probe = retry_probe(&addr);
    rusty_tokio::time::sleep(Duration::from_secs(8)).await;
    assert!(
        probe.accept().is_err(),
        "something still retries the dead hub"
    );
}

/// The same, when the hub dies first and the last handle goes while the node
/// is already retrying.
#[rusty_tokio::test]
async fn a_node_dropped_while_retrying_stops_retrying() {
    let mut hub = start(temp_dir("dropped-late"), "ka alice reusable\n");
    let addr = hub.url.trim_start_matches("http://").to_owned();
    let node = ts_net::Node::new(ts_net::NodeConfig {
        control_url: hub.url.clone(),
        derp_url: None,
        authkey: "ka".into(),
        hostname: "latecomer".into(),
        state_dir: hub.dir.join("node"),
        enable_direct: false,
        stun_server: None,
        trust: ts_net::Trust::System,
    })
    .await
    .expect("node joins");
    node.wait_ip().await.expect("address");
    hub.child.kill().unwrap();
    hub.child.wait().unwrap();
    // Let it notice and start retrying against the dead address.
    rusty_tokio::time::sleep(Duration::from_secs(2)).await;
    drop(node);

    let probe = retry_probe(&addr);
    rusty_tokio::time::sleep(Duration::from_secs(8)).await;
    assert!(
        probe.accept().is_err(),
        "something still retries the dead hub"
    );
}
