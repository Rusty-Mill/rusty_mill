#![allow(clippy::unwrap_used)]
//! Version negotiation over the stdio stream: the server speaks the revisions
//! up to `PROTOCOL_VERSION`, and answers a newer one with it.

use adk_mcp::{serve_stream, serve_tools, PROTOCOL_VERSION};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn negotiated(asked: &str) -> String {
    let (mut to_server, server_in) = tokio::io::duplex(1 << 16);
    let (server_out, from_server) = tokio::io::duplex(1 << 16);
    let server = serve_tools("versions", vec![]);
    tokio::spawn(async move { serve_stream(&server, server_in, server_out).await });
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": asked, "capabilities": {},
        "clientInfo": {"name": "t", "version": "0"}}});
    to_server
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let mut line = String::new();
    BufReader::new(from_server)
        .read_line(&mut line)
        .await
        .unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    reply["result"]["protocolVersion"]
        .as_str()
        .unwrap_or_else(|| panic!("no version in {reply}"))
        .to_owned()
}

#[tokio::test]
async fn an_older_supported_revision_is_answered_as_asked() {
    assert_eq!(negotiated("2025-03-26").await, "2025-03-26");
    assert_eq!(negotiated("2024-11-05").await, "2024-11-05");
}

#[tokio::test]
async fn a_newer_revision_is_answered_with_the_newest_supported() {
    assert_eq!(negotiated("2099-01-01").await, PROTOCOL_VERSION);
    assert_eq!(negotiated("2026-07-28").await, PROTOCOL_VERSION);
}
