//! Stdio transport: newline-delimited JSON-RPC over stdin/stdout.
//!
//! This is what an ADK agent's `StdioConnectionParams` launches — the server
//! is a subprocess, and the agent speaks JSON-RPC to it over pipes.
//!
//! Anything the server wants to log must go to **stderr**: stdout carries the
//! protocol, and a stray `println!` corrupts the stream.
//!
//! `rusty_mcp_server`'s stdio loop is blocking, so the async reader and writer
//! are joined to it by two small pump tasks and a pair of channels.

use std::io::{self, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use adk_core::{AdkError, Result};
use rusty_mcp_server::{serve_lines, StdioConfig};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::line_cap::{cap_error, LineCapped, MAX_LINE_BYTES};
use crate::server::McpServer;

/// Bytes read from the async side at a time, and chunks buffered each way.
const CHUNK: usize = 16 * 1024;
const BUFFERED_CHUNKS: usize = 8;

/// Serves `server` over stdin and stdout until stdin closes.
pub async fn serve_stdio(server: &McpServer) -> Result<()> {
    serve_stream(server, tokio::io::stdin(), tokio::io::stdout()).await
}

/// Serves `server` over an arbitrary reader and writer, until the reader
/// ends.
///
/// Exposed separately so the transport can be exercised over in-memory pipes.
///
/// # Errors
/// Fails if a line exceeds the 16 MiB cap (the session ends rather than
/// buffering it), or if the session fails for any reason other than the
/// peer going away.
pub async fn serve_stream<R, W>(server: &McpServer, reader: R, writer: W) -> Result<()>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    server.bind_runtime();
    let wire = Arc::new(
        server
            .wire_server()
            .map_err(|e| AdkError::Other(format!("MCP server description is invalid: {e}")))?,
    );
    let exceeded = Arc::new(AtomicBool::new(false));

    let (in_tx, in_rx) = mpsc::channel::<Vec<u8>>(BUFFERED_CHUNKS);
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(BUFFERED_CHUNKS);

    // Reader pump: ends at end of input, on a read error, or at the line cap;
    // dropping the reader then tells the peer to stop writing.
    let capped = LineCapped::new(reader, Arc::clone(&exceeded));
    let pump_in = tokio::spawn(async move {
        let mut capped = capped;
        let mut buf = vec![0u8; CHUNK];
        loop {
            match capped.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if in_tx.send(buf[..n].to_vec()).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Writer pump: every chunk is written and flushed as it arrives.
    let pump_out = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(chunk) = out_rx.recv().await {
            if writer.write_all(&chunk).await.is_err() || writer.flush().await.is_err() {
                break;
            }
        }
    });

    let config = StdioConfig {
        max_line_bytes: usize::try_from(MAX_LINE_BYTES).unwrap_or(usize::MAX),
        ..StdioConfig::default()
    };
    let served = tokio::task::spawn_blocking(move || {
        serve_lines(
            wire,
            BufReader::new(ChannelReader::new(in_rx)),
            ChannelWriter(out_tx),
            config,
        )
    })
    .await;

    pump_in.abort();
    // The writer pump ends once the loop (which owned the sender) is gone.
    let _ = pump_out.await;

    if exceeded.load(Ordering::Acquire) {
        return Err(AdkError::Other(cap_error()));
    }
    match served {
        Ok(Ok(())) => Ok(()),
        // The peer left while we were writing: nothing more to serve.
        Ok(Err(e)) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Ok(Err(e)) => Err(AdkError::Other(format!("MCP session failed: {e}"))),
        Err(e) => Err(AdkError::Other(format!("MCP server task failed: {e}"))),
    }
}

/// A blocking [`Read`] over chunks arriving on a channel; end of channel is
/// end of input. Must be read from outside the async runtime's workers.
struct ChannelReader {
    chunks: mpsc::Receiver<Vec<u8>>,
    current: Vec<u8>,
    taken: usize,
}

impl ChannelReader {
    fn new(chunks: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            chunks,
            current: Vec::new(),
            taken: 0,
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.taken == self.current.len() {
            match self.chunks.blocking_recv() {
                Some(chunk) => {
                    self.current = chunk;
                    self.taken = 0;
                }
                None => return Ok(0),
            }
        }
        let n = out.len().min(self.current.len() - self.taken);
        out[..n].copy_from_slice(&self.current[self.taken..self.taken + n]);
        self.taken += n;
        Ok(n)
    }
}

/// A blocking [`Write`] that sends each write to the async writer pump.
struct ChannelWriter(mpsc::Sender<Vec<u8>>);

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0
            .blocking_send(data.to_vec())
            .map_err(|_| io::ErrorKind::BrokenPipe)?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(()) // the pump flushes after every chunk
    }
}
