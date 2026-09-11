//! Web tools (PRD 03; Phase 6) — opt-in behind `RUSTYKEYS_ALLOW_WEB`, with an
//! SSRF / egress guard. The guard is the load-bearing security piece: before any
//! request, the URL's host is resolved and **every** resolved IP must be public
//! — loopback, private, link-local (incl. the cloud-metadata `169.254.169.254`),
//! and unspecified addresses are denied. (v1: a small TOCTOU window remains
//! between resolve and connect; documented, hardened later via connect-to-IP.)

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};

use crate::error::ToolError;
use crate::tool::{AiSdkTool, ToolRegistry};
use serde_json::Value;

const FETCH_CAP: usize = 50_000;

/// Hard cap on bytes read from the wire before HTML stripping, independent of
/// any `Content-Length` header (which may be absent for a chunked response,
/// or simply not trusted). Well above `FETCH_CAP` since markup inflates raw
/// size relative to the stripped text, but still bounded so an
/// adversarial/huge response cannot force buffering proportional to its full
/// size — the 30s request timeout is not a byte budget.
const MAX_FETCH_BYTES: usize = 2_000_000;

mod descriptors {
    use aisdk::core::tools::Tool;
    use aisdk::macros::tool;

    #[tool(name = "web_fetch")]
    /// Fetch a URL and return its content as plain text (HTML stripped).
    pub fn web_fetch_descriptor(url: String) -> Tool {
        Ok(url)
    }

    #[tool(name = "web_search")]
    /// Search the web and return results (requires a configured search provider).
    pub fn web_search_descriptor(query: String) -> Tool {
        Ok(query)
    }
}

/// Is `ip` safe to connect to (i.e. not internal)?
fn is_public_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: &Ipv4Addr) -> bool {
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local() // 169.254/16 — covers cloud metadata 169.254.169.254
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.octets()[0] == 0)
}

fn is_public_v6(ip: &Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() {
        return false;
    }
    let seg0 = ip.segments()[0];
    let link_local = (seg0 & 0xffc0) == 0xfe80; // fe80::/10
    let unique_local = (seg0 & 0xfe00) == 0xfc00; // fc00::/7
    !(link_local || unique_local)
}

/// Validate a URL for egress: http(s) scheme + all resolved IPs public.
/// Returns the validated URL string on success.
pub fn validate_public_url(raw: &str) -> Result<String, ToolError> {
    let parsed = rusty_url::Url::parse(raw)
        .map_err(|e| ToolError::InvalidArgs(format!("invalid url: {e}")))?;
    match parsed.scheme() {
        "http" | "https" => {}
        s => return Err(ToolError::InvalidArgs(format!("unsupported scheme '{s}'"))),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| ToolError::InvalidArgs("url has no host".into()))?;
    let port = parsed.port_or_known_default().unwrap_or(443);

    // An IP literal is checked directly; a hostname is resolved and ALL of its
    // addresses must be public (a single internal answer blocks the request).
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !is_public_ip(&ip) {
            return Err(ToolError::Other(format!(
                "blocked non-public address: {ip}"
            )));
        }
    } else {
        let addrs: Vec<_> = (host, port)
            .to_socket_addrs()
            .map_err(|e| ToolError::Other(format!("dns resolution failed: {e}")))?
            .collect();
        if addrs.is_empty() {
            return Err(ToolError::Other(format!("host '{host}' did not resolve")));
        }
        for addr in addrs {
            if !is_public_ip(&addr.ip()) {
                return Err(ToolError::Other(format!(
                    "blocked: '{host}' resolves to non-public {}",
                    addr.ip()
                )));
            }
        }
    }
    Ok(parsed.to_string())
}

