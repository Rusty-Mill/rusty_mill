//! `OpenAiHttp` against a scripted local server.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use orch_dispatch::ClassifiedError;
use orch_openai::{Config, OpenAiHttp, Prompt, Resolve, Usage};
use rusty_json::Value;

const PROMPT: Prompt<'static> = Prompt {
    system: "intent and rules",
    user: "claim C-1",
    max_output_tokens: 256,
};

const OK_BODY: &str = r#"{"choices":[{"message":{"content":"{\"posts\":[]}"}}],"usage":{"prompt_tokens":40,"completion_tokens":5}}"#;

/// What the server saw: the head lines and the body.
struct Seen {
    head: Vec<String>,
    body: String,
}

/// Serve one connection: read a request, then answer with `reply` (raw
/// bytes), or never answer when `reply` is `None`.
fn serve_once(reply: Option<String>) -> (SocketAddr, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let seen = read_request(&mut reader).expect("request");
        let _ = tx.send(seen);
        match reply {
            Some(raw) => {
                let mut out = stream;
                let _ = out.write_all(raw.as_bytes());
            }
            None => thread::sleep(Duration::from_secs(5)),
        }
    });
    (addr, rx)
}

fn read_request(reader: &mut impl BufRead) -> io::Result<Seen> {
    let mut head = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim_end().to_owned();
        if line.is_empty() {
            break;
        }
        head.push(line);
    }
    let len = head
        .iter()
        .find_map(|l| l.strip_prefix("Content-Length: "))
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; len];
    reader.read_exact(&mut body)?;
    Ok(Seen {
        head,
        body: String::from_utf8(body).expect("utf8"),
    })
}

fn response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

fn client(addr: SocketAddr, key_env: Option<&str>, timeout: Duration) -> OpenAiHttp {
    let url = format!("http://{addr}/v1");
    OpenAiHttp::new(Config::new(&url, "qwen", key_env, timeout, true).expect("config"))
}

const SECS: Duration = Duration::from_secs(5);

fn test_key(name: &str) -> Option<String> {
    (name == "LLAMA_KEY").then(|| "sk-test-secret".to_owned())
}

