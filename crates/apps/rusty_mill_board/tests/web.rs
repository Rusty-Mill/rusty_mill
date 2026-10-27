//! The HTTP surface over a real socket: guard, API status mapping, UI, MCP.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;

use rusty_mcp_server::{HttpConfig, HttpHandler};
use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::drain::InFlight;
use rusty_mill_board::mcp;
use rusty_mill_board::store::MemStore;
use rusty_mill_board::web::{Auth, Web};

fn start(token: Option<&str>) -> SocketAddr {
    start_auth(Auth::from(token))
}

fn start_auth(auth: Auth) -> SocketAddr {
    start_web(auth).0
}

fn start_web(auth: Auth) -> (SocketAddr, Arc<InFlight>) {
    let board: Arc<Shared> = Arc::new(Board::new(Box::new(MemStore::default()), Roles::default()));
    let config = HttpConfig {
        allowed_hosts: vec!["127.0.0.1".into()],
        ..HttpConfig::default()
    };
    let path = config.path.clone();
    let mcp = HttpHandler::new(mcp::server(board.clone()).expect("server"), config);
    let web = Web::new(board, mcp, path, vec!["127.0.0.1".into()], auth);
    let in_flight = web.in_flight();
    let server =
        rusty_serve::Server::bind_shared("127.0.0.1:0".parse().expect("addr"), web).expect("bind");
    let addr = server.local_addr().expect("addr");
    std::thread::spawn(move || {
        let _ = server.run();
    });
    (addr, in_flight)
}

/// (status, body) of one request.
fn send(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, String) {
    let mut raw = format!(
        "{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        raw.push_str(&format!("Host: {addr}\r\n"));
    }
    for (k, v) in headers {
        raw.push_str(&format!("{k}: {v}\r\n"));
    }
    raw.push_str("\r\n");
    raw.push_str(body);
    let mut sock = TcpStream::connect(addr).expect("connect");
    sock.write_all(raw.as_bytes()).expect("write");
    let mut out = String::new();
    sock.read_to_string(&mut out).expect("read");
    let status = out
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status");
    (
        status,
        out.split("\r\n\r\n").nth(1).unwrap_or("").to_owned(),
    )
}

const JSON: (&str, &str) = ("Content-Type", "application/json");

