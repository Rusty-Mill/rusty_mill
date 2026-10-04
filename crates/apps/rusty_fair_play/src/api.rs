//! The HTTP API as a pure function from request to response.
//!
//! No sockets here: [`Api::handle`] takes a parsed request and returns a
//! [`Response`], so every route is testable without a network and
//! [`crate::server`] stays a thin adapter.
//!
//! JSON. A bearer token is required on `/api` when the server was given
//! one; without a token the binary serves loopback only. Under `/api/v1`:
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | `/snapshot` | people and every card with its state: one boot read |
//! | POST | `/seed` | load the deck that ships in the binary; idempotent |
//! | GET, POST | `/people` | POST `{"name"}` |
//! | GET, PATCH | `/people/{id}` | PATCH `{"name"}` |
//! | GET, PATCH | `/cards/{id}` | PATCH any of the text fields, `notes`, `ownerId`, `parentCardId`, `position`; absent = keep, `null` = clear |
//! | POST | `/cards` | a custom card: `{"name","suit","parentCardId"?,"ownerId"?,…}` |
//! | POST | `/cards/{id}/split` | `{"children":[{"name","ownerId"?,…}],"ownerId"?,"notes"?}` |
//! | POST | `/cards/{id}/reset` | the six text fields back to the baseline |
//! | GET | `/cards/{id}/baseline` | the baseline and a field-by-field diff |
//! | PUT | `/cards/{id}/position` | `{"position": n}`: one durable slot write |
//! | PUT | `/cards/{id}/children/order` | `{"ids":[…]}`: every child once; positions follow the list |
//! | DELETE | `/cards/{id}` | a leaf card; 409 for a split parent |
//! | POST | `/cards/{id}/unsplit` | delete everything under the card, deepest first; the card stays |
//! | DELETE | `/people/{id}` | 409 while the person holds a card |
//!
//! Every card carries an `etag` (the card alone) and a `treeEtag` (the
//! card and everything under it). A write on a card (`PATCH`, `DELETE`,
//! `split`, `reset`, `position`) accepts `If-Match: "<etag>"`; the two
//! subtree writes (`unsplit`, `children/order`) accept
//! `If-Match: "<treeEtag>"`, which moves when any descendant is edited,
//! added, removed or reordered. A mismatch is 412 with the current card
//! in `current`; `*` always matches.
//!
//! Errors are `{"error":{"code","message"}}`: 400 malformed request, 401,
//! 404, 409 conflict, 412 stale, 422 invalid value, 500.

use crate::dto::{
    parse_suit, BaselineResponse, CardDto, CardsDto, CreateCard, CreatePerson, PatchCard,
    PatchPerson, PersonDto, SeedDto, SetOrder, SetPosition, SnapshotDto, SplitChildInput, SplitDto,
    SplitInput, TallyDto, UnsplitDto,
};
use crate::service::{
    CardPatch, CardView, NewCard, Service, ServiceError, SplitChild, SplitRequest,
};
use rusty_http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Shortest accepted token: a guessable token defeats the check.
pub const MIN_TOKEN_LEN: usize = 16;

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

    fn error(error: &ApiError) -> Self {
        #[derive(Serialize)]
        struct Body<'a> {
            error: Inner<'a>,
            #[serde(skip_serializing_if = "Option::is_none")]
            current: Option<&'a CardDto>,
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
            current: match error {
                ApiError::Stale(current) => Some(current),
                _ => None,
            },
        };
        let text = rusty_json::to_string(&body).unwrap_or_else(|_| "{}".to_string());
        Self {
            status: error.status(),
            body: text.into_bytes(),
        }
    }
}