#[test]
fn posts_one_completion_and_reads_text_and_usage() {
    let (addr, seen) = serve_once(Some(response("200 OK", OK_BODY)));
    let c = client(addr, Some("LLAMA_KEY"), SECS)
        .with_env(test_key)
        .complete(&PROMPT)
        .expect("completes");
    assert_eq!(c.text, r#"{"posts":[]}"#);
    assert_eq!(
        c.usage,
        Usage {
            input_tokens: 40,
            output_tokens: 5,
            cached_input_tokens: None
        }
    );

    let seen = seen.recv().expect("request seen");
    assert_eq!(seen.head[0], "POST /v1/chat/completions HTTP/1.1");
    assert!(
        seen.head.contains(&format!("Host: {addr}")),
        "{:?}",
        seen.head
    );
    assert!(seen
        .head
        .contains(&"Authorization: Bearer sk-test-secret".to_owned()));
    let body = Value::parse(&seen.body).expect("json body");
    assert_eq!(body.pointer("/model").and_then(Value::as_str), Some("qwen"));
    assert_eq!(
        body.pointer("/messages/0/content").and_then(Value::as_str),
        Some("intent and rules")
    );
}

#[test]
fn a_chunked_reply_is_read() {
    let chunked = format!(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{OK_BODY}\r\n0\r\n\r\n",
        OK_BODY.len()
    );
    let (addr, _seen) = serve_once(Some(chunked));
    let c = client(addr, None, SECS)
        .complete(&PROMPT)
        .expect("completes");
    assert_eq!(c.usage.output_tokens, 5);
}

#[test]
fn no_key_configured_sends_no_authorization() {
    let (addr, seen) = serve_once(Some(response("200 OK", OK_BODY)));
    client(addr, None, SECS)
        .complete(&PROMPT)
        .expect("completes");
    let seen = seen.recv().expect("seen");
    assert!(!seen.head.iter().any(|l| l.starts_with("Authorization")));
}

#[test]
fn a_missing_key_is_unavailable_before_connecting() {
    let (addr, seen) = serve_once(Some(response("200 OK", OK_BODY)));
    let e = client(addr, Some("UNSET_KEY"), SECS)
        .with_env(test_key)
        .complete(&PROMPT)
        .expect_err("no key");
    assert!(matches!(&e, ClassifiedError::Unavailable(m) if m.0.contains("UNSET_KEY is not set")));
    assert!(seen.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn the_key_never_appears_in_an_error() {
    let echo = response(
        "500 Internal Server Error",
        "bad request with Bearer sk-test-secret",
    );
    let (addr, _seen) = serve_once(Some(echo));
    let e = client(addr, Some("LLAMA_KEY"), SECS)
        .with_env(test_key)
        .complete(&PROMPT)
        .expect_err("500");
    let ClassifiedError::Transient(m) = e else {
        panic!("500 is transient: {e:?}");
    };
    assert!(m.0.contains("HTTP 500"), "{m}");
    assert!(!m.0.contains("sk-test-secret"), "{m}");
}

#[test]
fn a_slow_server_times_out_as_transient() {
    let (addr, _seen) = serve_once(None);
    let start = Instant::now();
    let e = client(addr, None, Duration::from_millis(300))
        .complete(&PROMPT)
        .expect_err("times out");
    assert!(
        matches!(&e, ClassifiedError::Transient(m) if m.0.contains("timed out")),
        "{e:?}"
    );
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn a_closed_port_is_unavailable() {
    let addr = TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr");
    let e = client(addr, None, SECS)
        .complete(&PROMPT)
        .expect_err("refused");
    assert!(matches!(e, ClassifiedError::Unavailable(_)), "{e:?}");
}

#[test]
fn a_malformed_reply_is_permanent() {
    let (addr, _seen) = serve_once(Some(response("200 OK", r#"{"choices":[]}"#)));
    let e = client(addr, None, SECS).complete(&PROMPT).expect_err("bad");
    assert!(matches!(e, ClassifiedError::Permanent(_)), "{e:?}");
}

/// Answers every lookup with fixed addresses.
struct FixedDns(Vec<SocketAddr>);

impl Resolve for FixedDns {
    fn resolve(&self, _: &str, _: u16) -> io::Result<Vec<SocketAddr>> {
        Ok(self.0.clone())
    }
}

#[test]
fn a_public_or_mixed_answer_is_refused_and_nothing_is_sent() {
    let (addr, seen) = serve_once(Some(response("200 OK", OK_BODY)));
    let public: SocketAddr = "203.0.113.7:8080".parse().expect("addr");
    let link_local: SocketAddr = "169.254.0.1:8080".parse().expect("addr");
    for answer in [vec![public], vec![addr, public], vec![link_local], vec![]] {
        let e = OpenAiHttp::new(
            Config::new("http://llama.lan:8080/v1", "m", None, SECS, false).expect("config"),
        )
        .with_resolver(FixedDns(answer.clone()))
        .complete(&PROMPT)
        .expect_err("refused");
        let ClassifiedError::Permanent(m) = e else {
            panic!("refusal is permanent: {e:?}");
        };
        assert!(m.0.contains("refusing llama.lan"), "{m}");
        if let Some(bad) = answer.iter().find(|a| !orch_openai::is_private(a.ip())) {
            assert!(m.0.contains(&bad.ip().to_string()), "{m}");
        }
    }
    assert!(seen.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn it_connects_to_the_address_it_checked() {
    let (addr, seen) = serve_once(Some(response("200 OK", OK_BODY)));
    OpenAiHttp::new(Config::new("http://llama.lan:9/v1", "m", None, SECS, false).expect("config"))
        .with_resolver(FixedDns(vec![addr]))
        .complete(&PROMPT)
        .expect("completes");
    let seen = seen.recv().expect("seen");
    assert!(seen.head.contains(&"Host: llama.lan:9".to_owned()));
}