#[test]
fn once_a_stop_begins_requests_wait_for_the_running_ones_and_new_ones_are_refused() {
    let (addr, in_flight) = start_web(Auth::from(Some("tok")));
    let auth = ("Authorization", "Bearer tok");
    assert_eq!(
        send(addr, "POST", "/api/board_list", &[JSON, auth], "{}").0,
        200
    );

    // A request is inside its handler (held here) when the stop begins: it is waited for, to a deadline.
    let running = std::thread::spawn({
        let flight = Arc::clone(&in_flight);
        move || {
            let held = flight.enter().expect("open");
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(held);
        }
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        !in_flight.close_and_wait(std::time::Duration::from_millis(20)),
        "still running at a short deadline"
    );
    assert_eq!(
        send(addr, "POST", "/api/board_list", &[JSON, auth], "{}").0,
        503
    );
    assert_eq!(
        send(addr, "GET", "/health", &[], "").0,
        503,
        "draining is not healthy"
    );
    running.join().expect("held request");
    assert!(in_flight.close_and_wait(std::time::Duration::from_secs(5)));
}

#[test]
fn health_answers_without_a_token_on_any_host() {
    let addr = start(Some("tok"));
    let (status, body) = send(addr, "GET", "/health", &[("Host", "other.lan")], "");
    assert_eq!((status, body.as_str()), (200, r#"{"ok":true}"#));
    assert_eq!(
        send(addr, "POST", "/health", &[JSON], "{}").0,
        404,
        "GET only"
    );
}

#[test]
fn guard_and_api() {
    let addr = start(Some("tok"));
    let auth = ("Authorization", "Bearer tok");
    assert_eq!(send(addr, "POST", "/api/board_list", &[JSON], "{}").0, 401);
    assert_eq!(
        send(
            addr,
            "POST",
            "/api/board_list",
            &[JSON, auth, ("Host", "evil.com")],
            "{}"
        )
        .0,
        403
    );
    assert_eq!(
        send(addr, "POST", "/api/board_list", &[auth], "{}").0,
        400,
        "non-JSON write refused"
    );
    assert_eq!(send(addr, "POST", "/api/nope", &[JSON, auth], "{}").0, 404);
    assert_eq!(
        send(
            addr,
            "POST",
            "/api/card_add",
            &[JSON, auth],
            r#"{"title":1}"#
        )
        .0,
        400
    );

    let (status, body) = send(
        addr,
        "POST",
        "/api/card_add",
        &[JSON, auth],
        r#"{"actor":"a","role":"human","title":"t"}"#,
    );
    assert_eq!(status, 200);
    let id = serde_json::from_str::<serde_json::Value>(&body).expect("json")["card"]["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let mv = |role: &str, to: &str| {
        let args = format!(r#"{{"actor":"a","role":"{role}","id":"{id}","to":"{to}"}}"#);
        send(addr, "POST", "/api/card_move", &[JSON, auth], &args).0
    };
    assert_eq!(mv("builder", "ready"), 403, "builder cannot triage");
    assert_eq!(mv("human", "done"), 422, "no such move");
    assert_eq!(mv("human", "ready"), 200);
}

#[test]
fn ui_is_served_without_token_but_requires_allowed_host() {
    let addr = start(Some("tok"));
    let (status, body) = send(addr, "GET", "/", &[], "");
    assert_eq!(status, 200);
    assert!(body.contains("Mill Board"));
    assert_eq!(send(addr, "GET", "/", &[("Host", "evil.com")], "").0, 403);
}

#[test]
fn mcp_requires_token_and_lists_tools() {
    let addr = start(Some("tok"));
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let accept = ("Accept", "application/json, text/event-stream");
    assert_eq!(send(addr, "POST", "/mcp", &[JSON, accept], call).0, 401);
    let (status, body) = send(
        addr,
        "POST",
        "/mcp",
        &[JSON, accept, ("Authorization", "Bearer tok")],
        call,
    );
    assert_eq!(status, 200);
    assert!(body.contains("card_move") && body.contains("board_events"));
}

const CLAUDE_TOKEN: &str = "claude-token-0123456789";
const CODEX_TOKEN: &str = "codex-token-0123456789a";

fn per_model() -> SocketAddr {
    let json = format!(
        r#"[{{"token":"{CLAUDE_TOKEN}","actor":"claude","role":"builder"}},{{"token":"{CODEX_TOKEN}","actor":"codex","role":"reviewer"}}]"#
    );
    start_auth(Auth::Credentials(
        rusty_mill_board::identities::Credentials::parse(
            &json,
            &Roles::default(),
            &rusty_mill_board::projects::Projects::default(),
        )
        .expect("credentials")
        .shared(),
    ))
}

fn bearer(token: &str) -> (&'static str, String) {
    ("Authorization", format!("Bearer {token}"))
}

fn api(addr: SocketAddr, token: &str, op: &str, body: &str) -> (u16, serde_json::Value) {
    let (k, v) = bearer(token);
    let (status, text) = send(addr, "POST", &format!("/api/{op}"), &[JSON, (k, &v)], body);
    (
        status,
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null),
    )
}

#[test]
fn per_model_tokens_fix_who_is_acting() {
    let addr = per_model();
    assert_eq!(
        api(addr, "nope-nope-nope-nope", "board_whoami", "{}").0,
        401
    );
    assert_eq!(
        send(addr, "POST", "/api/board_whoami", &[JSON], "{}").0,
        401,
        "no token"
    );

    let (status, me) = api(addr, CLAUDE_TOKEN, "board_whoami", "{}");
    assert_eq!(
        (status, me["actor"].clone(), me["role"].clone()),
        (200, "claude".into(), "builder".into())
    );

    // No actor or role in the call; the card's history carries the credential's actor.
    let (status, card) = api(addr, CLAUDE_TOKEN, "card_add", r#"{"title":"from claude"}"#);
    assert_eq!(status, 200);
    let id = card["card"]["id"].as_str().expect("id").to_owned();
    let (_, c) = api(
        addr,
        CLAUDE_TOKEN,
        "card_comment",
        &format!(r#"{{"id":"{id}","text":"hello"}}"#),
    );
    assert_eq!(c["card"]["comments"][0]["by"], "claude");

    // A model cannot claim someone else's role, nor use a role it does not hold.
    let (status, e) = api(
        addr,
        CLAUDE_TOKEN,
        "card_comment",
        &format!(r#"{{"id":"{id}","text":"x","role":"reviewer"}}"#),
    );
    assert_eq!(status, 422, "{e}");
    let (status, _) = api(
        addr,
        CLAUDE_TOKEN,
        "card_move",
        &format!(r#"{{"id":"{id}","to":"ready"}}"#),
    );
    assert_eq!(status, 403, "a builder cannot triage");
}

#[test]
fn mcp_calls_carry_the_callers_identity() {
    let addr = per_model();
    let accept = ("Accept", "application/json, text/event-stream");
    let call = |token: &str, name: &str, args: &str| {
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{name}","arguments":{args}}}}}"#
        );
        let (k, v) = bearer(token);
        let (status, text) = send(addr, "POST", "/mcp", &[JSON, accept, (k, &v)], &body);
        assert_eq!(status, 200, "{text}");
        let json_start = text.find('{').expect("json body");
        serde_json::from_str::<serde_json::Value>(text[json_start..].lines().next().unwrap_or(""))
            .expect("json-rpc reply")
    };

    let me = call(CODEX_TOKEN, "board_whoami", "{}");
    assert_eq!(me["result"]["structuredContent"]["actor"], "codex", "{me}");
    assert_eq!(me["result"]["structuredContent"]["role"], "reviewer");

    // A model that lies about its role gets a tool error naming the reason, not a protocol failure.
    let lie = call(CODEX_TOKEN, "card_add", r#"{"title":"t","role":"builder"}"#);
    assert_eq!(lie["result"]["isError"], true, "{lie}");
    assert!(
        lie["result"]["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains("fixed by your credential")
    );
}

// ---- OAuth ---------------------------------------------------------------

const ISSUER: &str = "http://localhost:8787";
const CALLBACK: &str = "https%3A%2F%2Fclaude.ai%2Fcb";

/// A board signing in under [`ISSUER`] with `auth`, its grants in a fresh directory.
fn start_oauth(auth: Auth) -> SocketAddr {
    start_oauth_at(auth, ISSUER)
}

/// [`start_oauth`] under `issuer`, accepting 127.0.0.1 and the issuer's hosts as `main` does.
fn start_oauth_at(auth: Auth, issuer: &str) -> SocketAddr {
    let issuer = rusty_oauth_server::validate_issuer(issuer).expect("issuer");
    let mut hosts = vec!["127.0.0.1".to_owned()];
    hosts.extend(rusty_mill_board::web::issuer_hosts(&issuer));
    let board: Arc<Shared> = Arc::new(Board::new(Box::new(MemStore::default()), Roles::default()));
    let config = HttpConfig {
        allowed_hosts: hosts.clone(),
        ..HttpConfig::default()
    };
    let path = config.path.clone();
    let mcp = HttpHandler::new(mcp::server(board.clone()).expect("server"), config);
    let state = std::env::temp_dir()
        .join(format!("mill-board-oauth-{}", uuid::Uuid::new_v4()))
        .join("oauth.json");
    let web = Web::new(board, mcp, path, hosts, auth)
        .with_oauth(issuer, state)
        .expect("oauth");
    let server =
        rusty_serve::Server::bind_shared("127.0.0.1:0".parse().expect("addr"), web).expect("bind");
    let addr = server.local_addr().expect("addr");
    std::thread::spawn(move || {
        let _ = server.run();
    });
    addr
}

const FORM: (&str, &str) = ("Content-Type", "application/x-www-form-urlencoded");

/// `name`'s value in a redirect's `Location` query.
fn location_param(raw: &str, name: &str) -> Option<String> {
    let location = raw.lines().find_map(|l| {
        l.strip_prefix("location: ")
            .or_else(|| l.strip_prefix("Location: "))
    })?;
    let query = location.split_once('?')?.1;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .map(str::to_owned)
}

/// One raw exchange, headers included.
fn raw(addr: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> String {
    let mut out = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(body);
    let mut sock = TcpStream::connect(addr).expect("connect");
    sock.write_all(out.as_bytes()).expect("write");
    let mut reply = String::new();
    sock.read_to_string(&mut reply).expect("read");
    reply
}

/// Register, authorize and consent with `token`: the redirect back to the client.
fn consent(addr: SocketAddr, token: &str) -> (String, String, String) {
    let (status, client) = send(
        addr,
        "POST",
        "/register",
        &[JSON],
        r#"{"redirect_uris":["https://claude.ai/cb"]}"#,
    );
    assert_eq!(status, 201, "{client}");
    let client: serde_json::Value = serde_json::from_str(&client).expect("client");
    let client_id = client["client_id"].as_str().expect("id").to_owned();
    let verifier = "v".repeat(64);
    let challenge = rusty_oauth::pkce::Pkce::from_verifier(
        verifier.clone(),
        rusty_oauth::pkce::CodeChallengeMethod::S256,
    )
    .expect("pkce")
    .code_challenge;
    let authorize = raw(
        addr,
        "GET",
        &format!(
            "/authorize?response_type=code&client_id={client_id}&redirect_uri={CALLBACK}&code_challenge={challenge}&code_challenge_method=S256"
        ),
        &[],
        "",
    );
    let txn = location_param(&authorize, "txn").expect("txn");
    let back = raw(
        addr,
        "POST",
        "/consent",
        &[FORM],
        &format!("txn={txn}&owner_token={token}&action=approve"),
    );
    (back, client_id, verifier)
}

/// The whole sign-in with `token`: an OAuth access token.
fn sign_in(addr: SocketAddr, token: &str) -> String {
    let (back, client_id, verifier) = consent(addr, token);
    let code = location_param(&back, "code").expect("code");
    let (status, issued) = send(
        addr,
        "POST",
        "/token",
        &[FORM],
        &format!(
            "grant_type=authorization_code&code={code}&client_id={client_id}&redirect_uri={CALLBACK}&code_verifier={verifier}"
        ),
    );
    assert_eq!(status, 200, "{issued}");
    let issued: serde_json::Value = serde_json::from_str(&issued).expect("tokens");
    issued["access_token"].as_str().expect("access").to_owned()
}

fn credentials() -> Arc<std::sync::RwLock<rusty_mill_board::identities::Credentials>> {
    let json = format!(
        r#"[{{"token":"{CLAUDE_TOKEN}","actor":"claude","role":"builder"}},{{"token":"{CODEX_TOKEN}","actor":"codex","role":"reviewer"}}]"#
    );
    rusty_mill_board::identities::Credentials::parse(
        &json,
        &Roles::default(),
        &rusty_mill_board::projects::Projects::default(),
    )
    .expect("credentials")
    .shared()
}

#[test]
fn an_oauth_grant_acts_as_the_agent_who_approved_it_until_they_are_removed() {
    let creds = credentials();
    let addr = start_oauth(Auth::Credentials(Arc::clone(&creds)));

    let metadata = send(
        addr,
        "GET",
        "/.well-known/oauth-authorization-server",
        &[],
        "",
    );
    assert_eq!(metadata.0, 200);
    assert!(metadata.1.contains(ISSUER), "{}", metadata.1);
    let accept = ("Accept", "application/json, text/event-stream");
    let refused = raw(addr, "POST", "/mcp", &[JSON, accept], "{}");
    assert!(refused.starts_with("HTTP/1.1 401"), "{refused}");
    assert!(
        refused.contains(&format!(
            "resource_metadata=\"{ISSUER}/.well-known/oauth-protected-resource/mcp\""
        )),
        "{refused}"
    );

    let claude = sign_in(addr, CLAUDE_TOKEN);
    let (status, me) = api(addr, &claude, "board_whoami", "{}");
    assert_eq!(
        (status, me["actor"].clone(), me["role"].clone()),
        (200, "claude".into(), "builder".into())
    );
    let codex = sign_in(addr, CODEX_TOKEN);
    assert_eq!(api(addr, &codex, "board_whoami", "{}").1["actor"], "codex");
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let (k, v) = bearer(&claude);
    assert_eq!(
        send(addr, "POST", "/mcp", &[JSON, accept, (k, &v)], call).0,
        200
    );

    // A role change applies to the grant at once; removing the agent suspends it.
    creds
        .write()
        .expect("lock")
        .update("claude", Some("qa".into()), None)
        .expect("update");
    assert_eq!(api(addr, &claude, "board_whoami", "{}").1["role"], "qa");
    creds
        .write()
        .expect("lock")
        .remove("claude")
        .expect("remove");
    assert_eq!(api(addr, &claude, "board_whoami", "{}").0, 401);
    assert_eq!(api(addr, &codex, "board_whoami", "{}").0, 200);
}

#[test]
fn consent_takes_only_the_boards_own_tokens() {
    let addr = start_oauth(Auth::Credentials(credentials()));
    let access = sign_in(addr, CLAUDE_TOKEN);
    for token in [access.as_str(), "nope-nope-nope-nope"] {
        let (back, _, _) = consent(addr, token);
        assert_eq!(
            location_param(&back, "error").as_deref(),
            Some("access_denied"),
            "{token}"
        );
    }
}

#[test]
fn with_a_shared_token_a_grant_is_that_tokens_caller_and_an_open_board_has_no_oauth() {
    let addr = start_oauth(Auth::from(Some("shared-token-0123456789")));
    let access = sign_in(addr, "shared-token-0123456789");
    let (status, me) = api(addr, &access, "board_whoami", "{}");
    assert_eq!((status, me["authenticated"].clone()), (200, false.into()));

    let board: Arc<Shared> = Arc::new(Board::new(Box::new(MemStore::default()), Roles::default()));
    let config = HttpConfig::default();
    let path = config.path.clone();
    let mcp = HttpHandler::new(mcp::server(board.clone()).expect("server"), config);
    let issuer = rusty_oauth_server::validate_issuer(ISSUER).expect("issuer");
    let open = Web::new(board, mcp, path, vec![], Auth::Open)
        .with_oauth(issuer, std::env::temp_dir().join("never-written.json"));
    assert!(open.is_err_and(|e| e.contains("--identities")));
}

#[test]
fn an_actor_named_by_two_credentials_can_neither_approve_nor_use_a_grant() {
    const HUMAN: &str = "x-human-token-0123456789";
    const BUILDER: &str = "x-builder-token-0123456";
    let entry =
        |token: &str, role: &str| format!(r#"{{"token":"{token}","actor":"x","role":"{role}"}}"#);
    let parse = |json: String| {
        rusty_mill_board::identities::Credentials::parse(
            &json,
            &Roles::default(),
            &rusty_mill_board::projects::Projects::default(),
        )
        .expect("credentials")
    };
    for (first, second) in [
        (entry(HUMAN, "human"), entry(BUILDER, "builder")),
        (entry(BUILDER, "builder"), entry(HUMAN, "human")),
    ] {
        let addr = start_oauth(Auth::Credentials(
            parse(format!("[{first},{second}]")).shared(),
        ));
        for token in [HUMAN, BUILDER] {
            let (back, _, _) = consent(addr, token);
            assert_eq!(
                location_param(&back, "error").as_deref(),
                Some("access_denied"),
                "{token}"
            );
            // The board's own tokens still tell the two apart.
            assert_eq!(api(addr, token, "board_whoami", "{}").0, 200);
        }
    }

    // A grant made while the actor was unique is refused once a reload makes it ambiguous.
    let creds = parse(format!("[{}]", entry(BUILDER, "builder"))).shared();
    let addr = start_oauth(Auth::Credentials(Arc::clone(&creds)));
    let access = sign_in(addr, BUILDER);
    assert_eq!(
        api(addr, &access, "board_whoami", "{}").1["role"],
        "builder"
    );
    *creds.write().expect("lock") = parse(format!(
        "[{},{}]",
        entry(HUMAN, "human"),
        entry(BUILDER, "builder")
    ));
    assert_eq!(
        api(addr, &access, "board_whoami", "{}").0,
        401,
        "no elevation"
    );
}

#[test]
fn an_ipv6_issuer_is_reachable_through_the_web_guard_and_mcp_alike() {
    let addr = start_oauth_at(
        Auth::Credentials(credentials()),
        "https://[2001:db8::1]:8787",
    );
    for host in ["[2001:db8::1]:8787", "[2001:db8::1]"] {
        let get = |path: &str| send(addr, "GET", path, &[("Host", host)], "");
        let metadata = get("/.well-known/oauth-authorization-server");
        assert_eq!(metadata.0, 200, "{host}: {}", metadata.1);
        assert!(
            metadata.1.contains("https://[2001:db8::1]:8787"),
            "{}",
            metadata.1
        );
        assert_eq!(
            get("/consent?txn=nope").0,
            400,
            "{host}: the consent page, not 403"
        );
        let accept = ("Accept", "application/json, text/event-stream");
        let (k, v) = bearer(CLAUDE_TOKEN);
        let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let mcp = send(
            addr,
            "POST",
            "/mcp",
            &[("Host", host), JSON, accept, (k, &v)],
            call,
        );
        assert_eq!(mcp.0, 200, "{host}: {}", mcp.1);
    }
    assert_eq!(
        send(
            addr,
            "GET",
            "/.well-known/oauth-authorization-server",
            &[("Host", "[2001:db8::2]")],
            ""
        )
        .0,
        403,
        "another address is still refused"
    );
}
