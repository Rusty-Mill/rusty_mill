#![cfg(unix)]

//! End-to-end: serve the LocalAPI over a real Unix socket and drive it with
//! an HTTP/1.1 client, exactly as `ts-cli` does. A mock backend stands in for
//! the engine so the test is hermetic.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll, ready};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, header};
use rusty_tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use rusty_tokio::net::UnixStream;
use ts_localapi::LocalBackend;
use ts_types::{MaskedPrefs, PingResult, Prefs, Status};

struct MockBackend {
    want_running: Mutex<bool>,
}

impl LocalBackend for MockBackend {
    async fn status(&self) -> Status {
        Status {
            version: "test".into(),
            backend_state: if *self.want_running.lock().unwrap() {
                "Running"
            } else {
                "Stopped"
            }
            .into(),
            ..Default::default()
        }
    }

    async fn edit_prefs(&self, masked: MaskedPrefs) -> Prefs {
        if let Some(w) = masked.want_running {
            *self.want_running.lock().unwrap() = w;
        }
        Prefs {
            want_running: *self.want_running.lock().unwrap(),
            ..Default::default()
        }
    }

    async fn ping(&self, ip: std::net::IpAddr) -> PingResult {
        PingResult {
            ip: ip.to_string(),
            node_ip: ip.to_string(),
            node_name: "mock".into(),
            latency_seconds: 0.001,
            ..Default::default()
        }
    }
}

/// A `rusty_tokio` stream under hyper's own I/O traits, so the independent
/// client runs on `rusty_tokio`. Reads go through a stack buffer to stay in
/// safe code.
struct HyperIo<T>(T);

impl<T: AsyncRead + Unpin> hyper::rt::Read for HyperIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut tmp = [0u8; 8192];
        let len = buf.remaining().min(tmp.len());
        let mut ours = ReadBuf::new(&mut tmp[..len]);
        ready!(Pin::new(&mut self.0).poll_read(cx, &mut ours))?;
        buf.put_slice(ours.filled());
        Poll::Ready(Ok(()))
    }
}

impl<T: AsyncWrite + Unpin> hyper::rt::Write for HyperIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

/// One request/response over a fresh UDS connection (mirrors ts-cli's client).
async fn request(socket: &PathBuf, method: Method, uri: &str, body: Bytes) -> (u16, Bytes) {
    let stream = UnixStream::connect(socket).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(HyperIo(stream))
        .await
        .unwrap();
    let driver = rusty_tokio::spawn(conn);
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "local-tailscaled.sock")
        .body(Full::new(body))
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status().as_u16();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    driver.abort();
    (status, bytes)
}

#[rusty_tokio::test]
async fn status_prefs_ping_over_uds() {
    let dir = std::env::temp_dir().join(format!("ts-localapi-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("localapi.sock");

    let backend = MockBackend {
        want_running: Mutex::new(true),
    };
    let serve_socket = socket.clone();
    rusty_tokio::spawn(async move {
        ts_localapi::serve(&serve_socket, backend).await.unwrap();
    });

    // Wait for the socket to appear.
    for _ in 0..50 {
        if socket.exists() {
            break;
        }
        rusty_tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // GET status → Running.
    let (code, body) = request(&socket, Method::GET, "/localapi/v0/status", Bytes::new()).await;
    assert_eq!(code, 200);
    let st: Status = serde_json::from_slice(&body).unwrap();
    assert_eq!(st.backend_state, "Running");

    // PATCH prefs down → prefs reflect WantRunning=false.
    let masked = serde_json::to_vec(&MaskedPrefs {
        want_running: Some(false),
    })
    .unwrap();
    let (code, body) = request(
        &socket,
        Method::PATCH,
        "/localapi/v0/prefs",
        Bytes::from(masked),
    )
    .await;
    assert_eq!(code, 200);
    let prefs: Prefs = serde_json::from_slice(&body).unwrap();
    assert!(!prefs.want_running);

    // Status now Stopped.
    let (_, body) = request(&socket, Method::GET, "/localapi/v0/status", Bytes::new()).await;
    let st: Status = serde_json::from_slice(&body).unwrap();
    assert_eq!(st.backend_state, "Stopped");

    // POST ping → PingResult for the queried IP.
    let (code, body) = request(
        &socket,
        Method::POST,
        "/localapi/v0/ping?ip=100.64.0.2&type=disco",
        Bytes::new(),
    )
    .await;
    assert_eq!(code, 200);
    let pr: PingResult = serde_json::from_slice(&body).unwrap();
    assert_eq!(pr.ip, "100.64.0.2");
    assert_eq!(pr.node_name, "mock");

    // Unknown route → 404.
    let (code, _) = request(&socket, Method::GET, "/nope", Bytes::new()).await;
    assert_eq!(code, 404);

    let _ = std::fs::remove_dir_all(&dir);
}
