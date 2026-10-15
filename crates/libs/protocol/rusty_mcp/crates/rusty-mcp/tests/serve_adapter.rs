//! The blocking-server adapters, against a real `rusty_serve` server.
//!
//! The decisions are unit-tested in the cores; what only shows up end to end is
//! that they hold on the wire: the status line, `WWW-Authenticate`,
//! `Retry-After`, and that the limit really sheds a second connection while the
//! first is still inside the handler.

#![cfg(feature = "serve")]

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use rusty_http::StatusCode;
use rusty_mcp::{
    auth::{AuthConfig, StaticTokenValidator, TokenValidator, ValidateFuture, VerifiedToken},
    serve::{AuthorizedHandler, LimitedHandler},
};
use rusty_serve::{Request, Response, Server, SharedHandler, ShutdownHandle};

/// Longest a test waits on the gate before failing.
const WAIT: Duration = Duration::from_secs(5);

const RESOURCE: &str = "https://mcp.example.com/mcp";

/// A reply as the client sees it.
struct Reply {
    status: u16,
    head: String,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }
}

/// One `GET` over a fresh connection.
fn get(addr: SocketAddr, target: &str, authorization: Option<&str>) -> Reply {
    let mut stream = TcpStream::connect(addr).expect("connect");
    let mut request = format!("GET {target} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n");
    if let Some(value) = authorization {
        request.push_str(&format!("Authorization: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).expect("write");

    let mut raw = String::new();
    stream.read_to_string(&mut raw).expect("read");
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("status line");
    Reply {
        status,
        head: head.to_owned(),
        body: body.to_owned(),
    }
}

/// Serve `handler` on a free port until the returned handle is shut down.
fn serve(handler: impl SharedHandler + 'static) -> (SocketAddr, ShutdownHandle) {
    let server = Server::bind_shared("127.0.0.1:0".parse().expect("addr"), handler).expect("bind");
    let addr = server.local_addr().expect("addr");
    let handle = server.shutdown_handle().expect("handle");
    thread::spawn(move || {
        let _ = server.run();
    });
    (addr, handle)
}

fn ok(body: &str) -> Response {
    Response::json(StatusCode::OK, body.as_bytes().to_vec())
}

fn config(validator: impl TokenValidator, required: &[&str]) -> AuthConfig {
    AuthConfig::new(RESOURCE, Arc::new(validator))
        .expect("valid resource")
        .with_required_scopes(required.iter().map(|s| s.to_string()))
}

fn tokens() -> StaticTokenValidator {
    StaticTokenValidator::new()
        .with_token(
            "good",
            VerifiedToken::new([RESOURCE])
                .with_scopes(["mcp:read"])
                .with_subject("user-1"),
        )
        .with_token(
            "thin",
            VerifiedToken::new([RESOURCE]).with_scopes(["other"]),
        )
        .with_token(
            "elsewhere",
            VerifiedToken::new(["https://other.example.com/mcp"]).with_scopes(["mcp:read"]),
        )
}

#[test]
fn a_valid_token_reaches_the_handler_with_its_claims() {
    let app = AuthorizedHandler::new(config(tokens(), &["mcp:read"]), |_req, token| {
        ok(&format!(
            "{{\"sub\":\"{}\"}}",
            token.subject.as_deref().unwrap_or("")
        ))
    });
    let (addr, stop) = serve(app);

    let reply = get(addr, "/mcp", Some("Bearer good"));
    stop.shutdown();

    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, r#"{"sub":"user-1"}"#);
}

#[test]
fn every_refusal_has_its_status_and_a_challenge() {
    let app = AuthorizedHandler::new(config(tokens(), &["mcp:read"]), |_req, _token| ok("{}"));
    let (addr, stop) = serve(app);

    let cases = [
        (None, 401, "Bearer"),
        (Some("Bearer unknown"), 401, "invalid_token"),
        (Some("Bearer elsewhere"), 401, "invalid_token"),
        (Some("Bearer thin"), 403, "insufficient_scope"),
        (Some("Basic dXNlcjpwdw=="), 400, "invalid_request"),
    ];
    for (authorization, status, needle) in cases {
        let reply = get(addr, "/mcp", authorization);
        assert_eq!(reply.status, status, "{authorization:?}");
        let challenge = reply
            .header("www-authenticate")
            .unwrap_or_else(|| panic!("{authorization:?}: no challenge"));
        assert!(challenge.contains(needle), "{authorization:?}: {challenge}");
        assert!(
            challenge.contains("resource_metadata="),
            "{authorization:?}: {challenge}"
        );
    }
    stop.shutdown();
}

/// Holds a request inside the handler until the test lets it go.
struct Gate {
    state: Mutex<(bool, bool)>, // (entered, open)
    changed: Condvar,
}

impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new((false, false)),
            changed: Condvar::new(),
        })
    }

    /// Called by the handler: announce arrival, then wait to be released.
    fn pass(&self) {
        let mut state = self.state.lock().expect("lock");
        state.0 = true;
        self.changed.notify_all();
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, WAIT, |state| !state.1)
            .expect("wait");
        drop(state);
        assert!(!timeout.timed_out(), "the gate was never opened");
    }

    /// Wait for a request to be inside the handler, or fail rather than hang.
    fn wait_until_entered(&self) {
        let state = self.state.lock().expect("lock");
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, WAIT, |state| !state.0)
            .expect("wait");
        drop(state);
        assert!(
            !timeout.timed_out(),
            "no request reached the handler (was the first one refused?)"
        );
    }

    fn open(&self) {
        self.state.lock().expect("lock").1 = true;
        self.changed.notify_all();
    }
}

