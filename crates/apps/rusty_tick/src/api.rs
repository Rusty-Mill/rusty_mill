//! The HTTP API as a pure function from request to response.
//!
//! No sockets here: [`Api::handle`] takes a parsed request and a [`Service`]
//! and returns a [`Response`], so every route is testable without a network
//! and [`crate::server`] stays a thin adapter.
//!
//! JSON, bearer-token auth except `/health`. Under `/api/v1`:
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | `/snapshot` | lists, tasks (trash and completed included), tags: one boot read |
//! | GET, POST | `/lists` | POST accepts a client-chosen `id` |
//! | GET, PATCH, DELETE | `/lists/{id}` | delete sends its tasks to the trash; the Inbox is permanent |
//! | GET | `/lists/{id}/tasks?status=open\|done\|wontdo&sort=manual\|due` | |
//! | POST | `/tasks` | accepts a client-chosen `id` |
//! | GET, PATCH, DELETE | `/tasks/{id}` | DELETE trashes; `?permanent=true` purges |
//! | POST | `/tasks/{id}/complete`, `/reopen`, `/restore` | |
//! | PUT | `/tasks/{id}/order` | `{"sortOrder": n}` |
//! | DELETE | `/trash` | purge everything in the trash |
//! | GET, POST | `/tags` | |
//! | PATCH, DELETE | `/tags/{name}` | |
//! | POST | `/tags/{name}/rename` | `{"label"}` renames on every task |
//! | GET | `/tags/{name}/tasks` | |
//! | GET | `/search?q=` | whole-word match on title and notes |
//! | GET | `/smart/{today,next7,overdue}?utcOffsetMin=` | |
//! | GET | `/docs/{kind}` | client-owned JSON: habits, focus records, prefs |
//! | PUT, DELETE | `/docs/{kind}/{id}` | |
//!
//! Writes accept `If-Match: "<etag>"`; a stale one is refused with 412 and
//! the current entity in `current`.

use crate::auth::{Authenticator, Denied};
use crate::dto::{
    CreateList, CreateTag, CreateTask, DocDto, Docs, ListDto, Lists, PatchList, PatchTag,
    PatchTask, Purged, RenameTag, SetOrder, SnapshotDto, TagDto, Tags, TaskDto, Tasks,
};
use crate::service::{
    ListOrderBy, ListPatch, NewList, NewTask, Service, ServiceError, TagPatch, TaskPatch,
};
use crate::task::{Priority, Status};
use crate::users::{RegistryFile, UserKey};
use rusty_http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::auth::MIN_TOKEN_LEN;
const MAX_UTC_OFFSET_MIN: i32 = 14 * 60;

/// A parsed request, as the router sees it.
pub struct Request<'a> {
    pub method: &'a Method,
    /// Origin-form target: path and optional `?query`.
    pub target: &'a str,
    pub authorization: Option<&'a str>,
    pub if_match: Option<&'a str>,
    pub body: &'a [u8],
}

pub struct Response {
    pub status: StatusCode,
    /// A JSON document, empty for 204.
    pub body: Vec<u8>,
}

impl Response {
    fn json<T: Serialize>(status: StatusCode, value: &T) -> Self {
        match rusty_json::to_string(value) {
            Ok(text) => Self {
                status,
                body: text.into_bytes(),
            },
            Err(_) => Self::error(&ApiError::Internal),
        }
    }

    fn empty(status: StatusCode) -> Self {
        Self {
            status,
            body: Vec::new(),
        }
    }

    /// A generic 500, for the transport layer when the service is unusable.
    pub fn internal_error() -> Self {
        Self::error(&ApiError::Internal)
    }

    /// 401, whatever the reason: a refusal says nothing about why.
    pub fn unauthorized() -> Self {
        Self::error(&ApiError::Unauthorized)
    }

    /// 503: the caller's data is busy elsewhere; try again.
    pub fn unavailable() -> Self {
        Self::error(&ApiError::Unavailable)
    }

    fn error(error: &ApiError) -> Self {
        #[derive(Serialize)]
        struct Body<'a> {
            error: Inner<'a>,
        }
        #[derive(Serialize)]
        struct Inner<'a> {
            code: &'a str,
            message: &'a str,
        }
        let body = Body {
            error: Inner {
                code: error.code(),
                message: &error.message(),
            },
        };
        let text = rusty_json::to_string(&body).unwrap_or_else(|_| "{}".to_string());
        Self {
            status: error.status(),
            body: text.into_bytes(),
        }
    }
}