enum ApiError {
    BadRequest(String),
    Unauthorized,
    NotFound(String),
    Invalid(String),
    Conflict(String),
    /// `If-Match` named another version; carries the card as stored now.
    Stale(Box<CardDto>),
    Internal,
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Stale(_) => StatusCode::PRECONDITION_FAILED,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::NotFound(_) => "not_found",
            Self::Invalid(_) => "invalid",
            Self::Conflict(_) => "conflict",
            Self::Stale(_) => "precondition_failed",
            Self::Internal => "internal",
        }
    }

    /// Internal errors never leak detail to the client.
    fn message(&self) -> String {
        match self {
            Self::BadRequest(m) | Self::NotFound(m) | Self::Invalid(m) | Self::Conflict(m) => {
                m.clone()
            }
            Self::Unauthorized => "missing or invalid bearer token".to_string(),
            Self::Stale(_) => "the card changed since you read it".to_string(),
            Self::Internal => "internal error".to_string(),
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
                eprintln!("rusty_fair_play: storage error: {e}");
                Self::Internal
            }
        }
    }
}

type Result<T> = std::result::Result<T, ApiError>;

/// The router plus the one token it may require.
pub struct Api {
    service: Service,
    token: Option<String>,
}

impl Api {
    /// `token` of `None` means no authentication: the binary then serves
    /// loopback only. `Err` for a token shorter than [`MIN_TOKEN_LEN`].
    pub fn new(service: Service, token: Option<String>) -> std::result::Result<Self, String> {
        if let Some(t) = &token {
            if t.len() < MIN_TOKEN_LEN {
                return Err(format!(
                    "the API token must be at least {MIN_TOKEN_LEN} characters"
                ));
            }
        }
        Ok(Self { service, token })
    }

    pub fn service(&self) -> &Service {
        &self.service
    }

    pub fn service_mut(&mut self) -> &mut Service {
        &mut self.service
    }

    /// Answer one request. `/health` needs no token; everything else is
    /// refused with a bare 401 unless the token is good (when one is set).
    pub fn handle(&mut self, request: &Request<'_>) -> Response {
        let path = split_target(request.target);
        let segments: Vec<String> = path
            .trim_matches('/')
            .split('/')
            .map(decode_segment)
            .collect();
        let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
        if request.method == &Method::Get && segments == ["health"] {
            return Response::json(StatusCode::OK, &Health { status: "ok" });
        }
        if !self.authorized(request.authorization) {
            return Response::error(&ApiError::Unauthorized);
        }
        let cx = Cx {
            method: request.method,
            body: request.body,
            if_match: request.if_match,
        };
        match segments.as_slice() {
            ["api", "v1", rest @ ..] => route(&mut self.service, &cx, rest),
            _ => Err(not_found()),
        }
        .unwrap_or_else(|error| Response::error(&error))
    }

    fn authorized(&self, header: Option<&str>) -> bool {
        let Some(token) = &self.token else {
            return true;
        };
        header
            .and_then(|h| h.strip_prefix("Bearer "))
            .is_some_and(|p| constant_time_eq(p.as_bytes(), token.as_bytes()))
    }
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

/// Compare without exiting early on the first differing byte.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

/// What a route handler needs besides the service.
struct Cx<'a> {
    method: &'a Method,
    body: &'a [u8],
    if_match: Option<&'a str>,
}

impl Cx<'_> {
    fn body<T: for<'de> Deserialize<'de>>(&self) -> Result<T> {
        rusty_json::from_slice(self.body)
            .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))
    }

    /// `Err(412 with the current card)` when `If-Match` is present and
    /// does not name the card as stored now; `Ok` otherwise.
    fn check_match(&self, service: &Service, id: Uuid) -> Result<()> {
        self.check(service, id, CardView::etag)
    }

    /// [`Cx::check_match`] against the subtree tag, for the writes that
    /// act on the children too.
    fn check_tree(&self, service: &Service, id: Uuid) -> Result<()> {
        self.check(service, id, |v| v.tree_etag.clone())
    }

    fn check(&self, service: &Service, id: Uuid, tag: fn(&CardView) -> String) -> Result<()> {
        let Some(presented) = self.if_match else {
            return Ok(());
        };
        let presented = presented.trim();
        if presented == "*" {
            return Ok(());
        }
        let current = service.card(id)?;
        let presented = presented.trim_start_matches("W/").trim_matches('"');
        if presented == tag(&current) {
            return Ok(());
        }
        Err(ApiError::Stale(Box::new(CardDto::from(current))))
    }
}

