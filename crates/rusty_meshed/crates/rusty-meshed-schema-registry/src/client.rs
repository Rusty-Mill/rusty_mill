//! [`SchemaRegistryEnforcer`]: a Schema Registry client wrapper that
//! enforces `FULL_TRANSITIVE` compatibility -- the Rust port of
//! `meshed.schema_registry.client`.

use crate::models::{CompatibilityMode, CompatibilityViolation};
use rusty_err::Error;
use rusty_request::{Client, Json};

/// Errors from a Schema Registry HTTP call that aren't a compatibility
/// violation (that's [`CompatibilityViolation`], handled separately by
/// [`SchemaRegistryEnforcer::register_schema`]).
#[derive(Debug, Error)]
pub enum SchemaRegistryError {
    /// The HTTP request itself failed (connection, TLS, non-2xx
    /// status via [`rusty_request::Response::error_for_status`], ...).
    #[error("Schema Registry request failed: {0}")]
    Http(#[from] rusty_request::Error),
    /// The response's JSON body didn't have the shape this call
    /// expected.
    #[error("unexpected Schema Registry response: {0}")]
    UnexpectedResponse(String),
}

/// Raised by [`SchemaRegistryEnforcer::set_subject_compatibility`] when
/// `mode` isn't one of the 7 valid [`CompatibilityMode`] wire values.
#[derive(Debug, Error)]
pub enum SetCompatibilityError {
    /// `{0}` is the raw string that was passed; `{1}` is the
    /// Python-list-repr-formatted list of valid modes, matching the
    /// source's own error message shape exactly.
    #[error("Unknown compatibility mode: '{0}'. Valid modes: {1}")]
    InvalidMode(String, String),
    /// The registry call itself failed, once past mode validation.
    #[error("{0}")]
    Request(#[from] SchemaRegistryError),
}

/// Raised by [`SchemaRegistryEnforcer::register_schema`]: either the
/// registry rejected the schema as incompatible, or the call itself
/// failed for some other reason.
#[derive(Debug, Error)]
pub enum RegisterSchemaError {
    /// The registry rejected the schema (HTTP 409) as violating the
    /// configured compatibility mode.
    #[error("{0}")]
    Compatibility(#[from] CompatibilityViolation),
    /// Any other registry-call failure.
    #[error("{0}")]
    Request(#[from] SchemaRegistryError),
}

fn valid_modes_list() -> String {
    let parts: Vec<String> = CompatibilityMode::ALL
        .iter()
        .map(|mode| format!("'{}'", mode.as_str()))
        .collect();
    format!("[{}]", parts.join(", "))
}

/// Schema Registry client wrapper that enforces `FULL_TRANSITIVE`
/// compatibility. Raises [`CompatibilityViolation`] with a clear error
/// message when a schema publish would violate the configured
/// compatibility mode; non-compatibility errors propagate unchanged so
/// callers can distinguish infrastructure failures from governance
/// rejections.
pub struct SchemaRegistryEnforcer {
    client: Client,
    base_url: String,
}

impl SchemaRegistryEnforcer {
    /// The compatibility mode this enforcer initializes the registry
    /// to via [`initialize_global_compatibility`](Self::initialize_global_compatibility).
    pub const DEFAULT_COMPATIBILITY: CompatibilityMode = CompatibilityMode::FullTransitive;

    /// Builds an enforcer talking to the Schema Registry at `url` (e.g.
    /// `"http://localhost:8081"`) over a fresh [`rusty_request::Client`].
    pub fn new(url: impl Into<String>) -> Self {
        SchemaRegistryEnforcer::with_client(url, Client::new())
    }

    /// Builds an enforcer with an already-constructed
    /// [`rusty_request::Client`] -- the seam this crate's own tests use
    /// to point at a local fake server instead of a real registry.
    pub fn with_client(url: impl Into<String>, client: Client) -> Self {
        let base_url = url.into();
        let base_url = base_url
            .strip_suffix('/')
            .map(str::to_string)
            .unwrap_or(base_url);
        SchemaRegistryEnforcer { client, base_url }
    }

    /// Sets the global compatibility level to
    /// [`DEFAULT_COMPATIBILITY`](Self::DEFAULT_COMPATIBILITY). Must be
    /// called once after the Schema Registry starts to ensure no
    /// subject is ever registered under the registry's weaker default.
    pub async fn initialize_global_compatibility(&self) -> Result<(), SchemaRegistryError> {
        self.set_compatibility(None, Self::DEFAULT_COMPATIBILITY)
            .await
    }

    /// Overrides the compatibility level for a single subject. Accepts
    /// the mode as a raw string (matching the Python source's dynamic
    /// coercion of a caller-supplied value) and validates it *before*
    /// making any registry call.
    pub async fn set_subject_compatibility(
        &self,
        subject: &str,
        mode: &str,
    ) -> Result<(), SetCompatibilityError> {
        let mode = CompatibilityMode::parse(mode).ok_or_else(|| {
            SetCompatibilityError::InvalidMode(mode.to_string(), valid_modes_list())
        })?;
        self.set_compatibility(Some(subject), mode).await?;
        Ok(())
    }

    async fn set_compatibility(
        &self,
        subject: Option<&str>,
        mode: CompatibilityMode,
    ) -> Result<(), SchemaRegistryError> {
        let path = match subject {
            Some(subject) => format!("{}/config/{subject}", self.base_url),
            None => format!("{}/config", self.base_url),
        };
        let mut body = Json::object();
        body.insert("compatibility", mode.as_str());
        let response = self.client.put(&path)?.json(&body)?.send().await?;
        response.error_for_status()?;
        Ok(())
    }

    /// Returns the current compatibility level for a subject, as
    /// reported by the registry.
    pub async fn get_subject_compatibility(
        &self,
        subject: &str,
    ) -> Result<String, SchemaRegistryError> {
        let path = format!("{}/config/{subject}", self.base_url);
        let response = self.client.get(&path)?.send().await?;
        let response = response.error_for_status()?;
        let body = response.json()?;
        body.get("compatibilityLevel")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                SchemaRegistryError::UnexpectedResponse(
                    "missing 'compatibilityLevel' field".to_string(),
                )
            })
    }