#[derive(Debug)]
enum ApiError {
    BadRequest(String),
    Unauthorized,
    NotFound(String),
    Invalid(String),
    Conflict(String),
    Internal,
    Unavailable,
    /// A remote server this one called for the client failed or sent junk.
    Upstream(String),
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::NotFound(_) => "not_found",
            Self::Invalid(_) => "invalid",
            Self::Conflict(_) => "conflict",
            Self::Internal => "internal",
            Self::Unavailable => "unavailable",
            Self::Upstream(_) => "upstream",
        }
    }

    /// Internal errors never leak detail to the client.
    fn message(&self) -> String {
        match self {
            Self::BadRequest(m)
            | Self::NotFound(m)
            | Self::Invalid(m)
            | Self::Conflict(m)
            | Self::Upstream(m) => m.clone(),
            Self::Unauthorized => "missing or invalid bearer token".to_string(),
            Self::Internal => "internal error".to_string(),
            Self::Unavailable => "temporarily unavailable".to_string(),
        }
    }
}

impl From<ServiceError> for ApiError {
    fn from(error: ServiceError) -> Self {
        match error {
            ServiceError::NotFound(what) => Self::NotFound(format!("{what} not found")),
            ServiceError::Invalid(message) => Self::Invalid(message),
            ServiceError::Conflict(message) => Self::Conflict(message),
            ServiceError::Storage(e) => {
                eprintln!("rusty_tick: storage error: {e}");
                Self::Internal
            }
        }
    }
}

type Result<T> = std::result::Result<T, ApiError>;

/// The router plus the way it tells who is asking.
///
/// Serving a request is three steps, run by [`crate::backend::Backend`]:
/// answer what needs no user ([`Api::public`]), find the user
/// ([`Api::authenticate`]), then route within that user's data
/// ([`Api::serve`]).
pub struct Api {
    auth: Authenticator,
}

impl Api {
    /// One shared token, one user. `Err` if `token` is shorter than
    /// [`MIN_TOKEN_LEN`].
    pub fn new(token: String) -> std::result::Result<Self, String> {
        Ok(Self {
            auth: Authenticator::single(token)?,
        })
    }

    /// Tokens per user, checked against `users`.
    pub fn multi_user(users: RegistryFile) -> Self {
        Self {
            auth: Authenticator::multi(users),
        }
    }

    /// The answer to a request that needs no user (`GET /health`), or `None`.
    pub fn public(request: &Request<'_>) -> Option<Response> {
        let (path, _) = split_target(request.target);
        let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
        (request.method == &Method::Get && segments == ["health"])
            .then(|| Response::json(StatusCode::OK, &Health { status: "ok" }))
    }

    /// The user behind the request's credential.
    ///
    /// # Errors
    ///
    /// [`Denied`] when there is none; [`Response::unauthorized`] is its answer.
    pub fn authenticate(&mut self, request: &Request<'_>) -> std::result::Result<UserKey, Denied> {
        self.auth.authenticate(request.authorization)
    }

    /// `POST /api/v1/fetch-ics {"url"}`: the calendar feed at `url`, as
    /// `{"text"}`. It touches the network, not the user's data, so it is
    /// answered by the server adapter after authentication rather than by
    /// [`Api::serve`].
    pub fn fetch_ics(body: &[u8]) -> Response {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            url: String,
        }
        #[derive(Serialize)]
        struct Output {
            text: String,
        }
        let result = rusty_json::from_slice::<Input>(body)
            .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))
            .and_then(|input| {
                crate::fetch::fetch_text(&input.url).map_err(|e| match e {
                    crate::fetch::FetchError::Invalid(m) | crate::fetch::FetchError::Refused(m) => {
                        ApiError::Invalid(m)
                    }
                    crate::fetch::FetchError::Upstream(m) => ApiError::Upstream(m),
                    crate::fetch::FetchError::Busy => ApiError::Unavailable,
                })
            });
        match result {
            Ok(text) => Response::json(StatusCode::OK, &Output { text }),
            Err(error) => Response::error(&error),
        }
    }

    /// Route an authenticated request within one user's data.
    pub fn serve(service: &mut Service, request: &Request<'_>) -> Response {
        let (path, query) = split_target(request.target);
        let segments: Vec<String> = path
            .trim_matches('/')
            .split('/')
            .map(decode_segment)
            .collect();
        let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
        let cx = Cx {
            method: request.method,
            query: &query,
            body: request.body,
            if_match: request.if_match,
        };
        match segments.as_slice() {
            ["api", "v1", rest @ ..] => route(service, &cx, rest),
            _ => Err(not_found()),
        }
        .unwrap_or_else(|error| Response::error(&error))
    }
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

