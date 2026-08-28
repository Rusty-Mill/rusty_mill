//! The socket integration `protocol.rs` explicitly deferred: a real
//! `rusty_tokio` TCP listener, a connection loop that reads a framed
//! [`crate::protocol::Request`] off the wire, dispatches it against a real
//! [`crate::retention::Log`]/[`crate::consumer::ConsumerOffsets`], and
//! writes the encoded [`crate::protocol::Response`] back.
//!
//! [`AppState`]'s `Log` and `ConsumerOffsets` are each shared across every
//! connection via their own `rusty_tokio::sync::Mutex` — `Log::append`/
//! `ConsumerOffsets::commit` need `&mut self`, and nothing about the
//! thread-per-core runtime guarantees every connection lands on the same
//! core, so a real cross-core-safe lock is the correct default here, not an
//! optimization to defer. Two separate locks, not one covering both: a
//! `Commit`/`LastCommitted` request never needs to wait on `Log`'s lock, and
//! vice versa. A connection holds whichever lock it needs only for the
//! duration of one dispatch, never across a network read/write.
//!
//! A frame claiming a body longer than `MAX_FRAME_LEN` gets its connection
//! ended immediately, before the oversized buffer is ever allocated or a
//! byte of the (possibly nonexistent) body is read — a client can claim a
//! multi-gigabyte body, but this never pays for that claim.
//!
//! [`serve`] shuts down gracefully on request: its caller holds the
//! `rusty_tokio::sync::watch::Sender<bool>` half of the channel whose
//! `Receiver` half is passed in, and sending `true` stops the accept loop
//! from taking any *new* connection while every connection already
//! in-flight keeps running until it finishes on its own (the peer
//! disconnects, or a real I/O error) — `serve` itself returns only once
//! the last one has. Nothing is aborted just because shutdown was
//! requested.

use std::sync::Arc;

use rusty_tokio::io::{TcpListener, TcpStream};
use rusty_tokio::sync::{watch, Mutex};
use rusty_tokio::task::JoinSet;

use crate::consumer::ConsumerOffsets;
use crate::protocol::{self, Request, Response};
use crate::retention::Log;

/// The largest frame body [`handle_connection`] will allocate a buffer
/// for. A client's declared length past this is rejected before any
/// allocation or read of the body happens -- see this module's top-level
/// docs.
const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024; // 16 MiB

/// Everything a connection needs to dispatch a request — a `Log` and a
/// `ConsumerOffsets`, each independently lockable. Cheap to clone (two
/// `Arc`s) — every connection task gets its own handle to the same shared
/// state, not a copy of it.
#[derive(Clone)]
pub struct AppState {
    pub log: Arc<Mutex<Log>>,
    pub consumer_offsets: Arc<Mutex<ConsumerOffsets>>,
}

/// Accepts connections on `listener`, spawning one task per connection,
/// until `listener` itself errors, `shutdown` is told to close (`true`),
/// or this task is aborted. On a graceful shutdown, every connection
/// already accepted is drained (awaited to its own natural completion,
/// not aborted) before this returns — see this module's top-level docs.
pub async fn serve(
    listener: TcpListener,
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
) -> std::io::Result<()> {
    let mut connections: JoinSet<std::io::Result<()>> = JoinSet::new();

    loop {
        // `select!`'s arms can't `return`/`break` directly (they're
        // spliced into a `poll_fn` closure, a separate control-flow
        // boundary) -- each arm evaluates to a plain value instead, and
        // the actual `return`/`break` happens below, back in `serve`'s
        // own scope.
        let accepted = rusty_tokio::select! {
            result = listener.accept() => Some(result),
            _ = shutdown.changed() => None,
        };
        let Some(result) = accepted else {
            break; // shutdown requested -- stop accepting new connections
        };
        match result {
            Ok((stream, _addr)) => {
                let state = state.clone();
                connections.spawn(async move { handle_connection(stream, state).await });
            }
            Err(e) => return Err(e),
        }
    }

    while connections.join_next().await.is_some() {}
    Ok(())
}

/// One connection's request/response loop: read a framed request, dispatch
/// it, write the framed response, repeat until the peer disconnects or a
/// real I/O error occurs.
async fn handle_connection(stream: TcpStream, state: AppState) -> std::io::Result<()> {
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

        let len = protocol::frame_len(header);
        if len > MAX_FRAME_LEN {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("frame length {len} exceeds the {MAX_FRAME_LEN}-byte cap"),
            ));
        }

        let mut body = vec![0u8; len as usize];
        stream.read_exact(&mut body).await?;

        let response = match protocol::decode_request(&body) {
            Ok(req) => dispatch(req, &state).await,
            Err(e) => Response::Error {
                message: format!("{e:?}"),
            },
        };

        let encoded = protocol::encode_response(&response);
        let framed = protocol::frame(&encoded);
        stream.write_all(&framed).await?;
    }
}

