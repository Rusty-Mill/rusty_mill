//! Consumer-driven contract gate via the Schema Registry compatibility
//! API -- the Rust port of `meshed.observability.contract_gate`.
//!
//! Implements the CI gate pattern: producers must prove their schema
//! changes are backward compatible with all registered consumer
//! contracts before those changes reach production.
//!
//! ```ignore
//! let subject = contract_subject_name("billing-service", "order-events-value");
//! register_consumer_contract(registry_url, &subject, consumer_avro_schema).await?;
//! assert_schema_compatible(registry_url, producer_avro_schema, &subject).await?;
//! ```

use rusty_err::Error;
use rusty_request::{Client, Json};

/// Errors from [`register_consumer_contract`] and
/// [`assert_schema_compatible`].
#[derive(Debug, Error)]
pub enum ContractGateError {
    /// The HTTP request itself failed (connection, TLS, non-2xx status
    /// via [`rusty_request::Response::error_for_status`], ...) --
    /// matches the source's unguarded `httpx.HTTPStatusError`.
    #[error("Schema Registry request failed: {0}")]
    Http(#[from] rusty_request::Error),
    /// The response's JSON body didn't have the shape this call
    /// expected.
    #[error("unexpected Schema Registry response: {0}")]
    UnexpectedResponse(String),
    /// [`assert_schema_compatible`] found the producer schema
    /// incompatible with the registered consumer contract -- the
    /// Rust equivalent of the source's `AssertionError`.
    #[error("Producer schema is incompatible with consumer contract '{0}': {1}")]
    Incompatible(String, String),
}

/// Builds the contract subject name following the platform naming
/// convention: `{consumer_group}.contracts.{producer_subject}`. Pure
/// naming-convention function, no I/O -- separates consumer contract
/// schemas from producer schemas in the Schema Registry namespace,
/// preventing accidental overlap.
pub fn contract_subject_name(consumer_group: &str, producer_subject: &str) -> String {
    format!("{consumer_group}.contracts.{producer_subject}")
}

/// Registers a consumer contract schema under `contract_subject`. The
/// contract captures the minimum schema a consumer requires -- used as
/// the compatibility baseline for future producer changes. Returns the
/// schema ID assigned by the Schema Registry.
pub async fn register_consumer_contract(
    registry_url: &str,
    contract_subject: &str,
    avro_schema_str: &str,
) -> Result<i64, ContractGateError> {
    let url = format!("{registry_url}/subjects/{contract_subject}/versions");
    let mut body = Json::object();
    body.insert("schemaType", "AVRO");
    body.insert("schema", avro_schema_str);

    let response = Client::new().post(&url)?.json(&body)?.send().await?;
    let response = response.error_for_status()?;
    let body = response.json()?;
    body.get("id")
        .and_then(|value| value.as_f64())
        .map(|id| id as i64)
        .ok_or_else(|| ContractGateError::UnexpectedResponse("missing 'id' field".to_string()))
}

/// Asserts that a producer schema is compatible with a registered
/// consumer contract, checking against all versions registered under
/// `contract_subject` via the Schema Registry `/compatibility` endpoint
/// with `verbose=true` for detailed failure messages. Returns
/// [`ContractGateError::Incompatible`] (not `Ok(false)`) when the
/// schema is incompatible, matching the source's `AssertionError`.
pub async fn assert_schema_compatible(
    registry_url: &str,
    producer_schema_str: &str,
    contract_subject: &str,
) -> Result<(), ContractGateError> {
    let url =
        format!("{registry_url}/compatibility/subjects/{contract_subject}/versions?verbose=true");
    let mut body = Json::object();
    body.insert("schemaType", "AVRO");
    body.insert("schema", producer_schema_str);

    let response = Client::new().post(&url)?.json(&body)?.send().await?;
    let response = response.error_for_status()?;
    let body = response.json()?;

    let is_compatible = body
        .get("is_compatible")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if is_compatible {
        return Ok(());
    }

    let detail = body
        .get("messages")
        .and_then(|value| value.as_array())
        .map(|messages| {
            let joined: Vec<&str> = messages.iter().filter_map(|m| m.as_str()).collect();
            joined.join("; ")
        })
        .filter(|detail| !detail.is_empty())
        .unwrap_or_else(|| "Schema is incompatible".to_string());

    Err(ContractGateError::Incompatible(
        contract_subject.to_string(),
        detail,
    ))
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

    fn start_fake_server(
        status: u16,
        response_body: &'static str,
    ) -> (String, rusty_tokio::JoinHandle<CapturedRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).expect("failed to bind");
        let addr = listener.local_addr().expect("failed to read local_addr");
        let url = format!("http://{addr}");

        let handle = rusty_tokio::spawn(async move {
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

            CapturedRequest {
                method: head.method.as_str().to_string(),
                target: head.target.clone(),
                body: String::from_utf8(body_bytes).expect("request body wasn't UTF-8"),
            }
        });

        (url, handle)
    }