/// The path without its query string (no route reads a query today).
fn split_target(target: &str) -> &str {
    target.split_once('?').map_or(target, |(path, _)| path)
}

/// Percent-decode one path segment. `+` is literal in a path, unlike a query.
fn decode_segment(segment: &str) -> String {
    let escaped = segment.replace('+', "%2B");
    rusty_url::form_urlencoded::parse(format!("={escaped}").as_bytes())
        .next()
        .map_or_else(|| segment.to_string(), |(_, value)| value.into_owned())
}

fn route(service: &mut Service, cx: &Cx<'_>, path: &[&str]) -> Result<Response> {
    match (cx.method, path) {
        (Method::Get, ["snapshot"]) => Ok(Response::json(
            StatusCode::OK,
            &SnapshotDto::from(service.snapshot()?),
        )),
        (Method::Post, ["seed"]) => {
            let r = service.seed_deck()?;
            Ok(Response::json(
                StatusCode::OK,
                &SeedDto {
                    card_defaults: TallyDto {
                        created: r.card_defaults.created,
                        existing: r.card_defaults.existing,
                    },
                    cards: TallyDto {
                        created: r.cards.created,
                        existing: r.cards.existing,
                    },
                },
            ))
        }
        (_, ["people", rest @ ..]) => route_people(service, cx, rest),
        (_, ["cards", rest @ ..]) => route_cards(service, cx, rest),
        _ => Err(not_found()),
    }
}

fn route_people(service: &mut Service, cx: &Cx<'_>, path: &[&str]) -> Result<Response> {
    match (cx.method, path) {
        (Method::Get, []) => {
            let people: Vec<PersonDto> = service
                .snapshot()?
                .people
                .into_iter()
                .map(PersonDto::from)
                .collect();
            Ok(Response::json(StatusCode::OK, &People { people }))
        }
        (Method::Post, []) => {
            let input: CreatePerson = cx.body()?;
            let person = service.create_person(&input.name)?;
            Ok(Response::json(
                StatusCode::CREATED,
                &PersonDto::from(person),
            ))
        }
        (Method::Get, [id]) => {
            let person = service.person(parse_id(id)?)?;
            Ok(Response::json(StatusCode::OK, &PersonDto::from(person)))
        }
        (Method::Patch, [id]) => {
            let input: PatchPerson = cx.body()?;
            let person = service.rename_person(parse_id(id)?, &input.name)?;
            Ok(Response::json(StatusCode::OK, &PersonDto::from(person)))
        }
        (Method::Delete, [id]) => {
            service.delete_person(parse_id(id)?)?;
            Ok(Response::empty(StatusCode::NO_CONTENT))
        }
        _ => Err(not_found()),
    }
}

#[derive(Serialize)]
struct People {
    people: Vec<PersonDto>,
}

