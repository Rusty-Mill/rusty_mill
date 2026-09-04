//! The agent's local HTTP API: a small, deliberately *synchronous* server
//! (`tiny_http`, not `axum`/`tokio` -- there's no real concurrent I/O to
//! exploit for one local admin agent handling occasional requests, the
//! same reasoning `rustils`' own platform crate already applies to keep
//! tokio out of its layer). Requests are handled one at a time on the
//! calling thread; a long-running `dnf install`/`remove` doesn't block
//! this loop because [`crate::dnf::DnfController`] itself runs those on a
//! background thread and returns a task id immediately.
//!
//! Every route returns JSON: `200` with the payload on success, or the
//! error's own [`AgentError::http_status`] with `{"error": "..."}` on
//! failure. No `Content-Type` response header is set -- callers (this
//! workspace's own `rusty_fedora` client) parse the body as JSON
//! regardless, the same passthrough style `rusty_opnsense`/
//! `rusty_proxmox` already use against *their* upstream APIs.

use std::collections::HashMap;
use std::io::Cursor;

use serde::Serialize;
use serde_json::Value;
use tiny_http::{Method, Request, Response};

use crate::config_files::ConfigStore;
use crate::domain::{JournalQuery, Priority, ServiceAction, UnitType};
use crate::error::AgentError;
use crate::ports::{PackageController, SystemController};

/// The three adapters a request handler dispatches to. Generic over
/// `SystemController`/`PackageController` so tests can swap in a mock
/// without a real Fedora box or a real `tiny_http` listener; `main.rs`
/// instantiates this with the real `SystemdAdapter`/`DnfController`.
pub struct AgentState<S, P> {
    pub systemd: S,
    pub dnf: P,
    pub config: ConfigStore,
}

/// Binds `addr` (a private/Tailscale address, never `0.0.0.0` -- see this
/// crate's README) and serves requests until the process exits.
pub fn serve<S, P>(addr: &str, state: AgentState<S, P>) -> std::io::Result<()>
where
    S: SystemController,
    P: PackageController,
{
    let server = tiny_http::Server::http(addr)
        .map_err(|e| std::io::Error::other(format!("failed to bind {addr}: {e}")))?;
    for request in server.incoming_requests() {
        handle(request, &state);
    }
    Ok(())
}

fn handle<S, P>(mut request: Request, state: &AgentState<S, P>)
where
    S: SystemController,
    P: PackageController,
{
    let url = request.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
    let segments: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(percent_decode)
        .collect();
    let seg_refs: Vec<&str> = segments.iter().map(String::as_str).collect();
    let params = parse_query(query);
    let method = request.method().clone();

    let mut body = String::new();
    if matches!(method, Method::Post | Method::Put)
        && request.as_reader().read_to_string(&mut body).is_err()
    {
        respond(request, 400, &error_body("request body was not valid utf-8"));
        return;
    }

    let result = route(state, &method, seg_refs.as_slice(), &params, &body);
    match result {
        Ok(value) => respond(request, 200, &value),
        Err(err) => {
            let status = err.http_status();
            respond(request, status, &error_body(&err.to_string()));
        }
    }
}

fn route<S, P>(
    state: &AgentState<S, P>,
    method: &Method,
    segments: &[&str],
    params: &HashMap<String, String>,
    body: &str,
) -> Result<Value, AgentError>
where
    S: SystemController,
    P: PackageController,
{
    match (method, segments) {
        (Method::Get, ["status"]) => to_value(state.systemd.system_status()?),

        (Method::Get, ["services"]) => {
            let unit_type = match params.get("unit_type") {
                Some(v) => Some(parse_unit_type(v)?),
                None => None,
            };
            to_value(state.systemd.list_services(unit_type)?)
        }

        (Method::Post, ["services", name, "control"]) => {
            let action: ActionBody = parse_json_body(body)?;
            state.systemd.control_service(name, action.action)?;
            Ok(serde_json::json!({}))
        }

        (Method::Get, ["journal"]) => {
            let query = journal_query_from(params)?;
            to_value(state.systemd.read_journal(query)?)
        }

        (Method::Get, ["dnf", "updates"]) => to_value(state.dnf.list_updates()?),

        (Method::Post, ["dnf", "install"]) => {
            let request: PackagesBody = parse_json_body(body)?;
            let task_id = state.dnf.install(&request.packages)?;
            Ok(serde_json::json!({ "task_id": task_id.0 }))
        }

        (Method::Post, ["dnf", "remove"]) => {
            let request: PackagesBody = parse_json_body(body)?;
            let task_id = state.dnf.remove(&request.packages)?;
            Ok(serde_json::json!({ "task_id": task_id.0 }))
        }

        (Method::Get, ["tasks", id]) => {
            to_value(state.dnf.task_status(&crate::domain::TaskId(id.to_string()))?)
        }

        (Method::Get, ["config"]) => {
            let path = params
                .get("path")
                .ok_or_else(|| AgentError::InvalidRequest("missing 'path' query parameter".to_string()))?;
            let content = state.config.read(path)?;
            Ok(serde_json::json!({ "content": content }))
        }

        (Method::Put, ["config"]) => {
            let request: WriteConfigBody = parse_json_body(body)?;
            state
                .config
                .write(&request.path, &request.content, request.backup.unwrap_or(true))?;
            Ok(serde_json::json!({}))
        }

        _ => Err(AgentError::InvalidRequest(format!(
            "no route for {method:?} {}",
            segments.join("/")
        ))),
    }
}

