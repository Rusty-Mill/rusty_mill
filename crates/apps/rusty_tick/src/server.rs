//! The TCP adapter: [`rusty_serve`]'s blocking HTTP/1.1 server (one thread
//! per connection, bounded head, body, idle time and connection count)
//! with [`Backend`] as its [`Handler`]. Everything about the API itself
//! lives in [`crate::api`]; this file only converts request and response
//! shapes.

use crate::api;
use crate::backend::Backend;
pub use rusty_serve::{Handler, ShutdownHandle, DEFAULT_MAX_CONNECTIONS, MAX_BODY_BYTES};

pub type Server = rusty_serve::Server<Backend>;

impl Handler for Backend {
    fn handle(&mut self, request: &rusty_serve::Request<'_>) -> rusty_serve::Response {
        let response = Backend::handle(
            self,
            &api::Request {
                method: request.method,
                target: request.target,
                authorization: request.authorization,
                if_match: request.if_match,
                body: request.body,
            },
        );
        rusty_serve::Response {
            status: response.status,
            body: response.body,
        }
    }
}
