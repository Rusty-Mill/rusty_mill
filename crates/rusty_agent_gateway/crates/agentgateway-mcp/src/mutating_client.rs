//! An HTTP client that applies a guardrail's `headerMutation`.
//!
//! `mcpGuardrails` lets a processor change the headers of the upstream HTTP
//! request that carries an MCP call. That request is made by `rmcp`'s transport
//! rather than by this crate, and the transport's headers are fixed when the
//! target is dialled — so the change has to travel with the call itself.
//!
//! It rides in the request's `extensions`, which `rmcp` carries in memory from
//! [`rmcp::service::Peer::send_request`] down to
//! [`StreamableHttpClient::post_message`] without serializing them. This client
//! reads it back out there and folds it into the outgoing headers, then hands
//! the request to `reqwest` unchanged.
//!
//! # Only `mcp:` targets
//!
//! A `stdio` target speaks over a pipe: there is no HTTP request, so there are
//! no headers to change. Upstream says the same, and a mutation aimed at one is
//! dropped rather than quietly appearing somewhere else.

use std::collections::HashMap;
use std::sync::Arc;

use http::{HeaderName, HeaderValue};
use rmcp::model::{ClientJsonRpcMessage, GetExtensions};
use rmcp::transport::common::client_side_sse::BoxedSseResponse;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};

/// Header changes attached to one outgoing MCP request.
///
/// Put in a request's extensions by the federation and taken out again here.
#[derive(Debug, Clone, Default)]
pub struct HeaderOverride {
    /// Headers to add or overwrite.
    pub set: Vec<(HeaderName, HeaderValue)>,
    /// Header names to drop.
    pub remove: Vec<HeaderName>,
}

impl HeaderOverride {
    /// Whether there is anything to do.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.remove.is_empty()
    }

    /// Fold this override into the transport's per-request header map.
    fn apply(&self, headers: &mut HashMap<HeaderName, HeaderValue>) {
        for name in &self.remove {
            headers.remove(name);
        }
        for (name, value) in &self.set {
            headers.insert(name.clone(), value.clone());
        }
    }
}

/// A `reqwest`-backed client that honours [`HeaderOverride`].
///
/// Everything else is delegated: this exists only to read one extension off
/// the message on its way past.
#[derive(Debug, Clone, Default)]
pub struct MutatingClient {
    inner: reqwest::Client,
}

impl MutatingClient {
    /// Wrap a `reqwest` client.
    pub fn new(inner: reqwest::Client) -> Self {
        MutatingClient { inner }
    }
}

/// The override carried by a message, if it is a request and carries one.
fn override_of(message: &ClientJsonRpcMessage) -> Option<&HeaderOverride> {
    match message {
        ClientJsonRpcMessage::Request(request) => {
            request.request.extensions().get::<HeaderOverride>()
        }
        _ => None,
    }
}

impl StreamableHttpClient for MutatingClient {
    type Error = reqwest::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        mut custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        if let Some(changes) = override_of(&message) {
            changes.apply(&mut custom_headers);
        }
        self.inner
            .post_message(uri, message, session_id, auth_header, custom_headers)
            .await
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        // Tearing down a session is not a call a processor was asked about.
        self.inner
            .delete_session(uri, session_id, auth_header, custom_headers)
            .await
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxedSseResponse, StreamableHttpError<Self::Error>> {
        // The server-to-client stream carries no request to attach a mutation
        // to; it is opened once and outlives any individual call.
        self.inner
            .get_stream(uri, session_id, last_event_id, auth_header, custom_headers)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> HeaderName {
        HeaderName::try_from(text).expect("valid header name")
    }

    fn value(text: &str) -> HeaderValue {
        HeaderValue::from_str(text).expect("valid header value")
    }

    #[test]
    fn a_set_overwrites_what_the_transport_would_have_sent() {
        let mut headers = HashMap::from([(name("x-tenant"), value("default"))]);
        HeaderOverride {
            set: vec![(name("x-tenant"), value("acme"))],
            remove: Vec::new(),
        }
        .apply(&mut headers);

        assert_eq!(headers[&name("x-tenant")], "acme");
    }

    #[test]
    fn a_remove_drops_a_header_the_transport_would_have_sent() {
        let mut headers = HashMap::from([(name("x-internal"), value("secret"))]);
        HeaderOverride {
            set: Vec::new(),
            remove: vec![name("x-internal")],
        }
        .apply(&mut headers);

        assert!(headers.is_empty());
    }

    #[test]
    fn set_is_applied_after_remove() {
        // So a processor that sends both for one name ends up setting it,
        // rather than the order of two independent lists deciding the answer.
        let mut headers = HashMap::new();
        HeaderOverride {
            set: vec![(name("x-user"), value("u-1"))],
            remove: vec![name("x-user")],
        }
        .apply(&mut headers);

        assert_eq!(headers[&name("x-user")], "u-1");
    }

    #[test]
    fn an_empty_override_changes_nothing() {
        let mut headers = HashMap::from([(name("x-tenant"), value("acme"))]);
        HeaderOverride::default().apply(&mut headers);
        assert_eq!(headers.len(), 1);
        assert!(HeaderOverride::default().is_empty());
    }
}
