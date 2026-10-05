//! A runner for any [`Channel`]: `rusty_serve` in, an AG-UI agent through
//! `rusty_agui`'s client, `rusty_tls` out to the service.
//!
//! The service wants a prompt `200` (Slack within three seconds) and an
//! agent can take longer, so the request is answered first and the run
//! happens on its own thread. One run at a time per process, which also
//! keeps one conversation in order. Threads live in memory for as long as
//! the process does.

use std::collections::HashMap;
use std::io::Write as _;
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusty_agui::HttpAgent;
use rusty_http::body::response_framing;
use rusty_http::head::RequestHead;
use rusty_http::sync::SyncTransport;
use rusty_http::{HeaderMap, Method, StatusCode, Url, Version};
use rusty_serve::{Handler, Request, Response, Server};
use rusty_tls::{TlsConnector, TrustPolicy};

use crate::{Channel, Credential, Error, Headers, Outbound, Received, Reply, Thread};

/// A bot: one channel in front of one agent.
pub struct Bot<C> {
    channel: Arc<C>,
    agent: Arc<HttpAgent>,
    tls: Arc<TlsConnector>,
    path: &'static str,
    threads: Arc<Mutex<HashMap<String, Thread>>>,
}

impl<C: Channel + 'static> Bot<C> {
    /// A bot answering `channel`'s requests at `path` with `agent`.
    pub fn new(channel: C, agent: HttpAgent, path: &'static str) -> Result<Self, String> {
        let tls = TlsConnector::new(&TrustPolicy::System).map_err(|e| e.to_string())?;
        Ok(Bot {
            channel: Arc::new(channel),
            agent: Arc::new(agent),
            tls: Arc::new(tls),
            path,
            threads: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Serve until the process ends, printing `LISTENING <url>` first.
    pub fn serve(self, bind: SocketAddr) -> Result<(), String> {
        let path = self.path;
        let server = Server::bind(bind, self).map_err(|e| e.to_string())?;
        let local = server.local_addr().map_err(|e| e.to_string())?;
        println!("LISTENING http://{local}{path}");
        let _ = std::io::stdout().flush();
        server.run().map_err(|e| e.to_string())
    }

    /// Run the agent for `inbound` and send the reply; the slow part,
    /// off the request thread.
    fn answer(&self, inbound: crate::Inbound) {
        let (channel, agent, tls, threads) = (
            Arc::clone(&self.channel),
            Arc::clone(&self.agent),
            Arc::clone(&self.tls),
            Arc::clone(&self.threads),
        );
        std::thread::spawn(move || {
            let Ok(mut threads) = threads.lock() else {
                return;
            };
            let thread = threads
                .entry(inbound.conversation.clone())
                .or_insert_with(|| Thread::new(channel.name(), &inbound.conversation));
            let input = thread.run(&inbound);
            let reply = match agent.run(&input) {
                Ok(stream) => thread.absorb(stream),
                Err(e) => Reply {
                    text: String::new(),
                    error: Some(e.to_string()),
                },
            };
            let text = match (&reply.error, reply.text.is_empty()) {
                (None, false) => reply.text,
                (None, true) => "(the agent had nothing to say)".into(),
                (Some(e), true) => format!("Something went wrong: {e}"),
                (Some(e), false) => format!("{}\n\n(then something went wrong: {e})", reply.text),
            };
            if let Err(e) = send_reply(&*channel, &tls, &inbound, &text) {
                eprintln!("reply to {} failed: {e}", inbound.conversation);
            }
        });
    }
}

impl<C: Channel + 'static> Handler for Bot<C> {
    fn handle(&mut self, request: &Request<'_>) -> Response {
        if *request.method != Method::Post || request.target != self.path {
            return Response::json(StatusCode::NOT_FOUND, b"{}".to_vec());
        }
        match self
            .channel
            .receive(&Head(request.headers), request.body, now())
        {
            Ok(Received::Challenge(challenge)) => {
                let body = format!(
                    r#"{{"challenge":{}}}"#,
                    rusty_json::Value::String(challenge).to_json_string()
                );
                Response::json(StatusCode::OK, body.into_bytes())
            }
            Ok(Received::Message(inbound)) => {
                self.answer(inbound);
                Response::json(StatusCode::OK, b"{}".to_vec())
            }
            Ok(Received::Ignored(why)) => {
                eprintln!("ignored: {why}");
                Response::json(StatusCode::OK, b"{}".to_vec())
            }
            Err(e) => {
                eprintln!("refused: {e}");
                let status = match e {
                    Error::Signature(_) => StatusCode::UNAUTHORIZED,
                    _ => StatusCode::BAD_REQUEST,
                };
                Response::json(status, b"{}".to_vec())
            }
        }
    }
}

/// Fetch the channel's credential if it needs one, then post the reply.
fn send_reply(
    channel: &dyn Channel,
    tls: &TlsConnector,
    to: &crate::Inbound,
    text: &str,
) -> Result<(), String> {
    if let Credential::Fetch(request) = channel.credential(now()) {
        let (status, body) = post(tls, &request)?;
        if !(200..300).contains(&status) {
            return Err(format!(
                "credential: {status} {}",
                String::from_utf8_lossy(&body).trim()
            ));
        }
        channel
            .accept_credential(&body, now())
            .map_err(|e| e.to_string())?;
    }
    let (status, body) = post(tls, &channel.reply(to, text))?;
    channel.reply_accepted(status, &body)
}

/// `rusty_serve`'s headers, as a channel reads them.
struct Head<'a>(&'a HeaderMap);

