use std::time::Duration;

use rp_core::ProviderError;

/// Caps how much of an upstream response body this crate will buffer in
/// memory. A misbehaving or hostile endpoint that returns (or declares) an
/// arbitrarily large body must not be able to grow this process without
/// bound. Mirrors the `MAX_RESPONSE_BODY_BYTES` convention `rp-skillopt`'s
/// `skillopt-model::http::read_capped_body` already established for the
/// same purpose on its own chat backends.
pub(crate) const MAX_RESPONSE_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Default total-request timeout for a provider's `reqwest::Client`, used
/// by every adapter's `new()`. Generous on purpose -- a non-streaming
/// completion from a large or reasoning-heavy model, or a long-running
/// stream, can legitimately take minutes; this only needs to bound the
/// failure mode of a connection that hangs forever, not shave time off
/// normal slow responses. Overridable per-provider via `with_timeout`
/// (wired to `[[providers]].timeout_secs` in `rp-router`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Builds a `reqwest::Client` with `timeout` as its total per-request
/// timeout (covers connecting, sending, and reading the full response --
/// including a streamed one). Panics only if the underlying TLS backend
/// fails to initialize, which `reqwest::Client::new()` (used everywhere
/// before this) would also have panicked on.
pub fn build_client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .expect("reqwest client should build with a timeout configured")
}

/// Reads `resp`'s body into `Bytes`, refusing to buffer past
/// [`MAX_RESPONSE_BODY_BYTES`]. Replaces `resp.text().await`/`resp.json().await`,
/// which collect the whole body no matter how large the endpoint makes it.
///
/// Checked twice: once against a declared `Content-Length` before reading
/// anything (the fast path for a well-behaved but oversized response), and
/// once against a running total while streaming chunks (for a response
/// with no -- or a lying -- `Content-Length`).
pub(crate) async fn read_capped_body(
    resp: reqwest::Response,
) -> Result<bytes::Bytes, ProviderError> {
    if let Some(len) = resp.content_length() {
        if len > MAX_RESPONSE_BODY_BYTES as u64 {
            return Err(ProviderError::Decode(format!(
                "response declared Content-Length {len}, exceeding the {MAX_RESPONSE_BODY_BYTES}-byte cap"
            )));
        }
    }

    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(map_reqwest_error)? {
        if buf.len() + chunk.len() > MAX_RESPONSE_BODY_BYTES {
            return Err(ProviderError::Decode(format!(
                "response body exceeded the {MAX_RESPONSE_BODY_BYTES}-byte cap"
            )));
        }
        buf.extend_from_slice(&chunk);
    }

    Ok(bytes::Bytes::from(buf))
}

/// Turn a non-2xx reqwest response into a classified `ProviderError`,
/// consuming the body (capped at [`MAX_RESPONSE_BODY_BYTES`]) for the error
/// message.
pub async fn map_error_response(resp: reqwest::Response) -> ProviderError {
    let status = resp.status();
    let retry_after_secs = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let body = match read_capped_body(resp).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(err) => return err,
    };
    let message = extract_error_message(&body).unwrap_or(body);

    match status.as_u16() {
        401 | 403 => ProviderError::Auth(message),
        400 | 404 | 422 => ProviderError::InvalidRequest(message),
        429 => ProviderError::RateLimited { retry_after_secs },
        s => ProviderError::Upstream { status: s, message },
    }
}

/// Best-effort extraction of a human-readable message from a provider's
/// JSON error body. Providers disagree on the exact shape, so this tries
/// the common spots and falls back to the raw body.
fn extract_error_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    value
        .get("error")
        .and_then(|e| e.get("message").or(Some(e)))
        .and_then(|m| m.as_str().map(str::to_owned))
        .or_else(|| {
            value
                .get("message")
                .and_then(|m| m.as_str().map(str::to_owned))
        })
}

