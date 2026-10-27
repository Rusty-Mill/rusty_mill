//! An MCP server over the task API, so an agent (Claude Code, say) can read
//! and change tasks. It is served at `/mcp` (Streamable HTTP) by the same
//! process and under the same bearer tokens as `/api`: the data directory is
//! locked by the running server, so a separate process could not open it.
//!
//! Each tool is a call to an existing route, made in-process through
//! [`Backend::handle`] with the caller's own `Authorization` header. The
//! routes' validation, per-user stores and error bodies are reused as they
//! are; a tool only names the route and passes its arguments through. Tool
//! names are shared with the web assistant's tools (`crate::assistant`).

use crate::api;
use crate::assistant::{
    ADD_TASK, COMPLETE_TASK, DELETE_TASK, DUE_TASKS, LIST_LISTS, REOPEN_TASK, SEARCH_TASKS,
    UPDATE_TASK,
};
use crate::backend::Backend;
use crate::lists::INBOX_ID;
use crate::oauth::OAuth;
use rusty_http::Method;
use rusty_json::{json, Value};
use rusty_mcp_server::proto::{CallToolParams, CallToolResult, ContentBlock, Tool};
use rusty_mcp_server::{CallContext, HttpConfig, HttpHandler, Server};
use rusty_oauth_server::Endpoints;
use rusty_serve::{Handler, Request, Response, SharedHandler};
use rusty_url::form_urlencoded::byte_serialize;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Where MCP is served.
pub const MCP_PATH: &str = "/mcp";

/// The whole HTTP surface: `/mcp` and the OAuth endpoints without the
/// backend's lock (a tool call takes it, per call), everything else under
/// it, as before.
pub struct Routes {
    backend: Arc<Mutex<Backend>>,
    mcp: HttpHandler,
    /// The OAuth endpoints, when OAuth is on ([`crate::oauth`]).
    oauth: Option<Endpoints>,
}

impl Routes {
    /// The routes over `backend`, with OAuth when `oauth` is given.
    ///
    /// # Errors
    /// The MCP server refused its tool table (a bug: the table is fixed).
    pub fn new(backend: Backend, oauth: Option<OAuth>) -> std::io::Result<Self> {
        let backend = Arc::new(Mutex::new(backend));
        let oauth = oauth.map(|oauth| crate::oauth::endpoints(oauth, &backend));
        // Any `Host`: every call carries a bearer token, which a page that
        // rebinds DNS cannot attach. No `Origin` is accepted, so browsers are refused.
        let config = HttpConfig {
            path: MCP_PATH.to_owned(),
            allowed_hosts: Vec::new(),
            ..HttpConfig::default()
        };
        let mcp = HttpHandler::new(Arc::new(server(Arc::clone(&backend))?), config);
        Ok(Self {
            backend,
            mcp,
            oauth,
        })
    }
}

impl SharedHandler for Routes {
    fn handle(&self, request: &Request<'_>) -> Response {
        // Before the lock: the consent page's token check takes it.
        let oauth = self.oauth.as_ref();
        if let Some(response) = oauth.and_then(|e| crate::oauth::handle(e, request)) {
            return response;
        }
        let Ok(mut backend) = self.backend.lock() else {
            return Response::internal_error();
        };
        if request.target.split('?').next() != Some(MCP_PATH) {
            return Handler::handle(&mut *backend, request);
        }
        // Refuse a bad token before MCP sees the request, as `/api` does.
        let denied = backend.authorize(&api_request(
            request.method,
            request.target,
            request.authorization,
            b"",
        ));
        drop(backend);
        match (denied, oauth) {
            (Ok(_), _) => self.mcp.handle_with(request, None),
            // The challenge points a client at the OAuth metadata.
            (Err(_), Some(endpoints)) => crate::oauth::to_serve(endpoints.unauthorized()),
            (Err(denied), None) => Response::json(denied.status, denied.body),
        }
    }
}

