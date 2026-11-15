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
    write_private(&dir.join("keys"), keys);
    std::fs::write(dir.join("policy"), "* -> *\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ts-daemon"))
        .args(["hub", "--listen", "127.0.0.1:0", "--state-dir"])
        .arg(dir.join("state"))
        .arg("--keys")
        .arg(dir.join("keys"))
        .arg("--policy")
        .arg(dir.join("policy"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    // The log shares stdout, so keep draining it after the address line.
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(addr) = line.strip_prefix("ts-daemon hub: listening on ") {
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