    /// Registers an Avro schema under the given subject. Returns the
    /// integer schema ID assigned by the registry, or a
    /// [`CompatibilityViolation`] if the registry rejects the schema
    /// (HTTP 409) as incompatible with the configured compatibility
    /// mode.
    pub async fn register_schema(
        &self,
        subject: &str,
        schema_str: &str,
    ) -> Result<i64, RegisterSchemaError> {
        let path = format!("{}/subjects/{subject}/versions", self.base_url);
        let mut body = Json::object();
        body.insert("schema", schema_str);
        body.insert("schemaType", "AVRO");
        let response = self
            .client
            .post(&path)
            .map_err(SchemaRegistryError::from)?
            .json(&body)
            .map_err(SchemaRegistryError::from)?
            .send()
            .await
            .map_err(SchemaRegistryError::from)?;

        if response.status().as_u16() == 409 {
            let message = response
                .json()
                .ok()
                .and_then(|body| {
                    body.get("message")
                        .and_then(|m| m.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| {
                    "Schema Registry rejected the schema as incompatible".to_string()
                });
            return Err(CompatibilityViolation::new(subject, message).into());
        }

        let response = response
            .error_for_status()
            .map_err(SchemaRegistryError::from)?;
        let body = response.json().map_err(SchemaRegistryError::from)?;
        let id = body
            .get("id")
            .and_then(|value| value.as_f64())
            .ok_or_else(|| {
                SchemaRegistryError::UnexpectedResponse("missing 'id' field".to_string())
            })?;
        Ok(id as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_http::async_tokio::AsyncTransport;
    use rusty_http::head::ResponseHead;
    use rusty_http::{HeaderMap, StatusCode, Version};
    use rusty_tokio::io::TcpListener;

    /// One captured request the fake server received.
    struct CapturedRequest {
        method: String,
        target: String,
        body: String,
    }

    /// Binds an ephemeral local listener, spawns a task that accepts
    /// exactly one connection and serves exactly one request/response
    /// exchange, and returns the base URL plus a handle to await the
    /// captured request once the client side has made its call.
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

    #[rusty_tokio::test]
    async fn initialize_global_compatibility_puts_full_transitive_to_config() {
        let (url, server) = start_fake_server(200, "{}");
        let enforcer = SchemaRegistryEnforcer::new(&url);
        enforcer.initialize_global_compatibility().await.unwrap();

        let request = server.await.unwrap();
        assert_eq!(request.method, "PUT");
        assert_eq!(request.target, "/config");
        assert!(request.body.contains("FULL_TRANSITIVE"));
    }

    #[rusty_tokio::test]
    async fn set_subject_compatibility_puts_to_the_subject_specific_path() {
        let (url, server) = start_fake_server(200, "{}");
        let enforcer = SchemaRegistryEnforcer::new(&url);
        enforcer
            .set_subject_compatibility("my-subject", "FORWARD_TRANSITIVE")
            .await
            .unwrap();

        let request = server.await.unwrap();
        assert_eq!(request.method, "PUT");
        assert_eq!(request.target, "/config/my-subject");
        assert!(request.body.contains("FORWARD_TRANSITIVE"));
    }

    #[rusty_tokio::test]
    async fn set_subject_compatibility_rejects_invalid_mode_before_any_request() {
        // No server started at all -- if this made an HTTP call it
        // would hang waiting for a connection nothing accepts.
        let enforcer = SchemaRegistryEnforcer::new("http://127.0.0.1:1");
        let err = enforcer
            .set_subject_compatibility("my-subject", "INVALID")
            .await
            .unwrap_err();
        match err {
            SetCompatibilityError::InvalidMode(mode, valid) => {
                assert_eq!(mode, "INVALID");
                assert!(valid.contains("'BACKWARD'"));
                assert!(valid.contains("'FULL_TRANSITIVE'"));
            }
            SetCompatibilityError::Request(_) => {
                panic!("expected InvalidMode, not a request error")
            }
        }
    }

    #[rusty_tokio::test]
    async fn get_subject_compatibility_reads_compatibility_level() {
        let (url, server) =
            start_fake_server(200, r#"{"compatibilityLevel": "FORWARD_TRANSITIVE"}"#);
        let enforcer = SchemaRegistryEnforcer::new(&url);
        let level = enforcer
            .get_subject_compatibility("my-subject")
            .await
            .unwrap();

        let request = server.await.unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.target, "/config/my-subject");
        assert_eq!(level, "FORWARD_TRANSITIVE");
    }

    #[rusty_tokio::test]
    async fn register_schema_returns_the_assigned_id() {
        let (url, server) = start_fake_server(200, r#"{"id": 42}"#);
        let enforcer = SchemaRegistryEnforcer::new(&url);
        let id = enforcer
            .register_schema(
                "my-subject",
                r#"{"type":"record","name":"Test","fields":[]}"#,
            )
            .await
            .unwrap();

        let request = server.await.unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/subjects/my-subject/versions");
        assert!(request.body.contains("\"schemaType\":\"AVRO\""));
        assert_eq!(id, 42);
    }

    #[rusty_tokio::test]
    async fn register_schema_maps_409_to_compatibility_violation() {
        let (url, server) = start_fake_server(
            409,
            r#"{"error_code": 409, "message": "Schema being registered is incompatible"}"#,
        );
        let enforcer = SchemaRegistryEnforcer::new(&url);
        let err = enforcer
            .register_schema("my-subject", "{}")
            .await
            .unwrap_err();
        server.await.unwrap();

        match err {
            RegisterSchemaError::Compatibility(violation) => {
                assert_eq!(violation.subject(), "my-subject");
                assert!(violation.message().contains("incompatible"));
            }
            RegisterSchemaError::Request(_) => {
                panic!("expected Compatibility, not a request error")
            }
        }
    }

    #[rusty_tokio::test]
    async fn register_schema_propagates_non_409_errors_unchanged() {
        let (url, server) = start_fake_server(500, r#"{"message": "internal error"}"#);
        let enforcer = SchemaRegistryEnforcer::new(&url);
        let err = enforcer
            .register_schema("my-subject", "{}")
            .await
            .unwrap_err();
        server.await.unwrap();

        assert!(matches!(err, RegisterSchemaError::Request(_)));
    }
}
