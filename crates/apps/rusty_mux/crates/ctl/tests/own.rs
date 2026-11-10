//! `rusty-mux-ctl own` against a real server (C46): it owns what it
//! launches, nobody else may change that, and a dropped connection is taken
//! over again inside the server's grace.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener as StdListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusty_json::Value;
use rusty_mux_core::BlockId;
use rusty_mux_proto::{Features, Message, decode, encode};
use rusty_mux_server::{Config, ConsoleHost, serve};
use rusty_tokio::io::UnixListener;
use rusty_tokio::{Builder, LocalOptions};

const PATIENCE: Duration = Duration::from_secs(20);

fn socket_path(tag: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "rusty-mux-own-{tag}-{}-{n}.sock",
        std::process::id()
    ))
}

fn start_server() -> PathBuf {
    let path = socket_path("srv");
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let bind_path = path.clone();
    let config = Config {
        program: "/bin/sh".into(),
        args: Vec::new(),
        cwd: "/".into(),
        console_host: ConsoleHost::default(),
        state_dir: None,
        grapheme_clustering: false,
        config_file: None,
    };
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime");
        runtime.block_on(async move {
            let listener = UnixListener::bind(&bind_path).expect("bind");
            ready_tx.send(()).expect("signal ready");
            serve(listener, config).await.expect("serve");
        });
    });
    ready_rx.recv_timeout(PATIENCE).expect("server bound");
    path
}

/// A socket that forwards each connection to `server`, and can cut every
/// connection it carries, as a network drop would.
struct Proxy {
    path: PathBuf,
    open: Arc<Mutex<Vec<UnixStream>>>,
}

impl Proxy {
    fn to(server: &Path) -> Self {
        let path = socket_path("proxy");
        let listener = StdListener::bind(&path).expect("bind the proxy");
        let open = Arc::new(Mutex::new(Vec::new()));
        let (server, carried) = (server.to_owned(), open.clone());
        std::thread::spawn(move || {
            for client in listener.incoming() {
                let Ok(client) = client else { return };
                let Ok(upstream) = UnixStream::connect(&server) else {
                    return;
                };
                let mut carried = carried.lock().expect("lock");
                carried.push(client.try_clone().expect("clone"));
                carried.push(upstream.try_clone().expect("clone"));
                pump(
                    client.try_clone().expect("clone"),
                    upstream.try_clone().expect("clone"),
                );
                pump(upstream, client);
            }
        });
        Self { path, open }
    }

    fn cut(&self) {
        for stream in self.open.lock().expect("lock").drain(..) {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

fn pump(mut from: UnixStream, mut to: UnixStream) {
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut from, &mut to);
        let _ = to.shutdown(Shutdown::Write);
    });
}

fn ctl(socket: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusty-mux-ctl"))
        .env_remove("RUSTY_MUX_SOCKET")
        .env_remove("RUSTY_MUX_SESSION")
        .env_remove("RUSTY_MUX_WINDOW")
        .env_remove("RUSTY_MUX_BLOCK")
        .env_remove("RUSTY_MUX_CONFIG")
        .arg("--socket")
        .arg(socket)
        .args(args)
        .output()
        .expect("run rusty-mux-ctl")
}

fn ok(socket: &Path, args: &[&str]) -> String {
    let output = ctl(socket, args);
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout)
        .expect("utf-8")
        .trim()
        .to_owned()
}

fn window(socket: &Path) -> String {
    let session = ok(socket, &["create-session", "--name", "s"]);
    ok(
        socket,
        &["create-window", "--session", &session, "--name", "w"],
    )
}

/// A running `own`, its stdin and its stdout by line.
struct Own {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Own {
    fn start(socket: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rusty-mux-ctl"))
            .env_remove("RUSTY_MUX_SOCKET")
            .arg("--socket")
            .arg(socket)
            .arg("own")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("start own");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{line}").expect("write a request");
    }

    fn line(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("read a line");
        assert!(!line.is_empty(), "own ended early");
        Value::from_json_str(&line).expect("a JSON line")
    }

    /// Lines until one has `key` equal to `want`.
    fn until(&mut self, key: &str, want: &str) -> Value {
        loop {
            let line = self.line();
            if line.get(key).and_then(Value::as_str) == Some(want) {
                return line;
            }
        }
    }

    fn answer(&mut self, id: u64) -> Value {
        loop {
            let line = self.line();
            if line.get("id").and_then(Value::as_u64) == Some(id) {
                return line;
            }
        }
    }

