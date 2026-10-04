//! Fair Play for a household: a JSON HTTP API and a static file server for
//! the web UI in `web/`, over `rusty_multimodal_db`'s `fair_play` domain
//! (ADR-0137). `rusty_tick`'s shape: `api` (a pure router) and `dto` (the
//! wire shapes) sit over `service` (the rules, holding the three stacks);
//! `server` binds `Api` to `rusty_serve`, the shared blocking HTTP server
//! that also serves the built UI.

pub mod api;
pub mod dto;
pub mod server;
pub mod service;
