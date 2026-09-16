//! The opt-in HTTP scrape listener (`MHTTP-FR-001`–`005`, ADR-0069).
//! Uses `rusty_http`'s synchronous message layer for one request per
//! connection, independently of the binary wire protocol. Reads the
//! shared counters verbatim; HTTP accepts and scrapes never count as
//! wire-protocol connections or requests. No TLS or authentication is
//! applied on this separate listener, as accepted in option (a).

use super::ServeOptions;
use rusty_http::head::{ResponseHead, DEFAULT_MAX_HEAD_LEN};
use rusty_http::sync::SyncTransport;
use rusty_http::{HeaderMap, Method, StatusCode, Version};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

/// `MHTTP-FR-001`/`004` (ADR-0069): accept on an already-bound listener,
/// serving each connection on its own OS thread against the identical
/// `ServeOptions` shared by the wire listener. A failed accept is skipped,
/// matching `serve_tables`; this loop runs for the listener's lifetime.
pub(super) fn serve_metrics_http(listener: TcpListener, options: Arc<ServeOptions>) {
    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(s) => s,
            Err(_) => continue, // one bad accept doesn't take down the server
        };
        let options = Arc::clone(&options);
        thread::spawn(move || handle_metrics_http_connection(stream, options.as_ref()));
    }
}

/// `MHTTP-FR-002`–`005`: one HTTP request, one best-effort response,
/// then close. A malformed, oversized, or truncated head gets no reply.
/// Only `GET /metrics` reads the counters; every other method/path gets
/// an empty `404`. No path increments any wire-protocol metric.
fn handle_metrics_http_connection(stream: TcpStream, options: &ServeOptions) {
    let _ = stream.set_nodelay(true);
    let mut transport = SyncTransport::new(stream);
    let head = match transport.read_request_head(DEFAULT_MAX_HEAD_LEN) {
        Ok(head) => head,
        Err(_) => return,
    };
    let (status, reason, body) = match (head.method, head.target.as_str()) {
        (Method::Get, "/metrics") => (StatusCode::OK, "OK", options.metrics().render()),
        _ => (StatusCode::NOT_FOUND, "Not Found", String::new()),
    };
    let mut headers = HeaderMap::new();
    // These fixed names/values (and a decimal byte length) are valid;
    // still treat a header-construction error as a clean close.
    if status == StatusCode::OK
        && headers
            .insert("Content-Type", "text/plain; version=0.0.4")
            .is_err()
    {
        return;
    }
    if headers
        .insert("Content-Length", &body.len().to_string())
        .is_err()
        || headers.insert("Connection", "close").is_err()
    {
        return;
    }
    let response = ResponseHead {
        status,
        reason: reason.into(),
        version: Version::Http11,
        headers,
    };
    let _ = transport.write_response_head(&response);
    let _ = transport.write_body(body.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_http::head::{parse_response_head, Outcome};
    use std::io::{Read, Write};
    use std::net::Shutdown;
    use std::time::Duration;

    fn exchange(request: &[u8], options: Arc<ServeOptions>) -> Vec<u8> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let handler = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_metrics_http_connection(stream, &options);
        });
        client.write_all(request).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut response = Vec::new();
        let result = client.read_to_end(&mut response);
        // Closing with unread bytes can reset the socket on some hosts.
        if let Err(error) = result {
            assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
        }
        handler.join().unwrap(); // a bad client must never panic the handler
        response
    }

    fn response_parts(bytes: &[u8]) -> (ResponseHead, &str) {
        match parse_response_head(bytes, DEFAULT_MAX_HEAD_LEN).unwrap() {
            Outcome::Complete { head, consumed } => {
                (head, std::str::from_utf8(&bytes[consumed..]).unwrap())
            }
            Outcome::Incomplete => panic!("incomplete response"),
        }
    }

    #[test]
    fn get_metrics_returns_verbatim_counters_and_exact_headers() {
        // Wire credentials do not gate this independent HTTP listener.
        let options = Arc::new(ServeOptions::new(Some("ro".into()), Some("rw".into())));
        options.metrics().record_connection_opened();
        options.metrics().record_request(true);
        options.metrics().record_request(false);
        let before = options.metrics().render();
        let response = exchange(
            b"GET /metrics HTTP/1.1\r\nHost: x\r\n\r\n",
            Arc::clone(&options),
        );
        let after = options.metrics().render();
        let (head, body) = response_parts(&response);
        assert_eq!(head.version, Version::Http11);
        assert_eq!(head.status, StatusCode::OK);
        assert_eq!(head.reason, "OK");
        assert_eq!(
            head.headers.get("Content-Type"),
            Some("text/plain; version=0.0.4")
        );
        assert_eq!(
            head.headers.get("Content-Length"),
            Some(body.len().to_string().as_str())
        );
        assert_eq!(head.headers.get("Connection"), Some("close"));
        // Only uptime can change between render calls, even if the test
        // thread is descheduled across a second boundary. Compare every
        // other byte and bound the observed uptime by the two renders.
        let split = |text: &str| {
            let (prefix, uptime) = text.rsplit_once("dogserver_uptime_seconds ").unwrap();
            (prefix.to_string(), uptime.trim().parse::<u64>().unwrap())
        };
        let (expected, first) = split(&before);
        let (actual, observed) = split(body);
        let (unchanged, last) = split(&after);
        assert_eq!(actual, expected);
        assert_eq!(unchanged, expected, "scraping must not mutate counters");
        assert!((first..=last).contains(&observed));
        assert_eq!(
            body,
            format!("{expected}dogserver_uptime_seconds {observed}\n")
        );
    }

    #[test]
    fn other_methods_and_paths_return_empty_not_found() {
        for request in [
            "GET / HTTP/1.1\r\n\r\n",
            "POST /metrics HTTP/1.1\r\n\r\n",
            "HEAD /metrics HTTP/1.1\r\n\r\n",
            "GET /metrics?x=1 HTTP/1.1\r\n\r\n",
        ] {
            let response = exchange(request.as_bytes(), Arc::new(ServeOptions::default()));
            let (head, body) = response_parts(&response);
            assert_eq!(head.status, StatusCode::NOT_FOUND);
            assert_eq!(head.reason, "Not Found");
            assert_eq!(head.headers.get("Content-Length"), Some("0"));
            assert_eq!(head.headers.get("Connection"), Some("close"));
            assert!(body.is_empty());
        }
    }

    #[test]
    fn malformed_head_closes_without_reply_or_panic() {
        assert!(exchange(b"garbage\r\n\r\n", Arc::new(ServeOptions::default())).is_empty());
    }

    #[test]
    fn oversized_head_closes_without_reply_or_panic() {
        let request = vec![b'x'; DEFAULT_MAX_HEAD_LEN + 1];
        assert!(exchange(&request, Arc::new(ServeOptions::default())).is_empty());
    }

    #[test]
    fn truncated_head_closes_without_reply_or_panic() {
        assert!(exchange(
            b"GET /metrics HTTP/1.1\r\nHost:",
            Arc::new(ServeOptions::default())
        )
        .is_empty());
    }
}