struct Query(Vec<(String, String)>);

impl Query {
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// What a route handler needs besides the service.
struct Cx<'a> {
    method: &'a Method,
    query: &'a Query,
    body: &'a [u8],
    if_match: Option<&'a str>,
}

impl Cx<'_> {
    fn body<T: for<'de> Deserialize<'de>>(&self) -> Result<T> {
        rusty_json::from_slice(self.body)
            .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))
    }

    /// `Some(response)` when `If-Match` is present and does not name `current`.
    fn stale<T: Serialize>(&self, current: &str, entity: impl FnOnce() -> T) -> Option<Response> {
        let presented = self
            .if_match?
            .trim()
            .trim_start_matches("W/")
            .trim_matches('"');
        if presented == current {
            return None;
        }
        #[derive(Serialize)]
        struct Stale<'a, T> {
            error: StaleError<'a>,
            current: T,
        }
        #[derive(Serialize)]
        struct StaleError<'a> {
            code: &'a str,
            message: &'a str,
        }
        Some(Response::json(
            StatusCode::PRECONDITION_FAILED,
            &Stale {
                error: StaleError {
                    code: "precondition_failed",
                    message: "the record changed since you read it",
                },
                current: entity(),
            },
        ))
    }
}

fn split_target(target: &str) -> (&str, Query) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let pairs = rusty_url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    (path, Query(pairs))
}

/// Percent-decode one path segment. `+` is literal in a path, unlike a query.
fn decode_segment(segment: &str) -> String {
    let escaped = segment.replace('+', "%2B");
    rusty_url::form_urlencoded::parse(format!("={escaped}").as_bytes())
        .next()
        .map_or_else(|| segment.to_string(), |(_, value)| value.into_owned())
}

fn route(service: &mut Service, cx: &Cx<'_>, path: &[&str]) -> Result<Response> {
    match path {
        ["snapshot"] if cx.method == &Method::Get => Ok(Response::json(
            StatusCode::OK,
            &SnapshotDto::from(service.snapshot()),
        )),
        ["lists", rest @ ..] => route_lists(service, cx, rest),
        ["tasks", rest @ ..] => route_tasks(service, cx, rest),
        ["tags", rest @ ..] => route_tags(service, cx, rest),
        ["docs", rest @ ..] => route_docs(service, cx, rest),
        ["trash"] if cx.method == &Method::Delete => {
            let purged = service.empty_trash()?;
            Ok(Response::json(StatusCode::OK, &Purged { purged }))
        }
        ["search"] if cx.method == &Method::Get => {
            let q = cx
                .query
                .get("q")
                .ok_or_else(|| ApiError::BadRequest("q is required".into()))?;
            Ok(Response::json(
                StatusCode::OK,
                &Tasks::new(service.search(q)),
            ))
        }
        ["smart", which] if cx.method == &Method::Get => {
            let offset = utc_offset(cx.query)?;
            let tasks = match *which {
                "today" => service.today(offset),
                "next7" => service.next_7_days(offset),
                "overdue" => service.overdue(),
                _ => return Err(not_found()),
            };
            Ok(Response::json(StatusCode::OK, &Tasks::new(tasks)))
        }
        _ => Err(not_found()),
    }
}

fn route_lists(service: &mut Service, cx: &Cx<'_>, path: &[&str]) -> Result<Response> {
    match (cx.method, path) {
        (Method::Get, []) => {
            let lists = service.lists().into_iter().map(ListDto::from).collect();
            Ok(Response::json(StatusCode::OK, &Lists { lists }))
        }
        (Method::Post, []) => {
            let input: CreateList = cx.body()?;
            let list = service.create_list(NewList {
                id: input.id,
                name: input.name,
                color: input.color,
            })?;
            Ok(Response::json(StatusCode::CREATED, &ListDto::from(list)))
        }
        (Method::Get, [id]) => {
            let list = service.list(parse_id(id)?)?;
            Ok(Response::json(StatusCode::OK, &ListDto::from(list)))
        }
        (Method::Patch, [id]) => {
            let id = parse_id(id)?;
            let current = service.list(id)?;
            if let Some(stale) = cx.stale(&current.version.to_string(), || {
                ListDto::from(current.clone())
            }) {
                return Ok(stale);
            }
            let input: PatchList = cx.body()?;
            let list = service.patch_list(
                id,
                ListPatch {
                    name: input.name,
                    color: input.color,
                    archived: input.archived,
                    view_mode: input.view_mode,
                    sort_type: input.sort_type,
                    sort_order: input.sort_order,
                },
            )?;
            Ok(Response::json(StatusCode::OK, &ListDto::from(list)))
        }
        (Method::Delete, [id]) => {
            service.delete_list(parse_id(id)?)?;
            Ok(Response::empty(StatusCode::NO_CONTENT))
        }
        (Method::Get, [id, "tasks"]) => {
            let status = match cx.query.get("status") {
                None => None,
                Some("open") => Some(Status::Open),
                Some("done") => Some(Status::Done),
                Some("wontdo") => Some(Status::WontDo),
                Some(_) => {
                    return Err(ApiError::BadRequest(
                        "status must be open, done or wontdo".into(),
                    ))
                }
            };
            let order = match cx.query.get("sort") {
                None | Some("manual") => ListOrderBy::Manual,
                Some("due") => ListOrderBy::Due,
                Some(_) => return Err(ApiError::BadRequest("sort must be manual or due".into())),
            };
            let tasks = service.tasks_in_list(parse_id(id)?, status, order)?;
            Ok(Response::json(StatusCode::OK, &Tasks::new(tasks)))
        }
        _ => Err(not_found()),
    }
}

