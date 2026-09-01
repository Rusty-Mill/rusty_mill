//! HTTP client for the meshed Data Product Registry REST API -- the
//! Rust port of `meshed.sdk.registry_client.RegistryClient`.
//!
//! Two deliberate deviations from the source, both decided with the
//! user rather than reproduced silently (per this migration's
//! boundary-contract rule that a source bug defaults to reproduced
//! unless someone explicitly rules otherwise):
//!
//! - **`register_output_port`'s dropped `event_type` (SDK-046).** The
//!   Python source accepts an `event_type` parameter but never places
//!   it into the outgoing payload -- the payload's `"event_type"` key
//!   is populated from a *different* parameter, `event_classification`,
//!   instead. Since this crate's `OutputPortCreate` (REG-033) has no
//!   `event_classification` concept at all and validates `event_type`
//!   against the `EventType` enum, reproducing the bug literally would
//!   feed an arbitrary string into a field that requires a real enum
//!   member. Fixed here: `event_type` is sent under the `"event_type"`
//!   key, and `event_classification` is dropped from the signature.
//! - **`get_output_port`'s port lookup by a nonexistent `"name"` field.**
//!   The source filters returned output-port JSON objects by
//!   `p.get("name") == port_name`, but neither the Python `OutputPort`
//!   model nor this crate's has a `name` field -- only `topic_name`/
//!   `schema_subject`/`event_type`/`description` exist.
//!   `register_output_port` stores the human-readable port name into
//!   `description` (the only field available for it), so filtering by
//!   `"name"` can never match a real registry response; the source's
//!   own unit tests only pass because their mocks fabricate a `"name"`
//!   key the real API never returns. Fixed here: the port-matching
//!   filter compares against `description` instead, so a
//!   register-then-look-up-by-name round trip actually works.

use crate::error::RegistryError;
use rusty_meshed_core::EventType;
use rusty_request::{Client, Json};

/// Async HTTP client for the meshed Data Product Registry. Stateless
/// (just a base URL) -- every method opens a fresh
/// [`rusty_request::Client`] per call (SDK-054), matching the source's
/// `async with httpx.AsyncClient(...)` per method.
pub struct RegistryClient {
    base_url: String,
}

