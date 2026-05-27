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
    let parsed =
        url::Url::parse(raw).map_err(|e| ToolError::InvalidArgs(format!("invalid url: {e}")))?;
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

async fn web_fetch_impl(args: Value) -> Result<String, ToolError> {
    let raw = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArgs("missing string field 'url'".into()))?;
    let url = validate_public_url(raw)?; // SSRF guard — runs before any network I/O
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| ToolError::Other(e.to_string()))?;
    let body = client
        .get(&url)
        .send()
        .await
        .map_err(|e| ToolError::Other(e.to_string()))?
        .text()
        .await
        .map_err(|e| ToolError::Other(e.to_string()))?;
    let mut text = strip_html(&body);
    if text.len() > FETCH_CAP {
        text.truncate(FETCH_CAP);
        text.push_str("… (truncated)");
    }
    Ok(text)
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
}