    fn launch(&mut self, id: u64, window: &str) -> String {
        self.send(&format!(
            r#"{{"id":{id},"op":"launch","window":"{window}"}}"#
        ));
        let answer = self.answer(id);
        assert_eq!(answer.get("ok"), Some(&Value::Bool(true)), "{answer:?}");
        answer["block"].as_str().expect("a block id").to_owned()
    }
}

impl Drop for Own {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The block's `owner` in a JSON listing.
fn owner(socket: &Path, block: &str) -> Value {
    ok(socket, &["--json", "list"])
        .lines()
        .map(|l| Value::from_json_str(l).expect("JSON"))
        .find(|e| e.get("block").and_then(Value::as_str) == Some(block))
        .expect("the block is listed")["owner"]
        .clone()
}

#[test]
fn own_owns_what_it_launches_and_hears_of_it() {
    let socket = start_server();
    let window = window(&socket);
    let mut own = Own::start(&socket);
    let block = own.launch(1, &window);
    assert!(
        owner(&socket, &block).as_str().is_some(),
        "owned by own's client"
    );

    let other = ctl(&socket, &["close-block", "--block", &block]);
    assert_eq!(other.status.code(), Some(1), "{other:?}");
    assert!(
        String::from_utf8_lossy(&other.stderr).contains("owned"),
        "{other:?}"
    );

    own.send(r#"{"id":"bad","op":"fly"}"#);
    let refused = own.until("id", "bad");
    assert_eq!(refused.get("ok"), Some(&Value::Bool(false)));

    own.send(&format!(r#"{{"id":2,"op":"close","block":"{block}"}}"#));
    assert_eq!(own.answer(2).get("ok"), Some(&Value::Bool(true)));
    let closed = own.until("event", "block-closed");
    assert_eq!(closed["block"].as_str(), Some(block.as_str()));
}

#[test]
fn own_takes_its_client_over_after_a_drop() {
    let socket = start_server();
    let proxy = Proxy::to(&socket);
    let window = window(&socket);
    let mut own = Own::start(&proxy.path);
    let block = own.launch(1, &window);
    let before = owner(&socket, &block);

    proxy.cut();
    let reconnected = own.until("event", "reconnected");
    assert_eq!(reconnected.get("resumed"), Some(&Value::Bool(true)));
    assert_eq!(
        owner(&socket, &block),
        before,
        "the same client still owns it"
    );

    own.send(&format!(r#"{{"id":2,"op":"close","block":"{block}"}}"#));
    assert_eq!(own.answer(2).get("ok"), Some(&Value::Bool(true)));
}

#[test]
fn the_end_of_stdin_leaves_the_block_to_orphan() {
    let socket = start_server();
    let window = window(&socket);
    let mut own = Own::start(&socket);
    let block = own.launch(1, &window);
    own.stdin = None;
    let status = own.child.wait().expect("own exits");
    assert!(status.success(), "{status:?}");
    // Kept for the server's grace: still owned, so still refused.
    let other = ctl(&socket, &["close-block", "--block", &block]);
    assert_eq!(other.status.code(), Some(1), "{other:?}");
}

/// The next message on `stream`, kept whole across reads in `buf`.
fn next(stream: &mut UnixStream, buf: &mut Vec<u8>) -> Message {
    loop {
        if let Some((message, used)) = decode(buf).expect("decode") {
            buf.drain(..used);
            return message;
        }
        let mut chunk = [0u8; 4096];
        let n = std::io::Read::read(stream, &mut chunk).expect("read");
        assert!(n > 0, "server closed the connection");
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn send(stream: &mut UnixStream, message: &Message) {
    stream
        .write_all(&encode(message).expect("encode"))
        .expect("write");
}

/// A bare client that asks for `block`'s lease, as a viewer would; the
/// connection, kept open, and the request's answer.
fn ask_for_lease(socket: &Path, block: &str) -> (UnixStream, Message) {
    let mut stream = UnixStream::connect(socket).expect("connect");
    let mut buf = Vec::new();
    let hello = Message::Hello {
        version: None,
        features: Features::LEASE_REQUESTS,
        resume: None,
        takeover: None,
    };
    send(&mut stream, &hello);
    assert!(matches!(
        next(&mut stream, &mut buf),
        Message::Welcome { .. }
    ));
    let block = BlockId::from_uuid(block.parse().expect("a block id"));
    send(&mut stream, &Message::RequestLease { req: 1, block });
    loop {
        // Events may come before the answer.
        let message = next(&mut stream, &mut buf);
        if matches!(message, Message::Done { .. } | Message::Failed { .. }) {
            return (stream, message);
        }
    }
}

#[test]
fn own_hears_a_viewer_ask_for_the_lease() {
    let socket = start_server();
    let window = window(&socket);
    let mut own = Own::start(&socket);
    own.send(&format!(
        r#"{{"id":1,"op":"launch","window":"{window}","input":"owner-only"}}"#
    ));
    let answer = own.answer(1);
    assert_eq!(answer.get("ok"), Some(&Value::Bool(true)), "{answer:?}");
    let block = answer["block"].as_str().expect("a block id").to_owned();

    let (_viewer, answer) = ask_for_lease(&socket, &block);
    assert!(matches!(answer, Message::Done { req: 1, .. }), "{answer:?}");
    let asked = own.until("event", "lease-requested");
    assert_eq!(asked["block"].as_str(), Some(block.as_str()));
    assert!(asked["principal"].as_str().is_some(), "{asked:?}");
}