fn api_request<'a>(
    method: &'a Method,
    target: &'a str,
    authorization: Option<&'a str>,
    body: &'a [u8],
) -> api::Request<'a> {
    api::Request {
        method,
        target,
        authorization,
        if_match: None,
        if_none_match: None,
        if_task_match: None,
        body,
    }
}

/// The route a tool call becomes, or why its arguments are unusable.
type Route = Result<(Method, String, Option<Value>), String>;

struct Spec {
    name: &'static str,
    description: &'static str,
    schema: Value,
    route: fn(&Value) -> Route,
}

/// The MCP server: one tool per spec, each run through `backend`.
fn server(backend: Arc<Mutex<Backend>>) -> std::io::Result<Server> {
    let builder = Server::builder("rusty_tick", env!("CARGO_PKG_VERSION")).instructions(
        "Tasks in rusty_tick. Times are Unix milliseconds; ids are UUIDs. \
         list_lists gives list ids; add_task without listId adds to the Inbox.",
    );
    let built = specs().into_iter().fold(builder, |builder, spec| {
        let mut tool = Tool::new(spec.name, spec.schema);
        tool.description = Some(spec.description.to_owned());
        let backend = Arc::clone(&backend);
        let route = spec.route;
        builder.tool(tool, move |ctx: &CallContext, call: CallToolParams| {
            Ok(call_route(&backend, ctx, &call, route))
        })
    });
    built
        .build()
        .map_err(|e| std::io::Error::other(format!("rusty_tick's MCP tools are invalid: {e:?}")))
}

/// Run one tool call as an API request and report its answer.
fn call_route(
    backend: &Mutex<Backend>,
    ctx: &CallContext,
    call: &CallToolParams,
    route: fn(&Value) -> Route,
) -> CallToolResult {
    let args = call.arguments.clone().unwrap_or_else(Value::object);
    let (method, target, body) = match route(&args) {
        Ok(route) => route,
        Err(message) => return result(true, message),
    };
    let body = body.map(|b| b.to_json_string()).unwrap_or_default();
    let authorization = ctx.caller().header("authorization");
    let Ok(mut backend) = backend.lock() else {
        return result(true, "the server is unavailable".into());
    };
    let response = backend.handle(&api_request(
        &method,
        &target,
        authorization,
        body.as_bytes(),
    ));
    let text = String::from_utf8_lossy(&response.body).into_owned();
    match (response.status.is_success(), text.is_empty()) {
        (true, true) => result(false, "done".into()),
        (ok, _) => result(!ok, text),
    }
}

fn result(is_error: bool, text: String) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::text(text)],
        is_error: is_error.then_some(true),
        ..CallToolResult::default()
    }
}

// ---- the tools ----------------------------------------------------------

