//! Extractors: the arguments a handler takes.
//!
//! Extraction is synchronous because the body is already read. Extractors
//! run in argument order; one that takes the body ([`Json`], `String`,
//! `Vec<u8>`) goes last, since a later one would see it empty.

use std::convert::Infallible;
use std::fmt;

use rusty_http::{HeaderMap, Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::de::Pairs;
use crate::request::Request;
use crate::response::{IntoResponse, Response};

/// Builds a handler argument from the request and the router's state.
/// Implement it for an extractor of your own, mapping failures to your own
/// [`Rejection`](FromRequest::Rejection) type.
pub trait FromRequest<S>: Sized {
    /// What the client gets when extraction fails.
    type Rejection: IntoResponse;

    /// Extracts `Self`, possibly taking the body out of `req`.
    ///
    /// # Errors
    ///
    /// The rejection that answers the request instead of the handler.
    fn from_request(req: &mut Request, state: &S) -> Result<Self, Self::Rejection>;
}

/// Why a built-in extractor refused a request: a status and a plain-text
/// message, which is also the response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    status: StatusCode,
    message: String,
}

impl Rejection {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Rejection {
            status,
            message: message.into(),
        }
    }

    /// The status the rejection answers with.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// What went wrong, for a custom extractor that remaps the rejection.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Rejection {}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}

/// The router's state, cloned (see [`Router::with_state`](crate::Router::with_state)).
#[derive(Debug, Clone, Copy, Default)]
pub struct State<S>(pub S);

impl<S: Clone> FromRequest<S> for State<S> {
    type Rejection = Infallible;

    fn from_request(_req: &mut Request, state: &S) -> Result<Self, Infallible> {
        Ok(State(state.clone()))
    }
}

/// The query string, deserialized: a struct by field name, a map, or a
/// `HashMap<String, String>`. A missing query is an empty one.
#[derive(Debug, Clone, Copy, Default)]
pub struct Query<T>(pub T);

impl<T: DeserializeOwned, S> FromRequest<S> for Query<T> {
    type Rejection = Rejection;

    fn from_request(req: &mut Request, _state: &S) -> Result<Self, Rejection> {
        let pairs = query_pairs(req.query().unwrap_or(""));
        T::deserialize(Pairs(&pairs))
            .map(Query)
            .map_err(|e| Rejection::new(StatusCode::BAD_REQUEST, format!("invalid query: {e}")))
    }
}

fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|field| !field.is_empty())
        .map(|field| {
            let (name, value) = field.split_once('=').unwrap_or((field, ""));
            (
                rusty_percent::decode_form(name).into_owned(),
                rusty_percent::decode_form(value).into_owned(),
            )
        })
        .collect()
}

/// The matched route's path parameters, deserialized: one value for one
/// parameter, a tuple by position, or a struct or map by name.
#[derive(Debug, Clone, Copy, Default)]
pub struct Path<T>(pub T);

impl<T: DeserializeOwned, S> FromRequest<S> for Path<T> {
    type Rejection = Rejection;

    fn from_request(req: &mut Request, _state: &S) -> Result<Self, Rejection> {
        T::deserialize(Pairs(req.params()))
            .map(Path)
            .map_err(|e| Rejection::new(StatusCode::BAD_REQUEST, format!("invalid path: {e}")))
    }
}

/// A JSON body, in a request or a response. A request must say
/// `Content-Type: application/json` (or a `+json` type): 415 otherwise, 400
/// for malformed JSON and 422 for JSON of the wrong shape, as axum answers.
#[derive(Debug, Clone, Copy, Default)]
pub struct Json<T>(pub T);

impl<T: DeserializeOwned, S> FromRequest<S> for Json<T> {
    type Rejection = Rejection;

    fn from_request(req: &mut Request, _state: &S) -> Result<Self, Rejection> {
        if !is_json(req.headers()) {
            return Err(Rejection::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "expected a request with Content-Type: application/json",
            ));
        }
        let body = req.take_body();
        serde_json::from_slice(&body).map(Json).map_err(|e| {
            let status = if e.is_data() {
                StatusCode::UNPROCESSABLE_ENTITY
            } else {
                StatusCode::BAD_REQUEST
            };
            Rejection::new(status, format!("invalid JSON body: {e}"))
        })
    }
}