fn route_tasks(service: &mut Service, cx: &Cx<'_>, path: &[&str]) -> Result<Response> {
    match (cx.method, path) {
        (Method::Post, []) => {
            let input: CreateTask = cx.body()?;
            let task = service.create_task(NewTask {
                id: input.id,
                list_id: input.list_id,
                parent_id: input.parent_id,
                title: input.title,
                notes: input.notes,
                kind: input.kind,
                priority: input.priority.map(parse_priority).transpose()?,
                start_ms: input.start_ms,
                due_ms: input.due_ms,
                is_all_day: input.is_all_day,
                time_zone: input.time_zone,
                reminders: input.reminders,
                repeat_flag: input.repeat_flag,
                items: input.items,
                tags: input.tags,
                sort_order: input.sort_order,
            })?;
            Ok(Response::json(StatusCode::CREATED, &TaskDto::from(task)))
        }
        (Method::Get, [id]) => {
            let task = service.task(parse_id(id)?)?;
            Ok(Response::json(StatusCode::OK, &TaskDto::from(task)))
        }
        (Method::Patch, [id]) => {
            let id = parse_id(id)?;
            if let Some(stale) = stale_task(service, cx, id)? {
                return Ok(stale);
            }
            let input: PatchTask = cx.body()?;
            let task = service.patch_task(id, task_patch(input)?)?;
            Ok(Response::json(StatusCode::OK, &TaskDto::from(task)))
        }
        (Method::Delete, [id]) => {
            let id = parse_id(id)?;
            if let Some(stale) = stale_task(service, cx, id)? {
                return Ok(stale);
            }
            match cx.query.get("permanent") {
                Some("true") => service.purge_task(id)?,
                None | Some("false") => {
                    service.trash_task(id)?;
                }
                Some(_) => {
                    return Err(ApiError::BadRequest(
                        "permanent must be true or false".into(),
                    ))
                }
            }
            Ok(Response::empty(StatusCode::NO_CONTENT))
        }
        (Method::Post, [id, "complete"]) => {
            let task = service.set_status(parse_id(id)?, Status::Done)?;
            Ok(Response::json(StatusCode::OK, &TaskDto::from(task)))
        }
        (Method::Post, [id, "reopen"]) => {
            let task = service.set_status(parse_id(id)?, Status::Open)?;
            Ok(Response::json(StatusCode::OK, &TaskDto::from(task)))
        }
        (Method::Post, [id, "restore"]) => {
            let task = service.restore_task(parse_id(id)?)?;
            Ok(Response::json(StatusCode::OK, &TaskDto::from(task)))
        }
        (Method::Put, [id, "order"]) => {
            let input: SetOrder = cx.body()?;
            let task = service.reorder(parse_id(id)?, input.sort_order)?;
            Ok(Response::json(StatusCode::OK, &TaskDto::from(task)))
        }
        _ => Err(not_found()),
    }
}

/// A 412 response when the request's `If-Match` is stale for task `id`.
fn stale_task(service: &Service, cx: &Cx<'_>, id: Uuid) -> Result<Option<Response>> {
    let current = service.task(id)?;
    Ok(cx.stale(&current.version.to_string(), || {
        TaskDto::from(current.clone())
    }))
}

