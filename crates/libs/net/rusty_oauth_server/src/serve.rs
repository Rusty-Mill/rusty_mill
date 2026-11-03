//! The endpoints on [`rusty_serve`] (feature `serve`): a server built on it
//! answers OAuth with [`handle`] and its `/mcp` 401 with [`unauthorized`].

use rusty_http::{Method, StatusCode};

use crate::{now_unix, Endpoints, Response};

/// The OAuth answer to `request`, or `None` when its path is not an OAuth
/// path.
pub fn handle(
    endpoints: &Endpoints,
    request: &rusty_serve::Request<'_>,
) -> Option<rusty_serve::Response> {
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((request.target, ""));
    let method = match request.method {
        Method::Get => crate::Method::Get,
        Method::Post => crate::Method::Post,
        _ => crate::Method::Other,
    };
    let request = crate::Request {
        method,
        path,
        query,
        body: request.body,
    };
    endpoints.handle(&request, now_unix()).map(convert)
}

/// [`Endpoints::unauthorized`]: the `/mcp` 401 whose challenge points a
/// client at the OAuth metadata.
pub fn unauthorized(endpoints: &Endpoints) -> rusty_serve::Response {
    convert(endpoints.unauthorized())
}

/// `response` as [`rusty_serve`] sends it. A header it refuses is a bug
/// here (the endpoints set only ordinary headers), answered as a 500.
fn convert(response: Response) -> rusty_serve::Response {
    let status = StatusCode::from_u16(response.status);
    let headers = response.headers;
    headers
        .iter()
        .try_fold(
            rusty_serve::Response::json(status, response.body),
            |out, (name, value)| out.with_header(name, value),
        )
        .unwrap_or_else(|error| {
            eprintln!("oauth: a response header was refused: {error}");
            rusty_serve::Response::internal_error()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Branding, Provider, StateStore, Subject, Verify};
    use std::sync::Arc;

    #[test]
    fn oauth_paths_are_answered_and_others_fall_through() {
        let dir = tempfile::tempdir().unwrap();
        let verify: Verify = Box::new(|_| None::<Subject>);
        let provider = Provider::new(verify, StateStore::new(dir.path().join("oauth.json")));
        let branding = Branding {
            resource_name: "t",
            resource_description: "t",
            token_label: "t",
        };
        let issuer = crate::validate_issuer("https://example.test").unwrap();
        let endpoints = Endpoints::new(Arc::new(provider), issuer, branding);
        let headers = rusty_http::HeaderMap::new();
        let request = |target: &'static str| rusty_serve::Request {
            method: &Method::Get,
            target,
            authorization: None,
            if_match: None,
            headers: &headers,
            body: b"",
        };
        let metadata = handle(
            &endpoints,
            &request("/.well-known/oauth-authorization-server?x"),
        )
        .expect("an OAuth path");
        assert_eq!(metadata.status, StatusCode::OK);
        assert!(handle(&endpoints, &request("/api/x")).is_none());
        let refused = unauthorized(&endpoints);
        assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
        assert!(refused
            .headers
            .get("www-authenticate")
            .is_some_and(|v| v.contains("resource_metadata=")));
    }
}
