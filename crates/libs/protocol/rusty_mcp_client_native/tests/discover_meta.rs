#![allow(clippy::unwrap_used)]
//! `server/discover` carries `_meta` (an `rmcp` server refuses a bare one, and
//! over stdio then hangs up), and the transport is told the revision being
//! tried (an HTTP server checks its header against `_meta`).

use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::proto::{ErrorCode, ErrorData, Message, ProtocolVersion};
use rusty_mcp_client_native::{Client, ClientConfig, NoHandler, Recv, Transport};
use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A scripted server: answers only a discovery that carries `_meta`.
#[derive(Default)]
struct Picky {
    out: VecDeque<Message>,
    /// The revisions the transport was told, `None` for a clear.
    versions: Arc<Mutex<Vec<Option<String>>>>,
    /// How the server treats a discovery with `_meta`.
    refuse_everything: bool,
}

impl Transport for Picky {
    fn send(&mut self, message: &Message) -> io::Result<()> {
        let Message::Request { id, method, params } = message else {
            return Ok(());
        };
        let has_meta = params.as_ref().is_some_and(|p| p.get("_meta").is_some());
        let reply = match (method.as_str(), has_meta) {
            ("server/discover", true) if !self.refuse_everything => {
                let body = r#"{"resultType":"complete","supportedVersions":["2025-11-25","2026-07-28"],"capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private"}"#;
                Message::Response {
                    id: id.clone(),
                    result: Value::from_json_str(body).unwrap(),
                }
            }
            ("server/discover", false) => Message::error(
                Some(id.clone()),
                ErrorData::new(ErrorCode::INVALID_PARAMS, "request _meta is missing"),
            ),
            ("server/discover", true) => Message::error(
                Some(id.clone()),
                ErrorData::new(ErrorCode::METHOD_NOT_FOUND, "no discovery here"),
            ),
            ("initialize", _) => Message::Response {
                id: id.clone(),
                result: Value::from_json_str(
                    r#"{"protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"s","version":"1"}}"#,
                )
                .unwrap(),
            },
            _ => return Ok(()),
        };
        self.out.push_back(reply);
        Ok(())
    }

    fn recv(&mut self, _timeout: Duration) -> io::Result<Recv> {
        Ok(self.out.pop_front().map_or(Recv::Timeout, Recv::Message))
    }

    fn set_protocol_version(&mut self, version: &ProtocolVersion) {
        self.versions
            .lock()
            .unwrap()
            .push(Some(version.as_str().to_owned()));
    }

    fn clear_protocol_version(&mut self) {
        self.versions.lock().unwrap().push(None);
    }
}

fn connect(refuse_everything: bool) -> (Client<Picky, NoHandler>, Vec<Option<String>>) {
    let server = Picky {
        refuse_everything,
        ..Picky::default()
    };
    let log = Arc::clone(&server.versions);
    let client = Client::connect(
        server,
        ClientConfig::new("c", "1"),
        NoHandler,
        Duration::from_secs(2),
    )
    .unwrap();
    let told = log.lock().unwrap().clone();
    (client, told)
}

#[test]
fn a_discovery_that_carries_meta_settles_the_stateless_revision() {
    let (client, _) = connect(false);
    assert_eq!(
        client.session().negotiated().unwrap().as_str(),
        "2026-07-28"
    );
}

#[test]
fn the_transport_is_told_the_revision_tried_and_keeps_it() {
    let (_, told) = connect(false);
    // Set for the discovery, announced again once settled; never cleared.
    assert!(!told.is_empty());
    assert!(
        told.iter().all(|v| v.as_deref() == Some("2026-07-28")),
        "{told:?}"
    );
}

#[test]
fn when_there_is_no_discovery_the_client_falls_back_and_unsets_the_revision() {
    let (client, told) = connect(true);
    assert_eq!(
        client.session().negotiated().unwrap().as_str(),
        "2025-11-25"
    );
    // Tried, cleared for `initialize`, then the classic revision announced.
    assert_eq!(
        told,
        [
            Some("2026-07-28".to_owned()),
            None,
            Some("2025-11-25".to_owned())
        ]
    );
}

/// A server that takes 600 ms to answer anything, and has no discovery.
struct Sluggish {
    ready: VecDeque<(Instant, Message)>,
}

impl Transport for Sluggish {
    fn send(&mut self, message: &Message) -> io::Result<()> {
        let Message::Request { id, method, .. } = message else {
            return Ok(());
        };
        let reply = match method.as_str() {
            "server/discover" => Message::error(
                Some(id.clone()),
                ErrorData::new(ErrorCode::METHOD_NOT_FOUND, "no discovery here"),
            ),
            _ => Message::Response {
                id: id.clone(),
                result: Value::from_json_str(
                    r#"{"protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"s","version":"1"}}"#,
                )
                .unwrap(),
            },
        };
        self.ready
            .push_back((Instant::now() + Duration::from_millis(600), reply));
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> io::Result<Recv> {
        let Some((at, _)) = self.ready.front() else {
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
            return Ok(Recv::Timeout);
        };
        let wait = at.saturating_duration_since(Instant::now());
        if wait > timeout {
            std::thread::sleep(timeout);
            return Ok(Recv::Timeout);
        }
        std::thread::sleep(wait);
        Ok(self
            .ready
            .pop_front()
            .map_or(Recv::Timeout, |(_, m)| Recv::Message(m)))
    }
}

#[test]
fn the_whole_handshake_shares_one_timeout() {
    // 600 ms to discover (refused), 600 ms to initialize: 1.2 s in all, so a
    // one-second budget must run out, and at one second, not after the second
    // step has had a second of its own.
    let started = Instant::now();
    let result = Client::connect(
        Sluggish {
            ready: VecDeque::new(),
        },
        ClientConfig::new("c", "1"),
        NoHandler,
        Duration::from_secs(1),
    );
    let took = started.elapsed();
    assert!(
        matches!(result, Err(rusty_mcp_client_native::ClientError::Timeout)),
        "{:?}",
        result.err()
    );
    assert!(
        took < Duration::from_millis(1_150),
        "the handshake ran past its budget: {took:?}"
    );
}