fn task_patch(input: PatchTask) -> Result<TaskPatch> {
    Ok(TaskPatch {
        title: input.title,
        notes: input.notes,
        kind: input.kind,
        status: input.status,
        priority: input.priority.map(parse_priority).transpose()?,
        start_ms: input.start_ms,
        due_ms: input.due_ms,
        is_all_day: input.is_all_day,
        time_zone: input.time_zone,
        reminders: input.reminders,
        repeat_flag: input.repeat_flag,
        ex_dates: input.ex_dates,
        items: input.items,
        tags: input.tags,
        list_id: input.list_id,
        sort_order: input.sort_order,
    })
}

fn route_tags(service: &mut Service, cx: &Cx<'_>, path: &[&str]) -> Result<Response> {
    match (cx.method, path) {
        (Method::Get, []) => {
            let tags = service.tags().into_iter().map(TagDto::from).collect();
            Ok(Response::json(StatusCode::OK, &Tags { tags }))
        }
        (Method::Post, []) => {
            let input: CreateTag = cx.body()?;
            let tag = service.create_tag(&input.label, input.color)?;
            Ok(Response::json(StatusCode::CREATED, &TagDto::from(tag)))
        }
        (Method::Patch, [name]) => {
            let current = service.tag(name)?;
            if let Some(stale) = cx.stale(&current.version.to_string(), || {
                TagDto::from(current.clone())
            }) {
                return Ok(stale);
            }
            let input: PatchTag = cx.body()?;
            let tag = service.patch_tag(
                name,
                TagPatch {
                    color: input.color,
                    parent: input.parent,
                    sort_order: input.sort_order,
                },
            )?;
            Ok(Response::json(StatusCode::OK, &TagDto::from(tag)))
        }
        (Method::Post, [name, "rename"]) => {
            let input: RenameTag = cx.body()?;
            let tag = service.rename_tag(name, &input.label)?;
            Ok(Response::json(StatusCode::OK, &TagDto::from(tag)))
        }
        (Method::Delete, [name]) => {
            service.delete_tag(name)?;
            Ok(Response::empty(StatusCode::NO_CONTENT))
        }
        (Method::Get, [name, "tasks"]) => Ok(Response::json(
            StatusCode::OK,
            &Tasks::new(service.with_tag(name)),
        )),
        _ => Err(not_found()),
    }
}

fn route_docs(service: &mut Service, cx: &Cx<'_>, path: &[&str]) -> Result<Response> {
    match (cx.method, path) {
        (Method::Get, [kind]) => {
            let docs = service
                .docs(kind)?
                .into_iter()
                .filter_map(DocDto::from_doc)
                .collect();
            Ok(Response::json(StatusCode::OK, &Docs { docs }))
        }
        (Method::Put, [kind, id]) => {
            let body = std::str::from_utf8(cx.body)
                .map_err(|_| ApiError::BadRequest("body must be UTF-8 JSON".into()))?;
            let doc = service.put_doc(kind, parse_id(id)?, body)?;
            let dto = DocDto::from_doc(doc).ok_or(ApiError::Internal)?;
            Ok(Response::json(StatusCode::OK, &dto))
        }
        (Method::Delete, [kind, id]) => {
            service.delete_doc(kind, parse_id(id)?)?;
            Ok(Response::empty(StatusCode::NO_CONTENT))
        }
        _ => Err(not_found()),
    }
}

fn not_found() -> ApiError {
    ApiError::NotFound("no such route".into())
}

fn parse_id(text: &str) -> Result<Uuid> {
    Uuid::parse_str(text).map_err(|_| ApiError::BadRequest("malformed id".into()))
}

fn parse_priority(value: u8) -> Result<Priority> {
    Priority::from_u8(value)
        .ok_or_else(|| ApiError::Invalid("priority must be 0, 1, 3 or 5".into()))
}

fn utc_offset(query: &Query) -> Result<i32> {
    let Some(text) = query.get("utcOffsetMin") else {
        return Ok(0);
    };
    match text.parse::<i32>() {
        Ok(v) if v.abs() <= MAX_UTC_OFFSET_MIN => Ok(v),
        _ => Err(ApiError::BadRequest(
            "utcOffsetMin must be within ±840".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::decode_segment;

    #[test]
    fn path_segments_decode_percent_escapes_but_keep_plus() {
        assert_eq!(decode_segment("plain"), "plain");
        assert_eq!(decode_segment("buy%20milk"), "buy milk");
        assert_eq!(decode_segment("c%2B%2B"), "c++");
        assert_eq!(decode_segment("c++"), "c++", "+ is literal in a path");
        assert_eq!(decode_segment("caf%C3%A9"), "café");
        assert_eq!(decode_segment("a%2Fb"), "a/b");
    }
}
