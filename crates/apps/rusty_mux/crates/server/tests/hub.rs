//! `rusty-mux-server --hub`: the binary starts the mesh hub instead of a mux
//! server, and `GET /key` answers with the hub's Noise key.
#![cfg(all(target_os = "linux", feature = "hub"))]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Hub(Child, std::path::PathBuf);

impl Drop for Hub {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
        std::fs::remove_dir_all(&self.1).ok();
    }
}

#[test]
fn hub_mode_serves_the_key_and_stops_on_a_bad_flag() {
    let dir = std::env::temp_dir().join(format!("mux-hub-it-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rusty-mux-server"))
        .args(["--hub", "--listen", "127.0.0.1:0", "--state-dir"])
        .arg(dir.join("state"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let hub = Hub(child, dir);
    // The log shares stdout, so keep draining it after the address line.
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

    let mut stream = TcpStream::connect(&addr).unwrap();
    stream
        .write_all(b"GET /key HTTP/1.1\r\nHost: hub\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.contains("publicKey"), "{reply}");
    drop(hub);

    let bad = Command::new(env!("CARGO_BIN_EXE_rusty-mux-server"))
        .args(["--hub", "--wat"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("rusty-mux-server --hub"));
}