fn is_json(headers: &HeaderMap) -> bool {
    let Some(value) = headers.get("content-type") else {
        return false;
    };
    let essence = value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    essence == "application/json"
        || (essence.starts_with("application/") && essence.ends_with("+json"))
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        match serde_json::to_vec(&self.0) {
            Ok(body) => {
                Response::new(StatusCode::OK, body).with_header("content-type", "application/json")
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("response did not serialize: {e}"),
            )
                .into_response(),
        }
    }
}

impl<S> FromRequest<S> for HeaderMap {
    type Rejection = Infallible;

    fn from_request(req: &mut Request, _state: &S) -> Result<Self, Infallible> {
        Ok(req.headers().clone())
    }
}

impl<S> FromRequest<S> for Method {
    type Rejection = Infallible;

    fn from_request(req: &mut Request, _state: &S) -> Result<Self, Infallible> {
        Ok(req.method().clone())
    }
}

/// The raw body.
impl<S> FromRequest<S> for Vec<u8> {
    type Rejection = Infallible;

    fn from_request(req: &mut Request, _state: &S) -> Result<Self, Infallible> {
        Ok(req.take_body())
    }
}

/// The body as UTF-8 text; 400 when it is not.
impl<S> FromRequest<S> for String {
    type Rejection = Rejection;

    fn from_request(req: &mut Request, _state: &S) -> Result<Self, Rejection> {
        String::from_utf8(req.take_body())
            .map_err(|_| Rejection::new(StatusCode::BAD_REQUEST, "request body is not UTF-8"))
    }
}

/// The whole request, body included. Goes last, like a body extractor.
impl<S> FromRequest<S> for Request {
    type Rejection = Infallible;

    fn from_request(req: &mut Request, _state: &S) -> Result<Self, Infallible> {
        let body = req.take_body();
        Ok(req.clone().with_body(body))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::HashMap;

    use super::*;

    fn json_req(body: &str) -> Request {
        Request::new(Method::Post, "/")
            .with_header("content-type", "application/json; charset=utf-8")
            .map(|r| r.with_body(body.as_bytes().to_vec()))
            .unwrap()
    }

    #[test]
    fn query_decodes_plus_and_percent() {
        let mut req = Request::new(Method::Get, "/s?q=a+b%26c&flag&=x");
        let Query(q) = Query::<HashMap<String, String>>::from_request(&mut req, &()).unwrap();
        assert_eq!(q["q"], "a b&c");
        assert_eq!(q["flag"], "");
    }

    #[test]
    fn missing_query_is_empty() {
        let mut req = Request::new(Method::Get, "/s");
        let Query(q) = Query::<HashMap<String, String>>::from_request(&mut req, &()).unwrap();
        assert!(q.is_empty());
    }

    #[test]
    fn json_statuses_follow_axum() {
        let ok = Json::<serde_json::Value>::from_request(&mut json_req("{\"a\":1}"), &());
        assert!(ok.is_ok());
        let syntax = Json::<serde_json::Value>::from_request(&mut json_req("{"), &());
        assert_eq!(syntax.unwrap_err().status(), StatusCode::BAD_REQUEST);
        let shape = Json::<Vec<u8>>::from_request(&mut json_req("{}"), &());
        assert_eq!(
            shape.unwrap_err().status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let mut plain = Request::new(Method::Post, "/").with_body(b"{}".to_vec());
        let media = Json::<serde_json::Value>::from_request(&mut plain, &());
        assert_eq!(
            media.unwrap_err().status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
    }

    #[test]
    fn json_suffix_types_count() {
        let mut headers = HeaderMap::new();
        headers
            .insert("content-type", "application/merge-patch+json")
            .unwrap();
        assert!(is_json(&headers));
        headers.insert("content-type", "text/json").unwrap();
        assert!(!is_json(&headers));
    }

    #[test]
    fn json_response_sets_content_type() {
        let r = Json(serde_json::json!({"ok": true})).into_response();
        assert_eq!(r.headers.get("content-type"), Some("application/json"));
        assert_eq!(r.body.as_bytes(), Some(&b"{\"ok\":true}"[..]));
    }

    #[test]
    fn string_body_must_be_utf8() {
        let mut req = Request::new(Method::Post, "/").with_body(vec![0xff]);
        let err = String::from_request(&mut req, &()).unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }
}
