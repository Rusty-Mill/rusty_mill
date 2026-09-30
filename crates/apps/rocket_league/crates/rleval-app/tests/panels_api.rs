//! The lazy-panel API against the real binary: `/api/analyze` returns just the data
//! (the heavy HTML stays on the server), `/api/analysis/<id>/<panel>` serves each
//! panel, and `?inline=1` keeps the legacy embedded shape.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// A running `rleval serve`, killed on drop.
struct Server(Child, u16);

impl Server {
    fn start() -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let replays = format!("{}/../assets/replays", env!("CARGO_MANIFEST_DIR"));
        let child = Command::new(env!("CARGO_BIN_EXE_rleval"))
            .args(["serve", "--port", &port.to_string(), "--replays", &replays])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rleval");
        let server = Self(child, port);
        for _ in 0..100 {
            if server.get("/healthz").0 == 200 {
                return server;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("server did not come up");
    }

    /// The raw `(head, body)` bytes of a GET sent with `extra` headers.
    fn get_raw(&self, path: &str, extra: &str) -> (String, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", self.1)).unwrap();
        write!(
            s,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\n{extra}Connection: close\r\n\r\n"
        )
        .unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).unwrap();
        let at = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("header end");
        (
            String::from_utf8_lossy(&raw[..at]).into_owned(),
            raw[at + 4..].to_vec(),
        )
    }

    /// `(status, body)` of a GET.
    fn get(&self, path: &str) -> (u16, String) {
        let Ok(mut s) = TcpStream::connect(("127.0.0.1", self.1)) else {
            return (0, String::new());
        };
        write!(
            s,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).unwrap();
        let text = String::from_utf8_lossy(&raw).into_owned();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let status = head
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        (status, body.to_string())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn analyze_is_light_by_default_and_panels_load_on_demand() {
    let srv = Server::start();
    let (status, body) = srv.get("/api/analyze/sample/419a.replay");
    assert_eq!(status, 200);
    let light: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(
        body.len() < 500_000,
        "default response is data only: {} bytes",
        body.len()
    );
    for k in ["viewer_html", "scoring_html", "ballchasing_html"] {
        assert!(light.get(k).is_none(), "{k} stays on the server");
    }
    let id = light["analysis_id"]
        .as_str()
        .expect("analysis_id")
        .to_string();

    for panel in ["viewer", "scoring", "ballchasing"] {
        let (status, html) = srv.get(&format!("/api/analysis/{id}/{panel}"));
        assert_eq!(status, 200, "{panel}");
        assert!(html.contains("<html"), "{panel} is a full document");
    }
    assert_eq!(srv.get(&format!("/api/analysis/{id}/nope")).0, 404);
    assert_eq!(srv.get("/api/analysis/unknown/viewer").0, 404);
}

#[test]
fn inline_keeps_the_legacy_shape() {
    let srv = Server::start();
    let (status, body) = srv.get("/api/analyze/sample/419a.replay?inline=1");
    assert_eq!(status, 200);
    let full: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(full["viewer_html"]
        .as_str()
        .is_some_and(|h| h.contains("<html")));
    assert!(full.get("analysis_id").is_none());
}

#[test]
fn gzip_is_sent_only_when_asked_for_and_decodes_to_the_same_bytes() {
    let srv = Server::start();
    let (head, plain) = srv.get_raw("/", "");
    assert!(!head.to_lowercase().contains("content-encoding"), "{head}");
    let (head, z) = srv.get_raw("/", "Accept-Encoding: gzip\r\n");
    assert!(head.contains("Content-Encoding: gzip"), "{head}");
    assert!(z.len() < plain.len() / 2 && z[..2] == [0x1f, 0x8b]);
    let mut gz = Command::new("gzip")
        .arg("-dc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("gzip");
    let mut stdin = gz.stdin.take().unwrap();
    let feeder = std::thread::spawn(move || stdin.write_all(&z));
    let out = gz.wait_with_output().unwrap();
    feeder.join().unwrap().unwrap();
    assert_eq!(
        out.stdout, plain,
        "the compressed page decodes to the plain page"
    );
}