impl Headers for Head<'_> {
    fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `GET` a URL over TLS: the status and body.
pub fn get(tls: &TlsConnector, url: &str) -> Result<(u16, Vec<u8>), String> {
    exchange(tls, Method::Get, url, &[], &[])
}

/// Send one [`Outbound`] over TLS: the status and body.
pub fn post(tls: &TlsConnector, out: &Outbound) -> Result<(u16, Vec<u8>), String> {
    exchange(tls, Method::Post, &out.url, &out.headers, &out.body)
}

fn exchange(
    tls: &TlsConnector,
    method: Method,
    url: &str,
    extra: &[(String, String)],
    body: &[u8],
) -> Result<(u16, Vec<u8>), String> {
    let url = Url::parse(url).map_err(|e| e.to_string())?;
    if url.scheme != "https" {
        return Err(format!("{url:?}: only https:// is sent", url = url.host));
    }
    let tcp = TcpStream::connect((url.host.as_str(), url.port)).map_err(|e| e.to_string())?;
    let mut stream = tls.connect(tcp, &url.host).map_err(|e| e.to_string())?;
    stream.complete_handshake().map_err(|e| e.to_string())?;
    let mut headers = HeaderMap::new();
    let _ = headers.insert("Host", &url.host_header());
    let _ = headers.insert("Content-Length", &body.len().to_string());
    let _ = headers.insert("Connection", "close");
    for (name, value) in extra {
        let _ = headers.insert(name, value);
    }
    let mut transport = SyncTransport::new(stream);
    transport
        .write_request_head(&RequestHead {
            method: method.clone(),
            target: url.request_target(),
            version: Version::Http11,
            headers,
        })
        .and_then(|()| transport.write_body(body))
        .map_err(|e| e.to_string())?;
    let head = transport
        .read_response_head(16 * 1024)
        .map_err(|e| e.to_string())?;
    let framing =
        response_framing(&head.headers, &method, head.status).map_err(|e| e.to_string())?;
    let mut reader = transport.into_body_reader(framing);
    let mut out = Vec::new();
    while let Ok(Some(chunk)) = reader.next_chunk() {
        out.extend_from_slice(&chunk);
    }
    Ok((head.status.as_u16(), out))
}
