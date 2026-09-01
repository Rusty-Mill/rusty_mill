//! Per-resource HTTP routers -- the Rust port of
//! `meshed.registry.routers.*`. Each submodule builds a standalone
//! [`crate::http::Router`] that [`crate::app::build_router`] merges
//! into the app's route table (the equivalent of FastAPI's
//! `app.include_router`, REG-006).

pub mod data_products;
pub mod governance;

use crate::http::response::Response;
use rusty_http::StatusCode;

/// Builds a `{"detail": "<message>"}` error response, matching
/// FastAPI's `HTTPException(status_code=..., detail="...")` JSON shape
/// for a plain-string detail.
pub(crate) fn detail_error(status: StatusCode, message: impl Into<String>) -> Response {
    let mut body = rusty_request::Json::object();
    body.insert("detail", message.into());
    Response::json(status, &body)
}

/// The 404 every resource router raises for an unknown ID, worded
/// identically to the source across every resource (`"Data product not
/// found"`, etc. -- callers pass their own resource-specific message).
pub(crate) fn not_found(resource: &str) -> Response {
    detail_error(StatusCode::NOT_FOUND, format!("{resource} not found"))
}
