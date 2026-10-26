//! `Server::stdio` end to end, in a child process: a small response must
//! reach the client while the server is still running, not when it exits
//! or writes again. The test re-runs its own binary as the server.
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use rusty_lsp::error::Result;
use rusty_lsp::lsp::{InitializeParams, InitializeResult};
use rusty_lsp::{LanguageServer, Server};

const CHILD_ENV: &str = "RUSTY_LSP_STDIO_CHILD";

struct Backend;

impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult::default())
    }
}

/// The server half; a no-op unless the parent test launched it.
#[test]
fn stdio_server_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    rusty_tokio::Runtime::new()
        .unwrap()
        .block_on(Server::stdio().serve(|_| Backend))
        .unwrap();
}

/// Reads one framed message, skipping the test harness's own stdout output.
fn read_frame(out: impl Read) -> String {
    let mut out = BufReader::new(out);
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        assert!(
            out.read_line(&mut line).unwrap() > 0,
            "server stdout closed"
        );
        // libtest's "test name ... " has no newline, so the header can
        // share its line.
        if let Some((_, n)) = line.split_once("Content-Length:") {
            length = Some(n.trim().parse::<usize>().unwrap());
        } else if line == "\r\n" && length.is_some() {
            break;
        }
    }
    let mut body = vec![0; length.unwrap()];
    out.read_exact(&mut body).unwrap();
    String::from_utf8(body).unwrap()
}

#[test]
fn stdio_delivers_a_small_response_while_the_server_runs() {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "stdio_server_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    let mut stdin = child.stdin.take().unwrap();
    write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    stdin.flush().unwrap();

    // Keep stdin open: the server stays up, so only a flush can deliver.
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || tx.send(read_frame(stdout)));
    let reply = rx.recv_timeout(Duration::from_secs(10));
    let _ = child.kill();
    let _ = child.wait();
    drop(stdin);

    let reply = reply.expect("initialize response delivered while the server runs");
    assert!(
        reply.contains(r#""id":1"#) && reply.contains(r#""result""#),
        "{reply}"
    );
}