/// Find the largest UTF-8 char-boundary index `<= idx` in `s`. A stable-Rust
/// stand-in for the unstable `str::floor_char_boundary` — needed because
/// `String::truncate` panics on a non-boundary index, and a fixed byte cap
/// (e.g. `FETCH_CAP`) can land inside a multibyte character.
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Strip HTML tags + `<script>`/`<style>` blocks; collapse whitespace. Crude v1.
/// Operates on bytes (UTF-8 safe — no char-boundary slicing) then lossily
/// reassembles, so it cannot panic on multibyte content.
fn strip_html(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let low = lower.as_bytes();
    let src = html.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(src.len());
    let mut in_tag = false;
    let mut skip_until: Option<&[u8]> = None;
    let mut i = 0;
    while i < src.len() {
        if let Some(end) = skip_until {
            if low[i..].starts_with(end) {
                i += end.len();
                skip_until = None;
            } else {
                i += 1;
            }
            continue;
        }
        if low[i..].starts_with(b"<script") {
            skip_until = Some(b"</script>");
            i += 1;
            continue;
        }
        if low[i..].starts_with(b"<style") {
            skip_until = Some(b"</style>");
            i += 1;
            continue;
        }
        match src[i] {
            b'<' => in_tag = true,
            b'>' => {
                in_tag = false;
                out.push(b' ');
            }
            b if !in_tag => out.push(b),
            _ => {}
        }
        i += 1;
    }
    String::from_utf8_lossy(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Fetch `url` and strip HTML from the response, enforcing both a
/// receive-time byte budget (`MAX_FETCH_BYTES`) and a post-strip output cap
/// (`FETCH_CAP`). Split out from `web_fetch_impl` so tests can exercise it
/// directly against a local server without going through the public-IP SSRF
/// guard.
async fn fetch_and_strip(url: &str) -> Result<String, ToolError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| ToolError::Other(e.to_string()))?;
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| ToolError::Other(e.to_string()))?;

    // Read the body incrementally instead of `.text()`, which would buffer
    // the entire response before any cap applies. The budget is enforced
    // against bytes actually received, so a chunked response with no
    // `Content-Length` is bounded the same way as one that declares a
    // (possibly false) size.
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| ToolError::Other(e.to_string()))?
    {
        buf.extend_from_slice(&chunk);
        if buf.len() > MAX_FETCH_BYTES {
            return Err(ToolError::Other(format!(
                "response exceeded the {MAX_FETCH_BYTES}-byte fetch budget"
            )));
        }
    }
    let body = String::from_utf8_lossy(&buf).into_owned();

    let mut text = strip_html(&body);
    if text.len() > FETCH_CAP {
        let cut = floor_char_boundary(&text, FETCH_CAP);
        text.truncate(cut);
        text.push_str("… (truncated)");
    }
    Ok(text)
}

async fn web_fetch_impl(args: Value) -> Result<String, ToolError> {
    let raw = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArgs("missing string field 'url'".into()))?;
    let url = validate_public_url(raw)?; // SSRF guard — runs before any network I/O
    fetch_and_strip(&url).await
}

async fn web_search_impl(_args: Value) -> Result<String, ToolError> {
    // v1 stub: a provider integration (brave/serper/duckduckgo via
    // RUSTYKEYS_SEARCH_PROVIDER) is a follow-up. Honest error rather than a
    // silent empty result.
    Err(ToolError::Other(
        "web_search is not configured (set RUSTYKEYS_SEARCH_PROVIDER; provider integration is a follow-up)"
            .into(),
    ))
}