fn route_cards(service: &mut Service, cx: &Cx<'_>, path: &[&str]) -> Result<Response> {
    match (cx.method, path) {
        (Method::Post, []) => {
            let input: CreateCard = cx.body()?;
            let suit = suit(&input.suit)?;
            let view = service.create_card(NewCard {
                id: input.id,
                name: input.name,
                suit,
                parent_card_id: input.parent_card_id,
                owner_id: input.owner_id,
                conception: input.conception,
                planning: input.planning,
                execution: input.execution,
                minimum_standard_of_care: input.minimum_standard_of_care,
                notes: input.notes,
            })?;
            Ok(Response::json(StatusCode::CREATED, &CardDto::from(view)))
        }
        (Method::Get, [id]) => {
            let view = service.card(parse_id(id)?)?;
            Ok(Response::json(StatusCode::OK, &CardDto::from(view)))
        }
        (Method::Patch, [id]) => {
            cx.check_match(service, parse_id(id)?)?;
            let input: PatchCard = cx.body()?;
            let patch = CardPatch {
                name: input.name,
                suit: input.suit.as_deref().map(suit).transpose()?,
                conception: input.conception,
                planning: input.planning,
                execution: input.execution,
                minimum_standard_of_care: input.minimum_standard_of_care,
                notes: input.notes,
                owner_id: input.owner_id,
                parent_card_id: input.parent_card_id,
                position: input.position,
            };
            let view = service.patch_card(parse_id(id)?, patch)?;
            Ok(Response::json(StatusCode::OK, &CardDto::from(view)))
        }
        (Method::Delete, [id]) => {
            let id = parse_id(id)?;
            cx.check_match(service, id)?;
            service.delete_card(id)?;
            Ok(Response::empty(StatusCode::NO_CONTENT))
        }
        (Method::Post, [id, "unsplit"]) => {
            let id = parse_id(id)?;
            cx.check_tree(service, id)?;
            let (parent, deleted) = service.unsplit(id)?;
            Ok(Response::json(
                StatusCode::OK,
                &UnsplitDto {
                    parent: CardDto::from(parent),
                    deleted,
                },
            ))
        }
        (Method::Put, [id, "children", "order"]) => {
            let id = parse_id(id)?;
            cx.check_tree(service, id)?;
            let input: SetOrder = cx.body()?;
            let cards = service.reorder_children(id, &input.ids)?;
            Ok(Response::json(
                StatusCode::OK,
                &CardsDto {
                    cards: cards.into_iter().map(CardDto::from).collect(),
                },
            ))
        }
        (Method::Post, [id, "split"]) => {
            cx.check_match(service, parse_id(id)?)?;
            let input: SplitInput = cx.body()?;
            let request = SplitRequest {
                children: input.children.into_iter().map(split_child).collect(),
                owner_id: input.owner_id,
                notes: input.notes,
            };
            let split = service.split(parse_id(id)?, request)?;
            Ok(Response::json(
                StatusCode::CREATED,
                &SplitDto {
                    parent: CardDto::from(split.parent),
                    children: split.children.into_iter().map(CardDto::from).collect(),
                },
            ))
        }
        (Method::Post, [id, "reset"]) => {
            cx.check_match(service, parse_id(id)?)?;
            let view = service.reset(parse_id(id)?)?;
            Ok(Response::json(StatusCode::OK, &CardDto::from(view)))
        }
        (Method::Get, [id, "baseline"]) => {
            let (baseline, diff) = service.baseline(parse_id(id)?)?;
            Ok(Response::json(
                StatusCode::OK,
                &BaselineResponse {
                    baseline: baseline.map(Into::into),
                    diff: diff.into_iter().map(Into::into).collect(),
                },
            ))
        }
        (Method::Put, [id, "position"]) => {
            cx.check_match(service, parse_id(id)?)?;
            let input: SetPosition = cx.body()?;
            service.set_position(parse_id(id)?, input.position)?;
            Ok(Response::empty(StatusCode::NO_CONTENT))
        }
        _ => Err(not_found()),
    }
}

fn split_child(c: SplitChildInput) -> SplitChild {
    SplitChild {
        id: c.id,
        name: c.name,
        owner_id: c.owner_id,
        conception: c.conception,
        planning: c.planning,
        execution: c.execution,
        minimum_standard_of_care: c.minimum_standard_of_care,
        notes: c.notes,
    }
}

fn suit(text: &str) -> Result<rusty_fair_play_domain::Suit> {
    parse_suit(text).ok_or_else(|| {
        ApiError::Invalid(
            "suit must be one of Home, Out, Caregiving, Magic, Wild, Unicorn Space".into(),
        )
    })
}

fn not_found() -> ApiError {
    ApiError::NotFound("no such route".into())
}

fn parse_id(text: &str) -> Result<Uuid> {
    Uuid::parse_str(text).map_err(|_| ApiError::BadRequest("malformed id".into()))
}