impl RegistryClient {
    /// Builds a client for the registry at `base_url`, stripping a
    /// trailing `/` if present (SDK-043).
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        let base_url = base_url
            .strip_suffix('/')
            .map(str::to_string)
            .unwrap_or(base_url);
        RegistryClient { base_url }
    }

    /// Registers a new data product. `tags` defaults to an empty list,
    /// JSON-encoded into the `tags` field exactly as the registry's
    /// `DataProduct.tags` storage expects (REG-017). Returns the
    /// created product as returned by the registry.
    pub async fn register_product(
        &self,
        product_name: &str,
        domain: &str,
        version: &str,
        owner: &str,
        description: &str,
        tags: Option<&[String]>,
    ) -> Result<Json, RegistryError> {
        let mut tags_array = Json::array();
        for tag in tags.unwrap_or(&[]) {
            tags_array.push(tag.as_str());
        }

        let mut body = Json::object();
        body.insert("name", product_name);
        body.insert("domain", domain);
        body.insert("version", version);
        body.insert("owner", owner);
        body.insert("description", description);
        body.insert("tags", tags_array.to_json_string());

        let url = format!("{}/data-products/", self.base_url);
        let response = Client::new()
            .post(&url)
            .map_err(http_err)?
            .json(&body)
            .map_err(http_err)?
            .send()
            .await
            .map_err(http_err)?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(RegistryError::new(format!(
                "Failed to register product '{product_name}': HTTP {status}"
            )));
        }
        response.json().map_err(http_err)
    }

    /// Registers an output port under an existing data product. `name`
    /// is the port's human-readable label -- stored in the registry's
    /// `description` field, since there is no dedicated `name` column
    /// (see the module doc). Returns the created output port.
    pub async fn register_output_port(
        &self,
        product_id: i64,
        name: &str,
        topic_name: &str,
        schema_subject: &str,
        event_type: EventType,
    ) -> Result<Json, RegistryError> {
        let mut body = Json::object();
        body.insert("topic_name", topic_name);
        body.insert("schema_subject", schema_subject);
        body.insert("event_type", event_type.as_str());
        body.insert("description", name);

        let url = format!("{}/data-products/{product_id}/output-ports", self.base_url);
        let response = Client::new()
            .post(&url)
            .map_err(http_err)?
            .json(&body)
            .map_err(http_err)?
            .send()
            .await
            .map_err(http_err)?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(RegistryError::new(format!(
                "Failed to register output port '{name}' on product {product_id}: HTTP {status}"
            )));
        }
        response.json().map_err(http_err)
    }

    /// Resolves an output port by product name and port name: first
    /// finds the product by exact `name` match (a real field on
    /// `DataProduct`), then finds the port among that product's output
    /// ports by exact `description` match (see the module doc for why
    /// `description`, not `name`).
    pub async fn get_output_port(
        &self,
        product_name: &str,
        port_name: &str,
    ) -> Result<Json, RegistryError> {
        let products_url = format!("{}/data-products", self.base_url);
        let response = Client::new()
            .get(&products_url)
            .map_err(http_err)?
            .query([("name", product_name)])
            .send()
            .await
            .map_err(http_err)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(RegistryError::new(format!(
                "Failed to search for product '{product_name}': HTTP {status}"
            )));
        }
        let products = response.json().map_err(http_err)?;
        let products = products.as_array().unwrap_or(&[]);
        let product_id = products
            .iter()
            .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(product_name))
            .and_then(|p| p.get("id"))
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                RegistryError::new(format!("No product found with name '{product_name}'"))
            })? as i64;

        let ports_url = format!("{}/data-products/{product_id}/output-ports", self.base_url);
        let response = Client::new()
            .get(&ports_url)
            .map_err(http_err)?
            .send()
            .await
            .map_err(http_err)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(RegistryError::new(format!(
                "Failed to fetch output ports for product {product_id}: HTTP {status}"
            )));
        }
        let ports = response.json().map_err(http_err)?;
        let ports = ports.as_array().unwrap_or(&[]);
        ports
            .iter()
            .find(|p| p.get("description").and_then(|v| v.as_str()) == Some(port_name))
            .cloned()
            .ok_or_else(|| {
                RegistryError::new(format!(
                    "No output port '{port_name}' found on product '{product_name}'"
                ))
            })
    }

    /// Fetches the current contract for a product/output-port pair.
    /// Returns `None` on exactly HTTP 404 (no contract registered yet)
    /// rather than treating it as an error.
    pub async fn get_contract(
        &self,
        product_id: i64,
        port_id: i64,
    ) -> Result<Option<Json>, RegistryError> {
        let url = format!(
            "{}/data-products/{product_id}/output-ports/{port_id}/contract",
            self.base_url
        );
        let response = Client::new()
            .get(&url)
            .map_err(http_err)?
            .send()
            .await
            .map_err(http_err)?;
        if response.status().as_u16() == 404 {
            return Ok(None);
        }
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(RegistryError::new(format!(
                "Failed to fetch contract for product {product_id}, port {port_id}: HTTP {status}"
            )));
        }
        response.json().map(Some).map_err(http_err)
    }
}