#[derive(serde::Deserialize)]
struct ActionBody {
    action: ServiceAction,
}

#[derive(serde::Deserialize)]
struct PackagesBody {
    packages: Vec<String>,
}

#[derive(serde::Deserialize)]
struct WriteConfigBody {
    path: String,
    content: String,
    #[serde(default)]
    backup: Option<bool>,
}

fn parse_json_body<T: serde::de::DeserializeOwned>(body: &str) -> Result<T, AgentError> {
    serde_json::from_str(body)
        .map_err(|e| AgentError::InvalidRequest(format!("malformed request body: {e}")))
}

fn to_value(payload: impl Serialize) -> Result<Value, AgentError> {
    serde_json::to_value(payload)
        .map_err(|e| AgentError::InvalidRequest(format!("could not encode response: {e}")))
}

fn parse_unit_type(value: &str) -> Result<UnitType, AgentError> {
    match value {
        "service" => Ok(UnitType::Service),
        "timer" => Ok(UnitType::Timer),
        "socket" => Ok(UnitType::Socket),
        other => Err(AgentError::InvalidRequest(format!(
            "unknown unit_type: {other}"
        ))),
    }
}

fn parse_priority(value: &str) -> Result<Priority, AgentError> {
    match value {
        "emerg" => Ok(Priority::Emerg),
        "alert" => Ok(Priority::Alert),
        "crit" => Ok(Priority::Crit),
        "err" => Ok(Priority::Err),
        "warning" => Ok(Priority::Warning),
        "notice" => Ok(Priority::Notice),
        "info" => Ok(Priority::Info),
        "debug" => Ok(Priority::Debug),
        other => Err(AgentError::InvalidRequest(format!(
            "unknown priority: {other}"
        ))),
    }
}

fn journal_query_from(params: &HashMap<String, String>) -> Result<JournalQuery, AgentError> {
    let lines = match params.get("lines") {
        Some(v) => Some(
            v.parse()
                .map_err(|_| AgentError::InvalidRequest(format!("invalid lines: {v}")))?,
        ),
        None => None,
    };
    let priority = match params.get("priority") {
        Some(v) => Some(parse_priority(v)?),
        None => None,
    };
    Ok(JournalQuery {
        unit: params.get("unit").cloned(),
        lines,
        since: params.get("since").cloned(),
        priority,
    })
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|s| !s.is_empty())
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next()?;
            let value = parts.next().unwrap_or("");
            Some((percent_decode(key), percent_decode(value)))
        })
        .collect()
}

/// Minimal `application/x-www-form-urlencoded`-style decode (`%XX` and
/// `+` for space) -- enough for this agent's own query strings (unit
/// names, small integers, ISO-ish timestamps), without adding a URL-
/// parsing dependency for it.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    None => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn error_body(message: &str) -> Value {
    serde_json::json!({ "error": message })
}

fn respond(request: Request, status: u16, body: &Value) {
    let text = serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string());
    let response: Response<Cursor<Vec<u8>>> = Response::from_string(text).with_status_code(status);
    // A write failure here means the peer already went away -- nothing
    // more this handler can do about it, and nothing worth logging for a
    // single local admin request.
    let _ = request.respond(response);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_handles_escapes_and_plus() {
        assert_eq!(percent_decode("ollama.service"), "ollama.service");
        assert_eq!(percent_decode("hello+world"), "hello world");
        assert_eq!(percent_decode("a%2Fb"), "a/b");
    }

    #[test]
    fn percent_decode_leaves_a_trailing_malformed_escape_alone() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("100%2"), "100%2");
    }

    #[test]
    fn parse_query_splits_pairs() {
        let params = parse_query("unit=ollama.service&lines=50");
        assert_eq!(params.get("unit").map(String::as_str), Some("ollama.service"));
        assert_eq!(params.get("lines").map(String::as_str), Some("50"));
    }

    #[test]
    fn parse_query_on_an_empty_string_is_empty() {
        assert!(parse_query("").is_empty());
    }

    #[test]
    fn unknown_unit_type_is_an_invalid_request() {
        assert!(matches!(
            parse_unit_type("mount"),
            Err(AgentError::InvalidRequest(_))
        ));
    }

    #[test]
    fn known_unit_types_parse() {
        assert_eq!(parse_unit_type("service").expect("valid"), UnitType::Service);
        assert_eq!(parse_unit_type("timer").expect("valid"), UnitType::Timer);
        assert_eq!(parse_unit_type("socket").expect("valid"), UnitType::Socket);
    }
}
