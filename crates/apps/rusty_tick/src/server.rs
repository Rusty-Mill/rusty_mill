//! The TCP adapter: [`rusty_serve`]'s blocking HTTP/1.1 server (one thread
//! per connection, bounded head, body, idle time and connection count)
//! with [`Backend`] as its [`Handler`]. Everything about the API itself
//! lives in [`crate::api`]; this file only converts request and response
//! shapes.

use crate::api;
use crate::backend::Backend;
pub use rusty_serve::{Handler, ShutdownHandle, DEFAULT_MAX_CONNECTIONS, MAX_BODY_BYTES};

pub type Server = rusty_serve::Server<Backend>;

/// The assistant's route: a streamed AG-UI run, so it bypasses the
/// buffered [`api::Response`] path.
const AGENT_PATH: &str = "/api/agent";
/// Fetching a calendar feed for the client: network, not the user's data.
const FETCH_ICS_PATH: &str = "/api/v1/fetch-ics";

impl Handler for Backend {
    fn handle(&mut self, request: &rusty_serve::Request<'_>) -> rusty_serve::Response {
        let api_request = api::Request {
            method: request.method,
            target: request.target,
            authorization: request.authorization,
            if_match: request.if_match,
            body: request.body,
        };
        let path = request.target.split('?').next().unwrap_or("");
        if path == AGENT_PATH && request.method == &rusty_http::Method::Post {
            return match self.authorize(&api_request) {
                Ok(_) => self.assistant.handle_run(request.body),
                Err(denied) => rusty_serve::Response::json(denied.status, denied.body),
            };
        }
        if path == FETCH_ICS_PATH && request.method == &rusty_http::Method::Post {
            // Authenticated here, under the lock; the fetch itself runs after the
            // lock is released, so a slow feed holds up no other request.
            return match self.authorize(&api_request) {
                Ok(_) => {
                    let body = request.body.to_vec();
                    rusty_serve::Response::deferred(move || {
                        let response = api::Api::fetch_ics(&body);
                        (response.status, response.body)
                    })
                }
                Err(denied) => rusty_serve::Response::json(denied.status, denied.body),
            };
        }
        let response = Backend::handle(self, &api_request);
        rusty_serve::Response::json(response.status, response.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Clock;
    use rusty_agui::{EventKind, HttpAgent, Message, RunAgentInput, Tool};
    use rusty_json::json;

    fn clock() -> Clock {
        Box::new(|| 1_700_000_000_000)
    }

    #[test]
    fn the_agent_route_streams_a_run_for_a_good_token_and_refuses_a_bad_one() {
        let dir = tempfile::tempdir().unwrap();
        let token = "a-token-long-enough-to-pass";
        let backend = Backend::single(dir.path(), token.into(), clock()).unwrap();
        let server = rusty_serve::Server::bind("127.0.0.1:0".parse().unwrap(), backend).unwrap();
        let addr = server.local_addr().unwrap();
        let stop = server.shutdown_handle().unwrap();
        std::thread::spawn(move || server.run().unwrap());
        let url = format!("http://{addr}/api/agent");

        let refused = HttpAgent::new(&url).unwrap();
        let input = RunAgentInput::new("t", "r", vec![Message::user("u", "help")]);
        assert!(matches!(
            refused.run(&input),
            Err(rusty_agui::Error::Status { status: 401, .. })
        ));

        let agent = HttpAgent::new(&url)
            .unwrap()
            .header("Authorization", &format!("Bearer {token}"));
        let mut input = RunAgentInput::new("t", "r", vec![Message::user("u", "add Buy milk")]);
        input.tools.push(Tool {
            name: crate::assistant::CREATE_TASK.into(),
            description: "Add a task".into(),
            parameters: json!({"type": "object"}),
        });
        let events: Vec<_> = agent
            .run(&input)
            .unwrap()
            .map(Result::unwrap)
            .map(|e| e.kind)
            .collect();
        assert!(events.iter().any(|k| matches!(
            k,
            EventKind::ToolCallStart { tool_call_name, .. } if tool_call_name == "create_task"
        )));
        assert!(matches!(events.last(), Some(EventKind::RunFinished { .. })));
        stop.shutdown();
    }

    /// One request over a fresh connection; the status and body of the reply.
    fn post(
        addr: std::net::SocketAddr,
        path: &str,
        token: Option<&str>,
        body: &str,
    ) -> (u16, String) {
        use std::io::{Read, Write};
        let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: x\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let mut sock = std::net::TcpStream::connect(addr).unwrap();
        sock.write_all(request.as_bytes()).unwrap();
        let mut reply = String::new();
        sock.read_to_string(&mut reply).unwrap();
        let status = reply.split(' ').nth(1).unwrap().parse().unwrap();
        (
            status,
            reply.split("\r\n\r\n").nth(1).unwrap_or("").to_string(),
        )
    }

    #[test]
    fn fetch_ics_needs_a_token_and_refuses_unsafe_targets_without_connecting() {
        let dir = tempfile::tempdir().unwrap();
        let token = "a-token-long-enough-to-pass";
        let backend = Backend::single(dir.path(), token.into(), clock()).unwrap();
        let server = rusty_serve::Server::bind("127.0.0.1:0".parse().unwrap(), backend).unwrap();
        let addr = server.local_addr().unwrap();
        let stop = server.shutdown_handle().unwrap();
        std::thread::spawn(move || server.run().unwrap());
        let path = FETCH_ICS_PATH;

        assert_eq!(
            post(addr, path, None, r#"{"url":"https://example.com/a.ics"}"#).0,
            401
        );
        assert_eq!(
            post(addr, path, Some("wrong-token-wrong-token"), "{}").0,
            401
        );
        assert_eq!(post(addr, path, Some(token), "not json").0, 400);
        assert_eq!(
            post(addr, path, Some(token), r#"{"url":"https://x","extra":1}"#).0,
            400
        );
        for url in [
            "http://example.com/a.ics",
            "https://example.com:8443/a.ics",
            "https://localhost/a.ics",
        ] {
            let (status, body) = post(addr, path, Some(token), &format!(r#"{{"url":"{url}"}}"#));
            assert_eq!(status, 422, "{url}: {body}");
            assert!(body.contains(r#""code":"invalid""#), "{body}");
        }
        stop.shutdown();
    }
}