async fn dispatch(req: Request, state: &AppState) -> Response {
    match req {
        Request::Produce { payload } => {
            let mut log = state.log.lock().await;
            match log.append(&payload).await {
                Ok(offset) => Response::Produced { offset },
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Request::Fetch { offset } => {
            let log = state.log.lock().await;
            match log.read(offset).await {
                Ok(payload) => Response::Fetched { payload },
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Request::Commit {
            consumer_id,
            offset,
        } => {
            let mut offsets = state.consumer_offsets.lock().await;
            match offsets.commit(&consumer_id, offset).await {
                Ok(()) => Response::Committed,
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Request::LastCommitted { consumer_id } => {
            let offsets = state.consumer_offsets.lock().await;
            Response::LastCommitted {
                offset: offsets.last_committed(&consumer_id),
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

    async fn test_state() -> AppState {
        let driver = SimDriver::new();
        let clock = Arc::new(SimClock::new());
        let log = Log::create(driver.clone(), clock, "/log", unbounded_policy())
            .await
            .unwrap();
        let consumer_offsets = ConsumerOffsets::create_on(driver, "/offsets")
            .await
            .unwrap();
        AppState {
            log: Arc::new(Mutex::new(log)),
            consumer_offsets: Arc::new(Mutex::new(consumer_offsets)),
        }
    }

    async fn spawn_server() -> (
        rusty_tokio::JoinHandle<std::io::Result<()>>,
        std::net::SocketAddr,
    ) {
        let state = test_state().await;
        let listener = TcpListener::bind_addrs("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        // Every test using this helper tears down via `server.abort()`,
        // not graceful shutdown -- leaking the sender keeps
        // `shutdown_rx.changed()` pending for this server's lifetime
        // instead of every caller having to hold and thread through a
        // sender it never uses. See
        // `shutdown_stops_accepting_and_drains_in_flight_connections`
        // for a test that actually exercises the sender.
        std::mem::forget(shutdown_tx);
        let server = rusty_tokio::spawn(serve(listener, state, shutdown_rx));
        (server, addr)
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
        let (server, addr) = spawn_server().await;
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
        let (server, addr) = spawn_server().await;
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
        let (server, addr) = spawn_server().await;
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

    /// A commit for a consumer, read back via `LastCommitted` on the same
    /// (or a different) connection -- the write/read pair this module
    /// exists to expose, end to end over a real socket.
    #[rusty_tokio::test]
    async fn commit_then_last_committed_round_trips_over_a_real_socket() {
        let (server, addr) = spawn_server().await;
        let client = TcpStream::connect(addr).await.unwrap();

        let committed = send_request(
            &client,
            &Request::Commit {
                consumer_id: "reader-1".to_string(),
                offset: Offset(7),
            },
        )
        .await;
        assert_eq!(committed, Response::Committed);

        let last = send_request(
            &client,
            &Request::LastCommitted {
                consumer_id: "reader-1".to_string(),
            },
        )
        .await;
        assert_eq!(
            last,
            Response::LastCommitted {
                offset: Some(Offset(7))
            }
        );

        server.abort();
    }

    /// `LastCommitted` for a consumer that has never committed comes back
    /// as `offset: None`, not an error -- a fresh consumer is expected,
    /// not exceptional.
    #[rusty_tokio::test]
    async fn last_committed_for_an_unknown_consumer_returns_none() {
        let (server, addr) = spawn_server().await;
        let client = TcpStream::connect(addr).await.unwrap();

        let last = send_request(
            &client,
            &Request::LastCommitted {
                consumer_id: "never-seen".to_string(),
            },
        )
        .await;
        assert_eq!(last, Response::LastCommitted { offset: None });

        server.abort();
    }

    /// A later commit for the same consumer overwrites the earlier one --
    /// `LastCommitted` always reflects the most recent commit, matching
    /// `ConsumerOffsets`'s own last-write-wins semantics.
    #[rusty_tokio::test]
    async fn a_later_commit_overwrites_an_earlier_one_for_the_same_consumer() {
        let (server, addr) = spawn_server().await;
        let client = TcpStream::connect(addr).await.unwrap();

        for offset in [Offset(1), Offset(2), Offset(3)] {
            let committed = send_request(
                &client,
                &Request::Commit {
                    consumer_id: "reader-1".to_string(),
                    offset,
                },
            )
            .await;
            assert_eq!(committed, Response::Committed);
        }

        let last = send_request(
            &client,
            &Request::LastCommitted {
                consumer_id: "reader-1".to_string(),
            },
        )
        .await;
        assert_eq!(
            last,
            Response::LastCommitted {
                offset: Some(Offset(3))
            }
        );

        server.abort();
    }

    /// Two consumers committing against the same shared `ConsumerOffsets`
    /// stay independent -- the same cross-connection safety property
    /// `two_concurrent_clients_share_one_log_safely` verifies for `Log`,
    /// but for the other lock this module holds.
    #[rusty_tokio::test]
    async fn two_consumers_committing_concurrently_stay_independent() {
        let (server, addr) = spawn_server().await;
        let client_a = TcpStream::connect(addr).await.unwrap();
        let client_b = TcpStream::connect(addr).await.unwrap();

        let a = send_request(
            &client_a,
            &Request::Commit {
                consumer_id: "reader-a".to_string(),
                offset: Offset(10),
            },
        )
        .await;
        let b = send_request(
            &client_b,
            &Request::Commit {
                consumer_id: "reader-b".to_string(),
                offset: Offset(20),
            },
        )
        .await;
        assert_eq!(a, Response::Committed);
        assert_eq!(b, Response::Committed);

        assert_eq!(
            send_request(
                &client_a,
                &Request::LastCommitted {
                    consumer_id: "reader-a".to_string()
                }
            )
            .await,
            Response::LastCommitted {
                offset: Some(Offset(10))
            }
        );
        assert_eq!(
            send_request(
                &client_b,
                &Request::LastCommitted {
                    consumer_id: "reader-b".to_string()
                }
            )
            .await,
            Response::LastCommitted {
                offset: Some(Offset(20))
            }
        );

        server.abort();
    }

    /// A frame header claiming a body past `MAX_FRAME_LEN` ends the
    /// connection immediately -- the server never asks for the (in this
    /// test, never sent) body, so the client's next read sees the
    /// connection close rather than hanging waiting for a response.
    #[rusty_tokio::test]
    async fn a_frame_claiming_more_than_the_cap_ends_the_connection() {
        let (server, addr) = spawn_server().await;
        let client = TcpStream::connect(addr).await.unwrap();

        let oversized_len = MAX_FRAME_LEN + 1;
        client
            .write_all(&oversized_len.to_be_bytes())
            .await
            .unwrap();

        let mut buf = [0u8; 1];
        let result = client.read_exact(&mut buf).await;
        assert!(result.is_err());

        server.abort();
    }

    /// A frame whose encoded length lands exactly on `MAX_FRAME_LEN` -- not
    /// just comfortably under it -- is still accepted and round-trips
    /// normally. The cap rejects what's past the limit, not the limit
    /// itself.
    #[rusty_tokio::test]
    async fn a_frame_at_exactly_the_cap_is_still_accepted() {
        let (server, addr) = spawn_server().await;
        let client = TcpStream::connect(addr).await.unwrap();

        // `Request::Produce`'s encoding is 1 (opcode) + 4 (u32 payload len)
        // + payload.len() bytes -- pick a payload that makes the total
        // exactly `MAX_FRAME_LEN`, the actual boundary the server checks.
        let payload = vec![7u8; (MAX_FRAME_LEN - 5) as usize];
        let produced = send_request(&client, &Request::Produce { payload }).await;
        assert_eq!(produced, Response::Produced { offset: Offset(0) });

        server.abort();
    }

    /// Requesting shutdown stops the accept loop, but a connection already
    /// in flight keeps working until it disconnects on its own -- and
    /// `serve` itself returns `Ok(())` once that happens, with no abort
    /// needed.
    #[rusty_tokio::test]
    async fn shutdown_stops_accepting_and_drains_in_flight_connections() {
        let state = test_state().await;
        let listener = TcpListener::bind_addrs("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let server = rusty_tokio::spawn(serve(listener, state, shutdown_rx));

        let client = TcpStream::connect(addr).await.unwrap();
        let produced = send_request(
            &client,
            &Request::Produce {
                payload: b"before shutdown".to_vec(),
            },
        )
        .await;
        assert_eq!(produced, Response::Produced { offset: Offset(0) });

        shutdown_tx.send(true).unwrap();

        // The already-open connection is unaffected by the shutdown
        // request -- it keeps serving requests normally.
        let fetched = send_request(&client, &Request::Fetch { offset: Offset(0) }).await;
        assert_eq!(
            fetched,
            Response::Fetched {
                payload: b"before shutdown".to_vec()
            }
        );

        // Only once the client disconnects does the connection -- and
        // then `serve` itself, once it's drained -- actually finish.
        drop(client);
        let result = server.await.unwrap();
        assert!(result.is_ok());
    }
}