fn http_err(err: rusty_request::Error) -> RegistryError {
    RegistryError::new(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_http::async_tokio::AsyncTransport;
    use rusty_http::head::ResponseHead;
    use rusty_http::{HeaderMap, StatusCode, Version};
    use rusty_tokio::io::TcpListener;

    struct CapturedRequest {
        method: String,
        target: String,
        body: String,
    }

    /// Serves `responses.len()` sequential request/response exchanges
    /// on one listener (`get_output_port` makes two calls per test).
    fn start_fake_server(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, rusty_tokio::JoinHandle<Vec<CapturedRequest>>) {
        let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).expect("failed to bind");
        let addr = listener.local_addr().expect("failed to read local_addr");
        let url = format!("http://{addr}");

        let handle = rusty_tokio::spawn(async move {
            let mut captured = Vec::new();
            for (status, response_body) in responses {
                let (stream, _peer) = listener.accept().await.expect("failed to accept");
                let mut transport = AsyncTransport::new(stream);
                let head = transport
                    .read_request_head(8192)
                    .await
                    .expect("failed to read request head");
                let framing = rusty_http::body::request_framing(&head.headers)
                    .expect("failed to determine framing");
                let body_bytes = transport
                    .read_body(framing)
                    .await
                    .expect("failed to read request body");

                let mut headers = HeaderMap::new();
                headers
                    .insert("Content-Length", &response_body.len().to_string())
                    .unwrap();
                headers.insert("Content-Type", "application/json").unwrap();
                let response_head = ResponseHead {
                    status: StatusCode::from_u16(status),
                    reason: String::new(),
                    version: Version::Http11,
                    headers,
                };
                transport
                    .write_response_head(&response_head)
                    .await
                    .expect("failed to write response head");
                transport
                    .write_body(response_body.as_bytes())
                    .await
                    .expect("failed to write response body");

                captured.push(CapturedRequest {
                    method: head.method.as_str().to_string(),
                    target: head.target.clone(),
                    body: String::from_utf8(body_bytes).expect("request body wasn't UTF-8"),
                });
            }
            captured
        });

        (url, handle)
    }

    #[test]
    fn new_strips_a_trailing_slash() {
        let client = RegistryClient::new("http://localhost:8000/");
        assert_eq!(client.base_url, "http://localhost:8000");
        let client = RegistryClient::new("http://localhost:8000");
        assert_eq!(client.base_url, "http://localhost:8000");
    }

    #[rusty_tokio::test]
    async fn register_product_posts_the_expected_payload() {
        let (url, server) = start_fake_server(vec![(200, r#"{"id": 1, "name": "orders"}"#)]);
        let client = RegistryClient::new(&url);
        let result = client
            .register_product(
                "orders",
                "commerce",
                "1.0.0",
                "team-a",
                "Order events",
                Some(&["finance".to_string()]),
            )
            .await
            .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].target, "/data-products/");
        assert!(requests[0].body.contains(r#""tags":"[\"finance\"]""#));
        assert_eq!(result.get("id").unwrap().as_f64(), Some(1.0));
    }

    #[rusty_tokio::test]
    async fn register_product_defaults_tags_to_an_empty_list() {
        let (url, server) = start_fake_server(vec![(200, r#"{"id": 1}"#)]);
        let client = RegistryClient::new(&url);
        client
            .register_product("orders", "commerce", "1.0.0", "team-a", "", None)
            .await
            .unwrap();

        let requests = server.await.unwrap();
        assert!(requests[0].body.contains(r#""tags":"[]""#));
    }

    #[rusty_tokio::test]
    async fn register_product_raises_registry_error_on_non_2xx() {
        let (url, server) = start_fake_server(vec![(500, "{}")]);
        let client = RegistryClient::new(&url);
        let err = client
            .register_product("orders", "commerce", "1.0.0", "team-a", "", None)
            .await
            .unwrap_err();
        server.await.unwrap();

        assert_eq!(
            err.message(),
            "Failed to register product 'orders': HTTP 500"
        );
    }

    #[rusty_tokio::test]
    async fn register_output_port_sends_the_real_event_type() {
        let (url, server) = start_fake_server(vec![(200, r#"{"id": 10}"#)]);
        let client = RegistryClient::new(&url);
        client
            .register_output_port(
                2,
                "orders-created",
                "orders.created",
                "orders.created-value",
                EventType::Delta,
            )
            .await
            .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].target, "/data-products/2/output-ports");
        assert!(requests[0].body.contains(r#""event_type":"delta""#));
        assert!(requests[0]
            .body
            .contains(r#""description":"orders-created""#));
    }

    #[rusty_tokio::test]
    async fn register_output_port_raises_registry_error_on_non_2xx() {
        let (url, server) = start_fake_server(vec![(404, "{}")]);
        let client = RegistryClient::new(&url);
        let err = client
            .register_output_port(999, "port", "t", "s", EventType::Delta)
            .await
            .unwrap_err();
        server.await.unwrap();

        assert_eq!(
            err.message(),
            "Failed to register output port 'port' on product 999: HTTP 404"
        );
    }

    #[rusty_tokio::test]
    async fn get_output_port_resolves_product_then_matches_port_by_description() {
        let (url, server) = start_fake_server(vec![
            (200, r#"[{"id": 2, "name": "orders"}]"#),
            (
                200,
                r#"[{"id": 10, "topic_name": "orders.created", "description": "orders-created"},
                    {"id": 11, "topic_name": "orders.cancelled", "description": "orders-cancelled"}]"#,
            ),
        ]);
        let client = RegistryClient::new(&url);
        let result = client
            .get_output_port("orders", "orders-created")
            .await
            .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests[0].target, "/data-products?name=orders");
        assert_eq!(requests[1].target, "/data-products/2/output-ports");
        assert_eq!(result.get("id").unwrap().as_f64(), Some(10.0));
    }

    #[rusty_tokio::test]
    async fn get_output_port_raises_when_no_product_matches() {
        let (url, server) = start_fake_server(vec![(200, "[]")]);
        let client = RegistryClient::new(&url);
        let err = client
            .get_output_port("nonexistent", "some-port")
            .await
            .unwrap_err();
        server.await.unwrap();

        assert_eq!(err.message(), "No product found with name 'nonexistent'");
    }

    #[rusty_tokio::test]
    async fn get_output_port_raises_when_no_port_matches() {
        let (url, server) = start_fake_server(vec![
            (200, r#"[{"id": 2, "name": "orders"}]"#),
            (200, r#"[{"id": 10, "description": "orders-created"}]"#),
        ]);
        let client = RegistryClient::new(&url);
        let err = client
            .get_output_port("orders", "missing-port")
            .await
            .unwrap_err();
        server.await.unwrap();

        assert_eq!(
            err.message(),
            "No output port 'missing-port' found on product 'orders'"
        );
    }

    #[rusty_tokio::test]
    async fn get_contract_returns_the_contract_on_200() {
        let (url, server) =
            start_fake_server(vec![(200, r#"{"id": 1, "schema_ref": "orders-value:1"}"#)]);
        let client = RegistryClient::new(&url);
        let contract = client.get_contract(2, 10).await.unwrap();

        let requests = server.await.unwrap();
        assert_eq!(
            requests[0].target,
            "/data-products/2/output-ports/10/contract"
        );
        assert_eq!(
            contract.unwrap().get("schema_ref").unwrap().as_str(),
            Some("orders-value:1")
        );
    }

    #[rusty_tokio::test]
    async fn get_contract_returns_none_on_404() {
        let (url, server) = start_fake_server(vec![(404, "{}")]);
        let client = RegistryClient::new(&url);
        let contract = client.get_contract(2, 10).await.unwrap();
        server.await.unwrap();

        assert_eq!(contract, None);
    }

    #[rusty_tokio::test]
    async fn get_contract_raises_registry_error_on_non_404_non_2xx() {
        let (url, server) = start_fake_server(vec![(500, "{}")]);
        let client = RegistryClient::new(&url);
        let err = client.get_contract(2, 10).await.unwrap_err();
        server.await.unwrap();

        assert_eq!(
            err.message(),
            "Failed to fetch contract for product 2, port 10: HTTP 500"
        );
    }
}