/// `args[key]` as a UUID, so an id can never reshape the path it is put in.
fn id(args: &Value, key: &str) -> Result<Uuid, String> {
    let text = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or(format!("{key} is required"))?;
    Uuid::parse_str(text).map_err(|_| format!("{key} is not a task or list id: {text:?}"))
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn query(value: &str) -> String {
    byte_serialize(value.as_bytes()).collect()
}

/// `args` without `key`: the rest is the body, checked by the route (unknown fields are refused).
fn without(args: &Value, key: &str) -> Value {
    let mut rest = args.clone();
    if let Some(map) = rest.as_object_mut() {
        map.remove(key);
    }
    rest
}

fn on_task(args: &Value, method: Method, suffix: &str) -> Route {
    Ok((
        method,
        format!("/api/v1/tasks/{}{suffix}", id(args, "id")?),
        None,
    ))
}

fn specs() -> Vec<Spec> {
    let by_id = json!({"type": "object", "properties": {"id": {"type": "string", "description": "Task id"}}, "required": ["id"]});
    let task_fields = json!({
        "title": {"type": "string"},
        "notes": {"type": "string"},
        "listId": {"type": "string", "description": "List id (list_lists); default the Inbox"},
        "priority": {"type": "integer", "enum": [0, 1, 3, 5], "description": "0 none, 1 low, 3 medium, 5 high"},
        "startMs": {"type": ["integer", "null"]},
        "dueMs": {"type": ["integer", "null"], "description": "Unix ms; null clears it"},
        "isAllDay": {"type": "boolean"},
        "tags": {"type": "array", "items": {"type": "string"}}
    });
    let mut update_fields = task_fields.clone();
    update_fields.insert("id", json!({"type": "string", "description": "Task id"}));
    vec![
        Spec {
            name: LIST_LISTS,
            description: "All lists, with their ids.",
            schema: json!({"type": "object"}),
            route: |_| Ok((Method::Get, "/api/v1/lists".into(), None)),
        },
        Spec {
            name: "list_tasks",
            description: "The tasks in one list.",
            schema: json!({"type": "object", "properties": {
                "listId": {"type": "string"},
                "status": {"type": "string", "enum": ["open", "done", "wontdo"], "description": "Default open"}
            }, "required": ["listId"]}),
            route: |args| {
                let status = query(text(args, "status").unwrap_or("open"));
                Ok((
                    Method::Get,
                    format!(
                        "/api/v1/lists/{}/tasks?status={status}",
                        id(args, "listId")?
                    ),
                    None,
                ))
            },
        },
        Spec {
            name: DUE_TASKS,
            description: "Open tasks due today, in the next 7 days, or overdue.",
            schema: json!({"type": "object", "properties": {
                "view": {"type": "string", "enum": ["today", "next7", "overdue"]},
                "utcOffsetMin": {"type": "integer", "description": "The user's offset from UTC, for where days begin; default 0"}
            }, "required": ["view"]}),
            route: |args| {
                let view = text(args, "view")
                    .filter(|v| ["today", "next7", "overdue"].contains(v))
                    .ok_or("view is one of today, next7, overdue")?;
                let offset = args
                    .get("utcOffsetMin")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                Ok((
                    Method::Get,
                    format!("/api/v1/smart/{view}?utcOffsetMin={offset}"),
                    None,
                ))
            },
        },
        Spec {
            name: SEARCH_TASKS,
            description: "Tasks whose title or notes contain a word starting with the query.",
            schema: json!({"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}),
            route: |args| {
                let q = text(args, "query").ok_or("query is required")?;
                Ok((Method::Get, format!("/api/v1/search?q={}", query(q)), None))
            },
        },
        Spec {
            name: "get_task",
            description: "One task, in full.",
            schema: by_id.clone(),
            route: |args| on_task(args, Method::Get, ""),
        },
        Spec {
            name: ADD_TASK,
            description: "Add a task.",
            schema: json!({"type": "object", "properties": task_fields, "required": ["title"]}),
            route: |args| {
                let mut body = args.clone();
                if body.get("listId").is_none() {
                    body.insert("listId", INBOX_ID.to_string());
                }
                Ok((Method::Post, "/api/v1/tasks".into(), Some(body)))
            },
        },
        Spec {
            name: UPDATE_TASK,
            description: "Change a task's fields; those left out are kept.",
            schema: json!({"type": "object", "properties": update_fields, "required": ["id"]}),
            route: |args| {
                Ok((
                    Method::Patch,
                    format!("/api/v1/tasks/{}", id(args, "id")?),
                    Some(without(args, "id")),
                ))
            },
        },
        Spec {
            name: COMPLETE_TASK,
            description: "Mark a task done (a repeating one moves to its next date).",
            schema: by_id.clone(),
            route: |args| on_task(args, Method::Post, "/complete"),
        },
        Spec {
            name: REOPEN_TASK,
            description: "Mark a done task open again.",
            schema: by_id.clone(),
            route: |args| on_task(args, Method::Post, "/reopen"),
        },
        Spec {
            name: DELETE_TASK,
            description: "Move a task to the Trash (it can be restored in the app).",
            schema: by_id,
            route: |args| on_task(args, Method::Delete, ""),
        },
    ]
}
