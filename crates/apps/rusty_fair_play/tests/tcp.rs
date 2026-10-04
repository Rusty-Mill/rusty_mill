//! End to end over a real socket: framing, the static UI, limits, shutdown.

use rusty_fair_play::api::Api;
use rusty_fair_play::server::{Server, ShutdownHandle, MAX_BODY_BYTES};
use rusty_fair_play::service::Service;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread::JoinHandle;
use std::time::Duration;

struct Running {
    addr: SocketAddr,
    stop: ShutdownHandle,
    thread: Option<JoinHandle<()>>,
    _dir: tempfile::TempDir,
}

impl Running {
    fn start(web_dir: Option<std::path::PathBuf>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut service = Service::open(dir.path()).unwrap();
        service.seed_deck().unwrap();
        let api = Api::new(service, None).unwrap();
        let mut server = Server::bind("127.0.0.1:0".parse().unwrap(), api).unwrap();
        if let Some(web) = web_dir {
            server = server.with_web_dir(web);
        }
        let (addr, stop) = (
            server.local_addr().unwrap(),
            server.shutdown_handle().unwrap(),
        );
        let thread = std::thread::spawn(move || server.run().unwrap());
        Self {
            addr,
            stop,
            thread: Some(thread),
            _dir: dir,
        }
    }

    fn exchange(&self, raw: &str) -> String {
        let mut stream = TcpStream::connect(self.addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.write_all(raw.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        out
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn request(method: &str, path: &str, body: &str) -> String {
    format!(
        "{method} {path} HTTP/1.1\r\nHost: t\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

fn status(response: &str) -> u16 {
    response.split(' ').nth(1).unwrap().parse().unwrap()
}

#[test]
fn serves_the_api_and_the_ui_over_a_socket() {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(web.path().join("index.html"), "<html>fair play</html>").unwrap();
    let server = Running::start(Some(web.path().to_path_buf()));

    let health = server.exchange(&request("GET", "/health", ""));
    assert_eq!(status(&health), 200);
    assert!(health.contains(r#"{"status":"ok"}"#));

    let snapshot = server.exchange(&request("GET", "/api/v1/snapshot", ""));
    assert_eq!(status(&snapshot), 200);
    assert!(snapshot.contains("Meals (Weekday Dinner)"));

    let created = server.exchange(&request("POST", "/api/v1/people", r#"{"name":"Ada"}"#));
    assert_eq!(status(&created), 201);

    // The UI: `/` and an unknown client route serve index.html with a CSP; the API never does.
    for path in ["/", "/deck/abc"] {
        let page = server.exchange(&request("GET", path, ""));
        assert_eq!(status(&page), 200, "{path}");
        assert!(page.contains("Content-Security-Policy"), "{path}");
        assert!(page.ends_with("<html>fair play</html>"), "{path}");
    }
    let missing = server.exchange(&request("GET", "/api/v1/nothing", ""));
    assert_eq!(status(&missing), 404);

    let too_big = server.exchange(&format!(
        "POST /api/v1/people HTTP/1.1\r\nHost: t\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        MAX_BODY_BYTES + 1
    ));
    assert_eq!(status(&too_big), 413);
    let garbage = server.exchange("not http at all\r\n\r\n");
    assert_eq!(status(&garbage), 400);
}
