use std::time::Duration;

use rp_core::ProviderError;
use rusty_tokio::time::{timeout_at, Instant};
use serde::Serialize;

/// Caps how much of an upstream response body this crate will buffer in
/// memory. A misbehaving or hostile endpoint that returns (or declares) an
/// arbitrarily large body must not be able to grow this process without
/// bound. Mirrors the `MAX_RESPONSE_BODY_BYTES` convention `rp-skillopt`'s
/// `skillopt-model::http::read_capped_body` already established for the
/// same purpose on its own chat backends.
pub(crate) const MAX_RESPONSE_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Default total-request timeout for a provider's [`HttpClient`], used
/// by every adapter's `new()`. Generous on purpose -- a non-streaming
/// completion from a large or reasoning-heavy model, or a long-running
/// stream, can legitimately take minutes; this only needs to bound the
/// failure mode of a connection that hangs forever, not shave time off
/// normal slow responses. Overridable per-provider via `with_timeout`
/// (wired to `[[providers]].timeout_secs` in `rp-router`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// A provider's outbound HTTP client, on `rusty_request`.
///
/// `timeout` bounds the whole exchange -- connecting, sending, and reading
/// the full response, a streamed one included -- not just the wait for
/// headers, which is all `rusty_request`'s own timeout covers for a
/// streamed response.
///
/// `rusty_request` runs on `rusty_tokio`: a host that calls the providers
/// from a tokio runtime must call `rusty_tokio::runtime::install_fallback()`
/// once at startup.
#[derive(Clone)]
pub(crate) struct HttpClient {
    inner: rusty_request::Client,
    timeout: Duration,
}

impl HttpClient {
    pub(crate) fn new(timeout: Duration) -> Self {
        let inner = rusty_request::Client::builder()
            .timeout(timeout)
            .no_cookie_store()
            .build();
        Self { inner, timeout }
    }

    /// POSTs `body` as JSON to `url` with `query` and `headers`, returning
    /// as soon as the response's headers arrive.
    pub(crate) async fn post_json<B: Serialize + ?Sized>(
        &self,
        url: &str,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
        body: &B,
    ) -> Result<Upstream, ProviderError> {
        let deadline = Instant::now() + self.timeout;
        let body = serde_json::to_vec(body)
            .map_err(|e| ProviderError::Network(format!("failed to encode the request: {e}")))?;
        let mut req = self
            .inner
            .post(url)
            .map_err(map_request_error)?
            .query(query.iter().copied())
            .header("Content-Type", "application/json")
            .map_err(map_request_error)?;
        for (name, value) in headers {
            req = req.header(name, value).map_err(map_request_error)?;
        }
        let resp = req
            .body(body)
            .send_streaming()
            .await
            .map_err(map_request_error)?;
        Ok(Upstream { resp, deadline })
    }
}

/// A response whose body is still to be read, before the exchange's deadline.
pub(crate) struct Upstream {
    resp: rusty_request::StreamingResponse,
    deadline: Instant,
}

impl Upstream {
    pub(crate) fn is_success(&self) -> bool {
        self.resp.status().is_success()
    }

    /// The next chunk of the body, `None` at its end.
    async fn chunk(&mut self) -> Result<Option<Vec<u8>>, ProviderError> {
        // `timeout_at` polls the read first, so a passed deadline alone
        // would not stop an endpoint whose bytes keep arriving.
        if Instant::now() >= self.deadline {
            return Err(ProviderError::Timeout);
        }
        match timeout_at(self.deadline, self.resp.chunk()).await {
            Ok(chunk) => chunk.map_err(map_request_error),
            Err(_) => Err(ProviderError::Timeout),
        }
    }

    /// The body as a `futures` byte stream, for the SSE decoder.
    pub(crate) fn into_byte_stream(
        self,
    ) -> impl futures_util::Stream<Item = Result<Vec<u8>, ProviderError>> + Send {
        futures_util::stream::unfold(Some(self), |state| async move {
            let mut upstream = state?;
            match upstream.chunk().await {
                Ok(Some(chunk)) => Some((Ok(chunk), Some(upstream))),
                Ok(None) => None,
                // End the stream after an error: the connection is unusable.
                Err(e) => Some((Err(e), None)),
            }
        })
    }

    fn content_length(&self) -> Option<u64> {
        self.resp
            .headers()
            .get("Content-Length")?
            .trim()
            .parse()
            .ok()
    }
}

