#![allow(clippy::unwrap_used)]
//! A server that refuses a bare `server/discover` with `-32602` (as `rmcp`
//! does) is asked again with `_meta` before the client gives up on stateless.

use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::proto::{ErrorCode, ErrorData, Message, ProtocolVersion};
use rusty_mcp_client_native::{Client, ClientConfig, NoHandler, Recv, Transport};
use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
                let body = r#"{"resultType":"complete","supportedVersions":["2025-11-25","2026-07-28"],"capabilities":{"tools":{}}}"#;
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
fn a_bare_discovery_refused_as_invalid_params_is_retried_with_meta() {
    let (client, _) = connect(false);
    assert_eq!(
        client.session().negotiated().unwrap().as_str(),
        "2026-07-28"
    );
}

#[test]
fn the_retry_names_the_revision_to_the_transport_and_keeps_it() {
    let (_, told) = connect(false);
    // Set for the retry, announced again once settled; never cleared.
    assert!(!told.is_empty());
    assert!(
        told.iter().all(|v| v.as_deref() == Some("2026-07-28")),
        "{told:?}"
    );
}

#[test]
fn when_the_retry_finds_no_discovery_the_client_falls_back_and_unsets_the_revision() {
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
