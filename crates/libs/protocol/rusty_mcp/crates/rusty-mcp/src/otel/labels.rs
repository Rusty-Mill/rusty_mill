//! What gets recorded, with no I/O and no metrics backend.
//!
//! Which methods and names may become labels, how a status becomes an outcome,
//! and which attributes each instrument carries are pure decisions, and the
//! ones that matter most: they are the cardinality guard (see
//! [`super::metrics`]). Nothing here imports `opentelemetry`, `tower`, `axum`,
//! `tokio` or the external `http` crate: statuses are `rusty_http`'s and header
//! values arrive as plain text. The adapter turns these labels into `KeyValue`s and does the
//! recording.

use std::collections::BTreeSet;

use rusty_http::StatusCode;

/// The `Mcp-Method` header (SEP-2243).
pub const HEADER_MCP_METHOD: &str = "mcp-method";
/// The `Mcp-Name` header (SEP-2243).
pub const HEADER_MCP_NAME: &str = "mcp-name";

/// `rmcp` wraps a header value that will not survive as-is in these sentinels.
const BASE64_PREFIX: &str = "=?base64?";
const BASE64_SUFFIX: &str = "?=";

/// Label used whenever a value is not in its allow-list.
const OTHER: &str = "other";

/// Every method the spec defines, so an unknown one cannot mint a label.
const KNOWN_METHODS: &[&str] = &[
    "completion/complete",
    "initialize",
    "logging/setLevel",
    "notifications/cancelled",
    "notifications/initialized",
    "notifications/progress",
    "notifications/roots/list_changed",
    "ping",
    "prompts/get",
    "prompts/list",
    "resources/list",
    "resources/read",
    "resources/subscribe",
    "resources/templates/list",
    "resources/unsubscribe",
    "roots/list",
    "server/discover",
    "subscriptions/listen",
    "tasks/cancel",
    "tasks/get",
    "tasks/update",
    "tools/call",
    "tools/list",
];

/// Methods whose `Mcp-Name` is a bounded identifier rather than free text.
///
/// `resources/read` is deliberately absent: its name is the URI, which a client
/// chooses. Task methods are absent for the same reason — a task id is unique
/// per task, which is the worst possible label.
const NAMED_METHODS: &[&str] = &["tools/call", "prompts/get"];

/// An attribute: a fixed key and a value from a closed set.
pub(crate) type Label = (&'static str, String);

/// How a request ended, as far as the transport can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// A 2xx response.
    Ok,
    /// A 401 or 403 — separated out because an authorization problem is a
    /// different page in the middle of the night than a malformed request.
    Unauthorized,
    /// Any other 4xx.
    ClientError,
    /// A 5xx.
    ServerError,
}

impl Outcome {
    pub(crate) fn from_status(status: StatusCode) -> Self {
        match status.as_u16() {
            200..=299 => Self::Ok,
            401 | 403 => Self::Unauthorized,
            400..=499 => Self::ClientError,
            _ => Self::ServerError,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Unauthorized => "unauthorized",
            Self::ClientError => "client_error",
            Self::ServerError => "server_error",
        }
    }
}

/// How a task ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskOutcome {
    /// Ran to completion and produced a result.
    Completed,
    /// Settled as cancelled, because its body cooperated with a cancel request.
    Cancelled,
    /// Failed with an error.
    Failed,
    /// Still running when the drain grace period expired, and aborted.
    ///
    /// Worth an alert of its own: a client is polling a task id that will never
    /// produce a result.
    Abandoned,
}

impl TaskOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Abandoned => "abandoned",
        }
    }
}

/// Map a method to itself if the spec defines it, and to `other` if not.
pub(crate) fn method_label(method: &str) -> &'static str {
    KNOWN_METHODS
        .iter()
        .find(|known| **known == method)
        .copied()
        .unwrap_or(OTHER)
}

/// The tool and prompt names that may appear in the `mcp.name` label.
#[derive(Debug, Clone, Default)]
pub(crate) struct KnownNames(BTreeSet<String>);

impl KnownNames {
    pub(crate) fn new(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self(names.into_iter().map(Into::into).collect())
    }

    /// The `mcp.name` label for a request, if one is safe to record.
    pub(crate) fn name_label(&self, method: &str, name_header: Option<&str>) -> Option<String> {
        if !NAMED_METHODS.contains(&method) {
            return None;
        }
        let name = decode_header(name_header?)?;
        self.0.contains(&name).then_some(name)
    }
}

/// A header value, decoded if `rmcp` base64-wrapped it. `None` when the
/// wrapper is malformed, so a bad value is dropped rather than recorded raw.
pub(crate) fn decode_header(raw: &str) -> Option<String> {
    let Some(encoded) = raw
        .strip_prefix(BASE64_PREFIX)
        .and_then(|rest| rest.strip_suffix(BASE64_SUFFIX))
    else {
        return Some(raw.to_string());
    };

    let bytes = rusty_base64::decode_standard(encoded).ok()?;
    String::from_utf8(bytes).ok()
}

/// Attributes of the in-flight gauge: the method only.
pub(crate) fn in_flight_labels(method: &str) -> Vec<Label> {
    vec![("mcp.method", method_label(method).to_owned())]
}

/// Attributes of the duration histogram: method and name, never the outcome.
pub(crate) fn duration_labels(method: &str, name: Option<&str>) -> Vec<Label> {
    vec![
        ("mcp.method", method_label(method).to_owned()),
        ("mcp.name", name.unwrap_or(OTHER).to_owned()),
    ]
}