    #[test]
    fn contract_subject_name_convention() {
        assert_eq!(
            contract_subject_name("billing-service", "order-events-value"),
            "billing-service.contracts.order-events-value"
        );
        assert_eq!(
            contract_subject_name("analytics", "user-activity-v1"),
            "analytics.contracts.user-activity-v1"
        );
    }

    #[rusty_tokio::test]
    async fn register_consumer_contract_returns_the_assigned_id() {
        let (url, server) = start_fake_server(200, r#"{"id": 7}"#);
        let id =
            register_consumer_contract(&url, "test-consumer.contracts.order-events-value", "{}")
                .await
                .unwrap();

        let request = server.await.unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.target,
            "/subjects/test-consumer.contracts.order-events-value/versions"
        );
        assert!(request.body.contains("\"schemaType\":\"AVRO\""));
        assert_eq!(id, 7);
    }

    #[rusty_tokio::test]
    async fn assert_schema_compatible_passes_when_registry_says_compatible() {
        let (url, server) = start_fake_server(200, r#"{"is_compatible": true}"#);
        assert_schema_compatible(&url, "{}", "my-contract")
            .await
            .unwrap();

        let request = server.await.unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.target,
            "/compatibility/subjects/my-contract/versions?verbose=true"
        );
    }

    #[rusty_tokio::test]
    async fn assert_schema_compatible_fails_with_registry_messages() {
        let (url, server) = start_fake_server(
            200,
            r#"{"is_compatible": false, "messages": ["field 'amount' removed"]}"#,
        );
        let err = assert_schema_compatible(&url, "{}", "my-contract")
            .await
            .unwrap_err();
        server.await.unwrap();

        match err {
            ContractGateError::Incompatible(subject, detail) => {
                assert_eq!(subject, "my-contract");
                assert!(detail.contains("field 'amount' removed"));
            }
            other => panic!("expected Incompatible, got {other:?}"),
        }
    }

    #[rusty_tokio::test]
    async fn assert_schema_compatible_falls_back_to_default_message_with_no_messages() {
        let (url, server) = start_fake_server(200, r#"{"is_compatible": false}"#);
        let err = assert_schema_compatible(&url, "{}", "my-contract")
            .await
            .unwrap_err();
        server.await.unwrap();

        match err {
            ContractGateError::Incompatible(_, detail) => {
                assert_eq!(detail, "Schema is incompatible");
            }
            other => panic!("expected Incompatible, got {other:?}"),
        }
    }

    #[rusty_tokio::test]
    async fn assert_schema_compatible_propagates_non_2xx_as_http_error() {
        let (url, server) = start_fake_server(500, r#"{"message": "internal error"}"#);
        let err = assert_schema_compatible(&url, "{}", "my-contract")
            .await
            .unwrap_err();
        server.await.unwrap();

        assert!(matches!(err, ContractGateError::Http(_)));
    }
}
