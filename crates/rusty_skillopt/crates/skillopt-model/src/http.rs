//! HTTP response-body hardening shared by every network-backed
//! [`skillopt_core::ChatBackend`] (`anthropic`, `azure_openai`,
//! `openai_compat`): a misbehaving or hostile endpoint that returns (or
//! declares) an arbitrarily large body must not be able to grow this
//! process without bound. [`read_capped_body`] replaces the unbounded
//! `resp.text()` every backend used to call directly.
//!
//! Per-call HTTP *timeouts* aren't handled here -- each backend carries its
//! own `timeout: Duration` field (default `DEFAULT_TIMEOUT`, overridable via
//! `with_timeout`), mirroring the convention `aisf_stage`/`claude_cli`
//! already use for their own (subprocess-based) request paths.

/// Caps how much of a chat response body this crate will buffer in memory.
/// Mirrors the `MAX_BODY_BYTES` convention already used elsewhere in the
/// workspace for bounding untrusted response/message bodies (e.g.
/// `rusty_llama`'s server, `nexus-lsp`/`nexus-dap`'s transport framing).
pub(crate) const MAX_RESPONSE_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Reads `resp`'s body into a `String`, refusing to buffer past
/// [`MAX_RESPONSE_BODY_BYTES`]. Replaces `resp.text().await`, which
/// collects the whole body no matter how large the endpoint makes it.
///
/// Checked twice: once against a declared `Content-Length` before reading
/// anything (the fast path for a well-behaved but oversized response), and
/// once against a running total while streaming chunks (for a response
/// with no -- or a lying -- `Content-Length`).
pub(crate) async fn read_capped_body(
    resp: reqwest::Response,
) -> anyhow::Result<(reqwest::StatusCode, String)> {
    let status = resp.status();
    if let Some(len) = resp.content_length() {
        anyhow::ensure!(
            len <= MAX_RESPONSE_BODY_BYTES as u64,
            "response declared Content-Length {len}, exceeding the {MAX_RESPONSE_BODY_BYTES}-byte cap"
        );
    }

    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        anyhow::ensure!(
            buf.len() + chunk.len() <= MAX_RESPONSE_BODY_BYTES,
            "response body exceeded the {MAX_RESPONSE_BODY_BYTES}-byte cap"
        );
        buf.extend_from_slice(&chunk);
    }

    Ok((status, String::from_utf8_lossy(&buf).into_owned()))
}