/// Attributes of the request counter: those of the histogram plus the outcome.
pub(crate) fn request_labels(method: &str, name: Option<&str>, outcome: Outcome) -> Vec<Label> {
    let mut labels = duration_labels(method, name);
    labels.push(("mcp.outcome", outcome.as_str().to_owned()));
    labels
}

/// Attributes of the tasks-finished counter.
pub(crate) fn task_labels(outcome: TaskOutcome) -> Vec<Label> {
    vec![("mcp.task.outcome", outcome.as_str().to_owned())]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(known: &[&str]) -> KnownNames {
        KnownNames::new(known.iter().copied())
    }

    #[test]
    fn a_known_method_keeps_its_name() {
        assert_eq!(method_label("tools/call"), "tools/call");
        assert_eq!(method_label("resources/read"), "resources/read");
    }

    #[test]
    fn an_unknown_method_cannot_mint_a_label() {
        // Otherwise anyone who can reach the endpoint owns your label space.
        assert_eq!(method_label("tools/../../etc/passwd"), OTHER);
        assert_eq!(method_label(""), OTHER);
        assert_eq!(method_label("a-method-invented-by-a-client"), OTHER);
    }

    #[test]
    fn a_known_tool_name_is_labelled() {
        let name = Some("add");
        assert_eq!(
            names(&["add", "divide"])
                .name_label("tools/call", name)
                .as_deref(),
            Some("add")
        );
    }

    #[test]
    fn an_unknown_tool_name_is_dropped() {
        // The call will fail, but the label would already have been recorded.
        let name = Some("a1b2c3d4");
        assert_eq!(names(&["add"]).name_label("tools/call", name), None);
    }

    #[test]
    fn a_resource_uri_is_never_labelled() {
        // `Mcp-Name` for `resources/read` is the URI. Unbounded by definition,
        // and the single easiest way to take a metrics backend down.
        let name = Some("db://tables/users");
        assert_eq!(
            names(&["db://tables/users"]).name_label("resources/read", name),
            None
        );
    }

    #[test]
    fn a_task_id_is_never_labelled() {
        let name = Some("task-1234");
        assert_eq!(names(&["task-1234"]).name_label("tasks/get", name), None);
    }

    #[test]
    fn a_base64_wrapped_header_is_decoded() {
        let encoded = rusty_base64::encode_standard(b"a prompt");
        let wrapped = format!("{BASE64_PREFIX}{encoded}{BASE64_SUFFIX}");
        let name = Some(wrapped.as_str());

        assert_eq!(
            names(&["a prompt"])
                .name_label("prompts/get", name)
                .as_deref(),
            Some("a prompt")
        );
    }

    #[test]
    fn a_malformed_base64_header_is_dropped_rather_than_recorded_raw() {
        let wrapped = format!("{BASE64_PREFIX}!!!not-base64!!!{BASE64_SUFFIX}");
        let name = Some(wrapped.as_str());
        assert_eq!(names(&["add"]).name_label("tools/call", name), None);
    }

    #[test]
    fn a_missing_name_header_is_not_an_error() {
        let name = None;
        assert_eq!(names(&["add"]).name_label("tools/call", name), None);
    }

    #[test]
    fn outcomes_map_from_status() {
        let cases = [
            (StatusCode::OK, Outcome::Ok),
            (StatusCode::ACCEPTED, Outcome::Ok),
            (StatusCode::UNAUTHORIZED, Outcome::Unauthorized),
            (StatusCode::FORBIDDEN, Outcome::Unauthorized),
            (StatusCode::PAYLOAD_TOO_LARGE, Outcome::ClientError),
            (StatusCode::INTERNAL_SERVER_ERROR, Outcome::ServerError),
        ];
        for (status, outcome) in cases {
            assert_eq!(Outcome::from_status(status), outcome, "{status}");
        }
    }

    fn keys(labels: &[Label]) -> Vec<&'static str> {
        labels.iter().map(|(key, _)| *key).collect()
    }

    #[test]
    fn each_instrument_carries_its_own_closed_attribute_set() {
        // The outcome is on the counter and nowhere else: on the histogram it
        // would multiply every bucket by the outcome count.
        assert_eq!(keys(&in_flight_labels("ping")), ["mcp.method"]);
        assert_eq!(
            keys(&duration_labels("ping", None)),
            ["mcp.method", "mcp.name"]
        );
        assert_eq!(
            keys(&request_labels("ping", None, Outcome::Ok)),
            ["mcp.method", "mcp.name", "mcp.outcome"]
        );
        assert_eq!(
            keys(&task_labels(TaskOutcome::Failed)),
            ["mcp.task.outcome"]
        );
    }

    #[test]
    fn labels_are_bounded_whatever_the_input() {
        let labels = request_labels("made/up", Some("whatever"), Outcome::ServerError);
        // The caller passes only a name it already checked; the method is
        // re-checked here, so an unknown one can never reach a backend.
        assert_eq!(labels[0], ("mcp.method", OTHER.to_owned()));
        assert_eq!(in_flight_labels("made/up")[0].1, OTHER);
        assert_eq!(labels[2], ("mcp.outcome", "server_error".to_owned()));
    }

    #[test]
    fn a_missing_name_is_labelled_other() {
        assert_eq!(duration_labels("tools/call", None)[1].1, OTHER);
        assert_eq!(duration_labels("tools/call", Some("add"))[1].1, "add");
    }

    #[test]
    fn every_task_outcome_has_its_own_value() {
        let all = [
            TaskOutcome::Completed,
            TaskOutcome::Cancelled,
            TaskOutcome::Failed,
            TaskOutcome::Abandoned,
        ];
        let values: BTreeSet<_> = all.iter().map(|o| o.as_str()).collect();
        assert_eq!(values.len(), all.len());
    }
}