pub fn map_reqwest_error(err: reqwest::Error) -> ProviderError {
    if err.is_timeout() {
        ProviderError::Timeout
    } else {
        // `reqwest::Error`'s `Display` impl can embed the full request URL
        // (query string included), which for a provider like Gemini means
        // the `?key=...` API key would otherwise leak into a client-facing
        // error response. `without_url()` strips it before formatting.
        ProviderError::Network(err.without_url().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // --- build_client --------------------------------------------------------

    #[tokio::test]
    async fn build_client_times_out_a_request_that_outlasts_the_configured_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(200)))
            .mount(&server)
            .await;

        let client = build_client(Duration::from_millis(20));
        let err = client.get(server.uri()).send().await.unwrap_err();
        assert!(err.is_timeout());
        assert!(matches!(map_reqwest_error(err), ProviderError::Timeout));
    }

    #[tokio::test]
    async fn build_client_succeeds_within_the_configured_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = build_client(Duration::from_secs(5));
        let resp = client.get(server.uri()).send().await.unwrap();
        assert!(resp.status().is_success());
    }

    // --- read_capped_body / map_error_response ----------------------------------

    /// Spawns a one-shot raw TCP server that accepts a single request and
    /// replies with `status_line` plus an oversized declared
    /// `Content-Length` and no actual body -- exercising the fast
    /// Content-Length-based rejection path without ever having to
    /// transfer (or buffer) gigabytes of data.
    fn oversized_content_length_server(
        status_line: &'static str,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let oversized_len = MAX_RESPONSE_BODY_BYTES as u64 + 1;
            let response = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {oversized_len}\r\n\r\n"
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        });
        (addr, handle)
    }

    #[tokio::test]
    async fn read_capped_body_rejects_a_response_with_an_oversized_declared_content_length() {
        let (addr, server) = oversized_content_length_server("HTTP/1.1 200 OK");
        let client = build_client(Duration::from_secs(5));
        let resp = client.get(format!("http://{addr}")).send().await.unwrap();

        let err = read_capped_body(resp).await.unwrap_err();
        assert!(
            err.to_string().contains("cap"),
            "expected an over-cap rejection, got: {err}"
        );
        let _ = server.join();
    }

    #[tokio::test]
    async fn map_error_response_rejects_an_oversized_body_instead_of_buffering_it() {
        let (addr, server) = oversized_content_length_server("HTTP/1.1 500 Internal Server Error");
        let client = build_client(Duration::from_secs(5));
        let resp = client.get(format!("http://{addr}")).send().await.unwrap();

        let err = map_error_response(resp).await;
        assert!(
            err.to_string().contains("cap"),
            "expected an over-cap rejection, got: {err}"
        );
        let _ = server.join();
    }

    #[test]
    fn extract_error_message_from_nested_error_message_field() {
        assert_eq!(
            extract_error_message(r#"{"error":{"message":"bad request"}}"#),
            Some("bad request".to_string())
        );
    }

    #[test]
    fn extract_error_message_from_a_plain_string_error_field() {
        assert_eq!(
            extract_error_message(r#"{"error":"overloaded"}"#),
            Some("overloaded".to_string())
        );
    }

    #[test]
    fn extract_error_message_from_a_top_level_message_field() {
        assert_eq!(
            extract_error_message(r#"{"message":"invalid api key"}"#),
            Some("invalid api key".to_string())
        );
    }

    #[test]
    fn extract_error_message_prefers_error_over_top_level_message() {
        assert_eq!(
            extract_error_message(r#"{"error":{"message":"from error"},"message":"from top"}"#),
            Some("from error".to_string())
        );
    }

    #[test]
    fn extract_error_message_is_none_for_json_with_neither_recognized_shape() {
        assert_eq!(extract_error_message(r#"{"foo":"bar"}"#), None);
    }

    #[test]
    fn extract_error_message_is_none_for_non_json_body() {
        assert_eq!(extract_error_message("not json at all"), None);
    }

    #[test]
    fn extract_error_message_is_none_for_an_empty_body() {
        assert_eq!(extract_error_message(""), None);
    }

    // --- map_reqwest_error -----------------------------------------------------

    #[tokio::test]
    async fn map_reqwest_error_strips_the_api_key_from_the_request_url() {
        // A connection failure to an unreachable port surfaces a
        // `reqwest::Error` whose `Display` embeds the full request URL --
        // including any query-string API key, e.g. Gemini's `?key=...`.
        let client = build_client(Duration::from_secs(5));
        let err = client
            .get("http://127.0.0.1:1/v1/models?key=SECRET")
            .send()
            .await
            .unwrap_err();
        // Sanity check: the unmodified error really would have leaked it.
        assert!(err.to_string().contains("SECRET"));

        let mapped = map_reqwest_error(err);
        let message = match mapped {
            ProviderError::Network(message) => message,
            other => panic!("expected ProviderError::Network, got {other:?}"),
        };
        assert!(!message.contains("SECRET"), "leaked API key: {message}");
    }
}
