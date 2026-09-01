//! Stdio transport: newline-delimited JSON-RPC over stdin/stdout.
//!
//! This is what an ADK agent's `StdioConnectionParams` launches — the server
//! is a subprocess, and the agent speaks JSON-RPC to it over pipes.
//!
//! Anything the server wants to log must go to **stderr**: stdout carries the
//! protocol, and a stray `println!` corrupts the stream.

use adk_core::Result;
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::server::McpServer;

/// Serves `server` over stdin and stdout until stdin closes.
pub async fn serve_stdio(server: &McpServer) -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    serve_stream(server, stdin, stdout).await
}

/// Serves `server` over an arbitrary reader and writer.
///
/// Exposed separately so the transport can be exercised over in-memory pipes.
pub async fn serve_stream<R, W>(server: &McpServer, reader: R, mut writer: W) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(response) = server.handle_raw(line).await {
            writer.write_all(response.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            // Flush per message: the client is blocked waiting for this
            // response, so buffering it would deadlock the exchange.
            writer.flush().await?;
        }
    }

    Ok(())
}