/// Reads `resp`'s body, refusing to buffer past [`MAX_RESPONSE_BODY_BYTES`].
///
/// Checked twice: once against a declared `Content-Length` before reading
/// anything (the fast path for a well-behaved but oversized response), and
/// once against a running total while reading chunks (for a response
/// with no -- or a lying -- `Content-Length`).
pub(crate) async fn read_capped_body(mut resp: Upstream) -> Result<Vec<u8>, ProviderError> {
    if let Some(len) = resp.content_length() {
        if len > MAX_RESPONSE_BODY_BYTES as u64 {
            return Err(ProviderError::Decode(format!(
                "response declared Content-Length {len}, exceeding the {MAX_RESPONSE_BODY_BYTES}-byte cap"
            )));
        }
    }

    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if buf.len() + chunk.len() > MAX_RESPONSE_BODY_BYTES {
            return Err(ProviderError::Decode(format!(
                "response body exceeded the {MAX_RESPONSE_BODY_BYTES}-byte cap"
            )));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Turn a non-2xx response into a classified `ProviderError`, consuming
/// the body (capped at [`MAX_RESPONSE_BODY_BYTES`]) for the error message.
pub(crate) async fn map_error_response(resp: Upstream) -> ProviderError {
    let status = resp.resp.status().as_u16();
    let retry_after_secs = resp
        .resp
        .headers()
        .get("Retry-After")
        .and_then(|v| v.trim().parse::<u64>().ok());
    let body = match read_capped_body(resp).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(err) => return err,
    };
    let message = extract_error_message(&body).unwrap_or(body);

    match status {
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

/// Classifies a transport error. Never echoes the request URL: Gemini
/// carries its API key in the query string, and these messages reach the
/// client.
fn map_request_error(err: rusty_request::Error) -> ProviderError {
    match err {
        rusty_request::Error::Timeout => ProviderError::Timeout,
        rusty_request::Error::InvalidUrl(_) => {
            ProviderError::Network("invalid provider URL".to_string())
        }
        other => ProviderError::Network(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(timeout: Duration) -> HttpClient {
        rusty_tokio::runtime::install_fallback().expect("fallback runtime should start");
        HttpClient::new(timeout)
    }

    async fn post(client: &HttpClient, url: &str) -> Result<Upstream, ProviderError> {
        client
            .post_json(url, &[], &[], &serde_json::json!({}))
            .await
    }

    // --- HttpClient ------------------------------------------------------------

    #[tokio::test]
    async fn post_json_times_out_a_request_that_outlasts_the_configured_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(200)))
            .mount(&server)
            .await;

        let err = post(&client(Duration::from_millis(20)), &server.uri())
            .await
            .err()
            .expect("the request should time out");
        assert!(matches!(err, ProviderError::Timeout), "got {err:?}");
    }

    #[tokio::test]
    async fn post_json_succeeds_within_the_configured_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let resp = post(&client(Duration::from_secs(5)), &server.uri())
            .await
            .unwrap();
        assert!(resp.is_success());
        assert_eq!(read_capped_body(resp).await.unwrap(), b"ok");
    }

    /// Spawns a one-shot raw TCP server that accepts a single request and
    /// replies with `head` (status line plus headers, through the blank
    /// line), then holds the connection open for `hold` without sending
    /// anything more.
    fn raw_server(head: String, hold: Duration) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            let _ = std::io::Write::write_all(&mut stream, head.as_bytes());
            std::thread::sleep(hold);
        });
        (format!("http://{addr}"), handle)
    }

    /// Reads one whole HTTP/1.1 request (headers plus a `Content-Length`
    /// body) off `stream`. A server that closes with request bytes still
    /// unread makes Windows reset the connection, which can discard the
    /// response before the client reads it.
    fn read_request(stream: &mut std::net::TcpStream) {
        use std::io::Read;
        let mut request = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let body_len: usize = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                if request.len() >= end + 4 + body_len {
                    return;
                }
            }
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => request.extend_from_slice(&buf[..n]),
            }
        }
    }

    #[tokio::test]
    async fn the_timeout_also_bounds_reading_a_body_that_stalls() {
        let head = "HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc".to_string();
        let (url, server) = raw_server(head, Duration::from_millis(500));
        let resp = post(&client(Duration::from_millis(100)), &url)
            .await
            .unwrap();

        let err = read_capped_body(resp).await.unwrap_err();
        assert!(matches!(err, ProviderError::Timeout), "got {err:?}");
        let _ = server.join();
    }

    // --- read_capped_body / map_error_response ----------------------------------

    fn oversized_head(status_line: &str) -> String {
        let oversized_len = MAX_RESPONSE_BODY_BYTES as u64 + 1;
        format!(
            "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {oversized_len}\r\n\r\n"
        )
    }

    #[tokio::test]
    async fn read_capped_body_rejects_a_response_with_an_oversized_declared_content_length() {
        let (url, server) = raw_server(oversized_head("HTTP/1.1 200 OK"), Duration::ZERO);
        let resp = post(&client(Duration::from_secs(5)), &url).await.unwrap();

        let err = read_capped_body(resp).await.unwrap_err();
        assert!(
            err.to_string().contains("cap"),
            "expected an over-cap rejection, got: {err}"
        );
        let _ = server.join();
    }

    #[tokio::test]
    async fn map_error_response_rejects_an_oversized_body_instead_of_buffering_it() {
        let head = oversized_head("HTTP/1.1 500 Internal Server Error");
        let (url, server) = raw_server(head, Duration::ZERO);
        let resp = post(&client(Duration::from_secs(5)), &url).await.unwrap();

        let err = map_error_response(resp).await;
        assert!(
            err.to_string().contains("cap"),
            "expected an over-cap rejection, got: {err}"
        );
        let _ = server.join();
    }

    #[tokio::test]
    async fn map_error_response_reads_retry_after_on_a_429() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "7"))
            .mount(&server)
            .await;

        let resp = post(&client(Duration::from_secs(5)), &server.uri())
            .await
            .unwrap();
        assert!(matches!(
            map_error_response(resp).await,
            ProviderError::RateLimited {
                retry_after_secs: Some(7)
            }
        ));
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

    // --- map_request_error ------------------------------------------------------

    #[tokio::test]
    async fn a_connection_failure_does_not_leak_the_api_key_in_the_url() {
        let client = client(Duration::from_secs(5));
        let err = client
            .post_json(
                "http://127.0.0.1:1/v1/models",
                &[("key", "SECRET")],
                &[],
                &(),
            )
            .await
            .err()
            .expect("nothing listens on port 1");
        let message = match err {
            ProviderError::Network(message) => message,
            other => panic!("expected ProviderError::Network, got {other:?}"),
        };
        assert!(!message.contains("SECRET"), "leaked API key: {message}");
    }

    #[test]
    fn an_invalid_url_error_does_not_echo_the_url() {
        let err = map_request_error(rusty_request::Error::InvalidUrl(
            "http://x y/?key=SECRET".to_string(),
        ));
        assert!(!err.to_string().contains("SECRET"), "leaked: {err}");
    }
}