/// Register the web tools. Call only when `RUSTYKEYS_ALLOW_WEB` is set — the
/// tools are absent (not just disabled) otherwise (PRD 03: blocked by default).
pub fn register_web_tools(registry: &mut ToolRegistry) {
    registry.insert(Box::new(AiSdkTool::new(
        descriptors::web_fetch_descriptor(),
        web_fetch_impl,
    )));
    registry.insert(Box::new(AiSdkTool::new(
        descriptors::web_search_descriptor(),
        web_search_impl,
    )));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_loopback_and_cloud_metadata() {
        assert!(validate_public_url("http://127.0.0.1/").is_err());
        assert!(validate_public_url("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(validate_public_url("http://10.0.0.5/").is_err());
        assert!(validate_public_url("http://192.168.1.1/").is_err());
        assert!(validate_public_url("http://[::1]/").is_err());
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(validate_public_url("file:///etc/passwd").is_err());
        assert!(validate_public_url("ftp://example.com/").is_err());
    }

    #[test]
    fn allows_a_public_ip_literal() {
        assert!(validate_public_url("http://1.1.1.1/").is_ok());
    }

    #[tokio::test]
    async fn web_fetch_denies_metadata_ip_before_any_request() {
        let out = web_fetch_impl(serde_json::json!({"url": "http://169.254.169.254/"})).await;
        assert!(out.is_err());
    }

    #[test]
    fn strip_html_removes_tags_and_scripts() {
        let html = "<html><script>evil()</script><p>Hello <b>world</b></p></html>";
        assert_eq!(strip_html(html), "Hello world");
    }

    /// Spawn a one-shot raw HTTP/1.1 server on loopback that drains the
    /// request and replies with exactly `response` bytes, then returns its
    /// `http://127.0.0.1:<port>/` URL. Used to exercise `fetch_and_strip`
    /// directly (bypassing the public-IP SSRF guard, which is irrelevant to
    /// these tests and is covered separately above).
    fn spawn_raw_http_server(response: Vec<u8>) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local_addr");
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut received = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            received.extend_from_slice(&chunk[..n]);
                            if received.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let _ = stream.write_all(&response);
                let _ = stream.flush();
            }
        });
        format!("http://{addr}/")
    }

    fn http_response_with_length(body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    /// Same body, but chunked transfer encoding and no `Content-Length` —
    /// the case a receive-time byte budget must still catch.
    fn http_response_chunked(body: &[u8], chunk_size: usize) -> Vec<u8> {
        let mut out =
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                .to_vec();
        for piece in body.chunks(chunk_size) {
            out.extend_from_slice(format!("{:x}\r\n", piece.len()).as_bytes());
            out.extend_from_slice(piece);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"0\r\n\r\n");
        out
    }

    #[test]
    fn floor_char_boundary_walks_back_to_a_valid_boundary() {
        let s = "a€b"; // 'a' (1B), '€' (3B) at index 1..4, 'b' at 4
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, 1), 1);
        assert_eq!(floor_char_boundary(s, 2), 1); // mid-'€'
        assert_eq!(floor_char_boundary(s, 3), 1); // mid-'€'
        assert_eq!(floor_char_boundary(s, 4), 4);
        assert_eq!(floor_char_boundary(s, 100), s.len());
    }

    #[tokio::test]
    async fn truncates_cleanly_on_2_byte_utf8_boundary_without_panicking() {
        // 49_999 ascii bytes then 'é' (2B, U+00E9) spanning bytes 49_999..50_001,
        // so byte index FETCH_CAP (50_000) lands mid-character.
        let body = format!("{}é{}", "a".repeat(49_999), "a".repeat(20));
        let url = spawn_raw_http_server(http_response_with_length(body.as_bytes()));
        let text = fetch_and_strip(&url).await.expect("fetch should not error");
        assert!(text.len() <= FETCH_CAP + "… (truncated)".len());
        assert!(text.ends_with("… (truncated)"));
        assert_eq!(text, format!("{}… (truncated)", "a".repeat(49_999)));
    }

    #[tokio::test]
    async fn truncates_cleanly_on_3_byte_utf8_boundary_without_panicking() {
        // 49_999 ascii bytes then '€' (3B, U+20AC) spanning bytes 49_999..50_002,
        // so byte index FETCH_CAP (50_000) lands mid-character.
        let body = format!("{}€{}", "a".repeat(49_999), "a".repeat(20));
        let url = spawn_raw_http_server(http_response_with_length(body.as_bytes()));
        let text = fetch_and_strip(&url).await.expect("fetch should not error");
        assert!(text.len() <= FETCH_CAP + "… (truncated)".len());
        assert!(text.ends_with("… (truncated)"));
        assert_eq!(text, format!("{}… (truncated)", "a".repeat(49_999)));
    }

    #[tokio::test]
    async fn truncates_cleanly_on_4_byte_utf8_boundary_without_panicking() {
        // 49_998 ascii bytes then '🎉' (4B, U+1F389) spanning bytes 49_998..50_002,
        // so byte index FETCH_CAP (50_000) lands mid-character.
        let body = format!("{}🎉{}", "a".repeat(49_998), "a".repeat(20));
        let url = spawn_raw_http_server(http_response_with_length(body.as_bytes()));
        let text = fetch_and_strip(&url).await.expect("fetch should not error");
        assert!(text.len() <= FETCH_CAP + "… (truncated)".len());
        assert!(text.ends_with("… (truncated)"));
        assert_eq!(text, format!("{}… (truncated)", "a".repeat(49_998)));
    }

    #[tokio::test]
    async fn rejects_oversized_response_with_content_length_without_full_buffering() {
        let body = vec![b'a'; MAX_FETCH_BYTES + 500_000];
        let url = spawn_raw_http_server(http_response_with_length(&body));
        let result = fetch_and_strip(&url).await;
        assert!(
            result.is_err(),
            "response over the receive budget must error"
        );
    }

    #[tokio::test]
    async fn rejects_oversized_chunked_response_with_no_content_length_header() {
        let body = vec![b'a'; MAX_FETCH_BYTES + 500_000];
        let url = spawn_raw_http_server(http_response_chunked(&body, 8192));
        let result = fetch_and_strip(&url).await;
        assert!(
            result.is_err(),
            "chunked response over the receive budget must error even without Content-Length"
        );
    }
}