fn gated(gate: &Arc<Gate>) -> impl Fn(&Request<'_>) -> Response + Send + Sync + use<> {
    let gate = Arc::clone(gate);
    move |request: &Request<'_>| {
        if request.target == "/slow" {
            gate.pass();
        }
        ok("{}")
    }
}

#[test]
fn a_second_request_over_the_limit_is_shed_while_the_first_is_inside() {
    let gate = Gate::new();
    let (addr, stop) = serve(LimitedHandler::new(gated(&gate), 1));

    let held = thread::spawn(move || get(addr, "/slow", None));
    gate.wait_until_entered();

    let shed = get(addr, "/fast", None);
    assert_eq!(shed.status, 503);
    assert_eq!(shed.header("retry-after"), Some("1"));
    assert!(shed.body.contains("overloaded"), "{}", shed.body);

    gate.open();
    assert_eq!(held.join().expect("join").status, 200);

    // The permit came back with the first response.
    assert_eq!(get(addr, "/fast", None).status, 200);
    stop.shutdown();
}

/// Counts how often a token is actually validated.
struct Counting(Arc<AtomicUsize>, StaticTokenValidator);

impl TokenValidator for Counting {
    fn validate<'a>(&'a self, token: &'a str) -> ValidateFuture<'a> {
        self.0.fetch_add(1, Ordering::SeqCst);
        self.1.validate(token)
    }
}

#[test]
fn a_shed_request_never_reaches_token_validation() {
    // Limits outside auth, as the module docs say: otherwise being flooded
    // with bad tokens would still cost a validation each.
    let gate = Gate::new();
    let validations = Arc::new(AtomicUsize::new(0));
    let inner = gated(&gate);
    let auth = AuthorizedHandler::new(
        config(Counting(Arc::clone(&validations), tokens()), &[]),
        move |request, _token| inner(request),
    );
    let (addr, stop) = serve(LimitedHandler::new(auth, 1));

    let held = thread::spawn(move || get(addr, "/slow", Some("Bearer good")));
    gate.wait_until_entered();
    assert_eq!(validations.load(Ordering::SeqCst), 1);

    let shed = get(addr, "/fast", Some("Bearer not-even-valid"));
    assert_eq!(shed.status, 503, "shed, not 401");
    assert_eq!(
        validations.load(Ordering::SeqCst),
        1,
        "the shed request was validated"
    );

    gate.open();
    assert_eq!(held.join().expect("join").status, 200);
    stop.shutdown();
}
