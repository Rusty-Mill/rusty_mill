//! The socket integration `protocol.rs` explicitly deferred: a real
//! `rusty_tokio` TCP listener, a connection loop that reads a framed
//! [`crate::protocol::Request`] off the wire, dispatches it against a real
//! [`crate::retention::Log`], and writes the encoded
//! [`crate::protocol::Response`] back.
//!
//! One [`Log`] is shared across every connection via `rusty_tokio::sync::
//! Mutex` — `Log::append` needs `&mut self`, and nothing about the
//! thread-per-core runtime guarantees every connection lands on the same
//! core, so a real cross-core-safe lock is the correct default here, not an
//! optimization to defer. A connection holds the lock only for the
//! duration of one dispatch, never across a network read/write.
//!
//! ## What this pass does not do
//!
//! - No consumer-offset commit request in the wire protocol yet — only
//!   `Produce`/`Fetch` exist in `protocol.rs`. `ConsumerOffsets` has no
//!   wire exposure at all yet.
//! - No graceful shutdown — [`serve`] runs until its listener errors or the
//!   task is aborted; there's no signal to drain in-flight connections
//!   first.
//! - A malformed frame (garbage length prefix, a body that never fully
//!   arrives) currently just ends that connection when the read fails or
//!   times out at the OS level — no explicit frame-size sanity cap yet, so
//!   a client claiming a multi-gigabyte body would make this allocate that
//!   much before finding out the rest never arrives. Fine for a scaffold
//!   talking to a trusted client; a real deployment needs a cap before
//!   this is internet-facing.

use std::sync::Arc;

use rusty_tokio::io::{TcpListener, TcpStream};
use rusty_tokio::sync::Mutex;

use crate::protocol::{self, Request, Response};
use crate::retention::Log;

/// Accepts connections on `listener` forever, spawning one task per
/// connection, until `listener` itself errors (e.g. the underlying socket
/// closed) or this task is aborted.
pub async fn serve(listener: TcpListener, log: Arc<Mutex<Log>>) -> std::io::Result<()> {
    loop {
        let (stream, _addr) = listener.accept().await?;
        let log = log.clone();
        rusty_tokio::spawn(async move {
            let _ = handle_connection(stream, log).await;
        });
    }
}

