//! A Slack bot in front of any AG-UI agent.
//!
//! Slack posts events here; each message becomes a run of the agent at
//! `AGENT_URL` (an agent directly, or `rusty_agent_gateway`'s `agui`
//! route), and the run's text goes back to the Slack thread it came from.
//!
//! ```text
//! SLACK_SIGNING_SECRET=… SLACK_BOT_TOKEN=xoxb-… AGENT_URL=http://127.0.0.1:8080/api/agent \
//!   cargo run -p rusty_channel --features bot --example slack_bot
//! ```
//!
//! Optional: `BIND` (default `127.0.0.1:3100`), `AGENT_TOKEN` (sent as a
//! bearer token to the agent). Point the Slack app's Events API request
//! URL at `/slack/events` (through a public tunnel) and subscribe it to
//! `app_mention` and `message.im`.
//!
//! Slack wants a `200` within three seconds and an agent can take longer,
//! so the run happens on its own thread after the request is answered.
//! Threads live in memory for as long as the process does.

use std::collections::HashMap;
use std::io::Write as _;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusty_agui::HttpAgent;
use rusty_channel::slack::Slack;
use rusty_channel::{Channel, Headers, Outbound, Received, Thread};
use rusty_http::body::response_framing;
use rusty_http::head::RequestHead;
use rusty_http::sync::SyncTransport;
use rusty_http::{HeaderMap, Method, StatusCode, Url, Version};
use rusty_serve::{Handler, Request, Response, Server};
use rusty_tls::{TlsConnector, TrustPolicy};

const EVENTS_PATH: &str = "/slack/events";

struct Bot {
    slack: Arc<Slack>,
    agent: Arc<HttpAgent>,
    tls: Arc<TlsConnector>,
    threads: Arc<Mutex<HashMap<String, Thread>>>,
}

impl Handler for Bot {
    fn handle(&mut self, request: &Request<'_>) -> Response {
        if *request.method != Method::Post || request.target != EVENTS_PATH {
            return Response::json(StatusCode::NOT_FOUND, b"{}".to_vec());
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        match self
            .slack
            .receive(&Head(request.headers), request.body, now)
        {
            Ok(Received::Challenge(challenge)) => {
                let body = format!(
                    r#"{{"challenge":{}}}"#,
                    rusty_json::Value::String(challenge).to_json_string()
                );
                Response::json(StatusCode::OK, body.into_bytes())
            }
            Ok(Received::Message(inbound)) => {
                let (slack, agent, tls, threads) = (
                    Arc::clone(&self.slack),
                    Arc::clone(&self.agent),
                    Arc::clone(&self.tls),
                    Arc::clone(&self.threads),
                );
                std::thread::spawn(move || {
                    // One run at a time per process: the map lock is held for
                    // the run, which also keeps one conversation in order.
                    let Ok(mut threads) = threads.lock() else {
                        return;
                    };
                    let thread = threads
                        .entry(inbound.conversation.clone())
                        .or_insert_with(|| Thread::new(slack.name(), &inbound.conversation));
                    let input = thread.run(&inbound);
                    let reply = match agent.run(&input) {
                        Ok(stream) => thread.absorb(stream),
                        Err(e) => rusty_channel::Reply {
                            text: String::new(),
                            error: Some(e.to_string()),
                        },
                    };
                    let text = match (&reply.error, reply.text.is_empty()) {
                        (None, false) => reply.text,
                        (None, true) => "(the agent had nothing to say)".into(),
                        (Some(e), true) => format!("Something went wrong: {e}"),
                        (Some(e), false) => {
                            format!("{}\n\n(then something went wrong: {e})", reply.text)
                        }
                    };
                    if let Err(e) = post(&tls, &slack.reply(&inbound, &text)) {
                        eprintln!("reply to {} failed: {e}", inbound.conversation);
                    }
                });
                Response::json(StatusCode::OK, b"{}".to_vec())
            }
            Ok(Received::Ignored(why)) => {
                eprintln!("ignored: {why}");
                Response::json(StatusCode::OK, b"{}".to_vec())
            }
            Err(e) => {
                eprintln!("refused: {e}");
                let status = match e {
                    rusty_channel::Error::Signature(_) => StatusCode::UNAUTHORIZED,
                    _ => StatusCode::BAD_REQUEST,
                };
                Response::json(status, b"{}".to_vec())
            }
        }
    }
}

/// `rusty_serve`'s headers, as the channel reads them.
struct Head<'a>(&'a HeaderMap);

impl Headers for Head<'_> {
    fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name)
    }
}

/// Send one `Outbound` over TLS and check Slack said `ok`.
fn post(tls: &TlsConnector, out: &Outbound) -> Result<(), String> {
    let url = Url::parse(&out.url).map_err(|e| e.to_string())?;
    let tcp = TcpStream::connect((url.host.as_str(), url.port)).map_err(|e| e.to_string())?;
    let mut stream = tls.connect(tcp, &url.host).map_err(|e| e.to_string())?;
    stream.complete_handshake().map_err(|e| e.to_string())?;
    let mut headers = HeaderMap::new();
    let _ = headers.insert("Host", &url.host_header());
    let _ = headers.insert("Content-Length", &out.body.len().to_string());
    let _ = headers.insert("Connection", "close");
    for (name, value) in &out.headers {
        let _ = headers.insert(name, value);
    }
    let mut transport = SyncTransport::new(stream);
    transport
        .write_request_head(&RequestHead {
            method: Method::Post,
            target: url.request_target(),
            version: Version::Http11,
            headers,
        })
        .and_then(|()| transport.write_body(&out.body))
        .map_err(|e| e.to_string())?;
    let head = transport
        .read_response_head(16 * 1024)
        .map_err(|e| e.to_string())?;
    let framing =
        response_framing(&head.headers, &Method::Post, head.status).map_err(|e| e.to_string())?;
    let mut reader = transport.into_body_reader(framing);
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = reader.next_chunk() {
        body.extend_from_slice(&chunk);
    }
    let body = String::from_utf8_lossy(&body);
    if head.status.as_u16() != 200 || !body.contains(r#""ok":true"#) {
        return Err(format!("{} {}", head.status.as_u16(), body.trim()));
    }
    Ok(())
}

fn env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is not set"))
}

fn main() -> Result<(), String> {
    let slack = Slack::new(env("SLACK_SIGNING_SECRET")?, env("SLACK_BOT_TOKEN")?);
    let mut agent = HttpAgent::new(&env("AGENT_URL")?).map_err(|e| e.to_string())?;
    if let Ok(token) = std::env::var("AGENT_TOKEN") {
        agent = agent.header("Authorization", &format!("Bearer {token}"));
    }
    let bind = std::env::var("BIND").unwrap_or_else(|_| "127.0.0.1:3100".into());
    let tls = TlsConnector::new(&TrustPolicy::System).map_err(|e| e.to_string())?;
    let bot = Bot {
        slack: Arc::new(slack),
        agent: Arc::new(agent),
        tls: Arc::new(tls),
        threads: Arc::new(Mutex::new(HashMap::new())),
    };
    let server = Server::bind(
        bind.parse()
            .map_err(|_| format!("BIND {bind:?} is not an address"))?,
        bot,
    )
    .map_err(|e| e.to_string())?;
    let local = server.local_addr().map_err(|e| e.to_string())?;
    println!("LISTENING http://{local}{EVENTS_PATH}");
    let _ = std::io::stdout().flush();
    server.run().map_err(|e| e.to_string())
}
