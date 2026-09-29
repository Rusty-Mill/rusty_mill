//! The HTTP API as a pure function from request to response.
//!
//! No sockets here: [`Api::handle`] takes a parsed request and a [`Service`]
//! and returns a [`Response`], so every route is testable without a network
//! and [`crate::server`] stays a thin adapter.
//!
//! Routes (JSON, bearer-token auth except `/health`):
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | `/health` | liveness, no auth |
//! | GET, POST | `/api/v1/lists` | |
//! | GET, PATCH, DELETE | `/api/v1/lists/{id}` | delete cascades to its tasks |
//! | GET | `/api/v1/lists/{id}/tasks?status=open\|done&sort=manual\|due` | |
//! | POST | `/api/v1/tasks` | |
//! | GET, PATCH, DELETE | `/api/v1/tasks/{id}` | delete cascades to subtasks |
//! | POST | `/api/v1/tasks/{id}/complete`, `/reopen` | |
//! | PUT | `/api/v1/tasks/{id}/order` | `{"sortOrder": n}` |
//! | GET | `/api/v1/search?q=` | whole-word match on title and notes |
//! | GET | `/api/v1/tags/{tag}/tasks` | |
//! | GET | `/api/v1/smart/{today,next7,overdue}?utcOffsetMin=` | |

use crate::auth::{Authenticator, Denied};
use crate::dto::{
    CreateList, CreateTask, ListDto, Lists, PatchList, PatchTask, SetOrder, TaskDto, Tasks,
};
use crate::service::{ListOrderBy, NewTask, Service, ServiceError, TaskPatch};
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
    Internal,
    Unavailable,
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::NotFound(_) => "not_found",
            Self::Invalid(_) => "invalid",
            Self::Internal => "internal",
            Self::Unavailable => "unavailable",
        }
    }

    /// Internal errors never leak detail to the client.
    fn message(&self) -> String {
        match self {
            Self::BadRequest(m) | Self::NotFound(m) | Self::Invalid(m) => m.clone(),
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

    /// Route an authenticated request within one user's data.
    pub fn serve(service: &mut Service, request: &Request<'_>) -> Response {
        let (path, query) = split_target(request.target);
        let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
        match route(service, request.method, &segments, &query, request.body) {
            Ok(response) => response,
            Err(error) => Response::error(&error),
        }
    }
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

/// Compare without exiting early on the first differing byte.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
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

fn split_target(target: &str) -> (&str, Query) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let pairs = rusty_url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    (path, Query(pairs))
}

fn route(
    service: &mut Service,
    method: &Method,
    segments: &[&str],
    query: &Query,
    body: &[u8],
) -> Result<Response> {
    match (method, segments) {
        (Method::Get, ["api", "v1", "lists"]) => {
            let lists = service.lists().into_iter().map(ListDto::from).collect();
            Ok(Response::json(StatusCode::OK, &Lists { lists }))
        }
        (Method::Post, ["api", "v1", "lists"]) => {
            let input: CreateList = parse_body(body)?;
            let list = service.create_list(&input.name)?;
            Ok(Response::json(StatusCode::CREATED, &ListDto::from(list)))
        }
        (Method::Get, ["api", "v1", "lists", id]) => {
            let list = service.list(parse_id(id)?)?;
            Ok(Response::json(StatusCode::OK, &ListDto::from(list)))
        }
        (Method::Patch, ["api", "v1", "lists", id]) => {
            let input: PatchList = parse_body(body)?;
            let list = service.patch_list(parse_id(id)?, input.name.as_deref(), input.archived)?;
            Ok(Response::json(StatusCode::OK, &ListDto::from(list)))
        }
        (Method::Delete, ["api", "v1", "lists", id]) => {
            service.delete_list(parse_id(id)?)?;
            Ok(Response::empty(StatusCode::NO_CONTENT))
        }
        (Method::Get, ["api", "v1", "lists", id, "tasks"]) => {
            let status = match query.get("status") {
                None => None,
                Some("open") => Some(Status::Open),
                Some("done") => Some(Status::Done),
                Some(_) => return Err(ApiError::BadRequest("status must be open or done".into())),
            };
            let order = match query.get("sort") {
                None | Some("manual") => ListOrderBy::Manual,
                Some("due") => ListOrderBy::Due,
                Some(_) => return Err(ApiError::BadRequest("sort must be manual or due".into())),
            };
            let tasks = service.tasks_in_list(parse_id(id)?, status, order)?;
            Ok(Response::json(StatusCode::OK, &Tasks::new(tasks)))
        }
        (Method::Post, ["api", "v1", "tasks"]) => {
            let input: CreateTask = parse_body(body)?;
            let task = service.create_task(NewTask {
                list_id: input.list_id,
                parent_id: input.parent_id,
                title: input.title,
                notes: input.notes,
                priority: input.priority.map(parse_priority).transpose()?,
                due_ms: input.due_ms,
                tags: input.tags,
            })?;
            Ok(Response::json(StatusCode::CREATED, &TaskDto::from(task)))
        }
        (Method::Get, ["api", "v1", "tasks", id]) => {
            let task = service.task(parse_id(id)?)?;
            Ok(Response::json(StatusCode::OK, &TaskDto::from(task)))
        }
        (Method::Patch, ["api", "v1", "tasks", id]) => {
            let input: PatchTask = parse_body(body)?;
            let task = service.patch_task(
                parse_id(id)?,
                TaskPatch {
                    title: input.title,
                    notes: input.notes,
                    priority: input.priority.map(parse_priority).transpose()?,
                    due_ms: input.due_ms,
                    tags: input.tags,
                },
            )?;
            Ok(Response::json(StatusCode::OK, &TaskDto::from(task)))
        }
        (Method::Delete, ["api", "v1", "tasks", id]) => {
            service.delete_task(parse_id(id)?)?;
            Ok(Response::empty(StatusCode::NO_CONTENT))
        }
        (Method::Post, ["api", "v1", "tasks", id, "complete"]) => {
            let task = service.set_status(parse_id(id)?, Status::Done)?;
            Ok(Response::json(StatusCode::OK, &TaskDto::from(task)))
        }
        (Method::Post, ["api", "v1", "tasks", id, "reopen"]) => {
            let task = service.set_status(parse_id(id)?, Status::Open)?;
            Ok(Response::json(StatusCode::OK, &TaskDto::from(task)))
        }
        (Method::Put, ["api", "v1", "tasks", id, "order"]) => {
            let input: SetOrder = parse_body(body)?;
            let task = service.reorder(parse_id(id)?, input.sort_order)?;
            Ok(Response::json(StatusCode::OK, &TaskDto::from(task)))
        }
        (Method::Get, ["api", "v1", "search"]) => {
            let q = query
                .get("q")
                .ok_or_else(|| ApiError::BadRequest("q is required".into()))?;
            Ok(Response::json(
                StatusCode::OK,
                &Tasks::new(service.search(q)),
            ))
        }
        (Method::Get, ["api", "v1", "tags", tag, "tasks"]) => Ok(Response::json(
            StatusCode::OK,
            &Tasks::new(service.with_tag(tag)),
        )),
        (Method::Get, ["api", "v1", "smart", which]) => {
            let offset = utc_offset(query)?;
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

fn parse_body<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T> {
    rusty_json::from_slice(body)
        .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn constant_time_eq_matches_ordinary_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }
}