/// One connection's request/response loop: read a framed request, dispatch
/// it, write the framed response, repeat until the peer disconnects or a
/// real I/O error occurs.
async fn handle_connection(stream: TcpStream, log: Arc<Mutex<Log>>) -> std::io::Result<()> {
    loop {
        let mut header = [0u8; 4];
        match stream.read_exact(&mut header).await {
            Ok(()) => {}
            // A peer that disconnects between requests (the common,
            // expected case) and one that disconnects mid-header both
            // surface as UnexpectedEof -- see this module's top-level docs
            // for why that's not distinguished here.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        }

        let len = protocol::frame_len(header) as usize;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await?;

        let response = match protocol::decode_request(&body) {
            Ok(req) => dispatch(req, &log).await,
            Err(e) => Response::Error {
                message: format!("{e:?}"),
            },
        };

        let encoded = protocol::encode_response(&response);
        let framed = protocol::frame(&encoded);
        stream.write_all(&framed).await?;
    }
}

async fn dispatch(req: Request, log: &Arc<Mutex<Log>>) -> Response {
    match req {
        Request::Produce { payload } => {
            let mut log = log.lock().await;
            match log.append(&payload).await {
                Ok(offset) => Response::Produced { offset },
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Request::Fetch { offset } => {
            let log = log.lock().await;
            match log.read(offset).await {
                Ok(payload) => Response::Fetched { payload },
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SimClock;
    use crate::offset::Offset;
    use crate::retention::RetentionPolicy;
    use rusty_tokio::io::SimDriver;

    fn unbounded_policy() -> RetentionPolicy {
        RetentionPolicy {
            max_segment_bytes: 1_000_000,
            max_total_bytes: None,
            max_segment_age_millis: None,
        }
    }

    async fn send_request(stream: &TcpStream, req: &Request) -> Response {
        let encoded = protocol::encode_request(req);
        let framed = protocol::frame(&encoded);
        stream.write_all(&framed).await.unwrap();

        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await.unwrap();
        let len = protocol::frame_len(header) as usize;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await.unwrap();
        protocol::decode_response(&body).unwrap()
    }

    /// A real client, over a real loopback TCP socket, against a real
    /// `Log` backed by `SimDriver` -- the actual integration this module
    /// exists for, not a stand-in for it. `rusty_tokio` has no network
    /// fault injection (only `SimDriver` does, for disk), so this test
    /// exercises real sockets rather than a simulated network.
    #[rusty_tokio::test]
    async fn produce_then_fetch_round_trips_over_a_real_socket() {
        let driver = SimDriver::new();
        let clock = Arc::new(SimClock::new());
        let log = Log::create(driver, clock, "/log", unbounded_policy())
            .await
            .unwrap();
        let log = Arc::new(Mutex::new(log));

        let listener = TcpListener::bind_addrs("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = rusty_tokio::spawn(serve(listener, log));

        let client = TcpStream::connect(addr).await.unwrap();

        let produced = send_request(
            &client,
            &Request::Produce {
                payload: b"hello over the wire".to_vec(),
            },
        )
        .await;
        assert_eq!(produced, Response::Produced { offset: Offset(0) });

        let fetched = send_request(&client, &Request::Fetch { offset: Offset(0) }).await;
        assert_eq!(
            fetched,
            Response::Fetched {
                payload: b"hello over the wire".to_vec()
            }
        );

        server.abort();
    }

    /// A `Fetch` for an offset that doesn't exist comes back as a real
    /// `Response::Error`, not a dropped connection or a panic.
    #[rusty_tokio::test]
    async fn fetching_an_unknown_offset_returns_an_error_response() {
        let driver = SimDriver::new();
        let clock = Arc::new(SimClock::new());
        let log = Log::create(driver, clock, "/log", unbounded_policy())
            .await
            .unwrap();
        let log = Arc::new(Mutex::new(log));

        let listener = TcpListener::bind_addrs("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = rusty_tokio::spawn(serve(listener, log));

        let client = TcpStream::connect(addr).await.unwrap();
        let response = send_request(&client, &Request::Fetch { offset: Offset(0) }).await;
        assert!(matches!(response, Response::Error { .. }));

        server.abort();
    }

    /// Two clients against the same `Log` interleave safely -- the shared
    /// `Mutex` is what makes that true regardless of which core either
    /// connection's task actually runs on.
    #[rusty_tokio::test]
    async fn two_concurrent_clients_share_one_log_safely() {
        let driver = SimDriver::new();
        let clock = Arc::new(SimClock::new());
        let log = Log::create(driver, clock, "/log", unbounded_policy())
            .await
            .unwrap();
        let log = Arc::new(Mutex::new(log));

        let listener = TcpListener::bind_addrs("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = rusty_tokio::spawn(serve(listener, log));

        let client_a = TcpStream::connect(addr).await.unwrap();
        let client_b = TcpStream::connect(addr).await.unwrap();

        let a = send_request(
            &client_a,
            &Request::Produce {
                payload: b"from a".to_vec(),
            },
        )
        .await;
        let b = send_request(
            &client_b,
            &Request::Produce {
                payload: b"from b".to_vec(),
            },
        )
        .await;

        // Both succeed, and land at two distinct, real offsets -- which one
        // got offset 0 depends on accept/scheduling order, so assert the
        // property (distinct, valid offsets) rather than a specific order.
        let (Response::Produced { offset: off_a }, Response::Produced { offset: off_b }) = (a, b)
        else {
            panic!("expected both produces to succeed");
        };
        assert_ne!(off_a, off_b);

        assert_eq!(
            send_request(&client_a, &Request::Fetch { offset: off_a }).await,
            Response::Fetched {
                payload: b"from a".to_vec()
            }
        );
        assert_eq!(
            send_request(&client_b, &Request::Fetch { offset: off_b }).await,
            Response::Fetched {
                payload: b"from b".to_vec()
            }
        );

        server.abort();
    }
}
