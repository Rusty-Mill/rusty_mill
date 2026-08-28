//! A Rust client for `rusty_stream`'s wire protocol (`docs/phase1-scope.md`
//! §2's "Client SDK — Rust first") — the last piece of Phase 1's scope
//! list, completing the loop [`crate::protocol`] encodes and
//! [`crate::server`] serves: connect once, then call [`Client::produce`],
//! [`Client::fetch`], [`Client::commit`], and [`Client::last_committed`]
//! instead of hand-framing requests the way `server.rs`'s own tests
//! previously had to.
//!
//! One [`Client`] wraps one [`rusty_tokio::io::TcpStream`] and is not safe
//! to use from two tasks at once — nothing here serializes concurrent
//! calls, since the request/response protocol has no way to match a
//! response back to a specific in-flight request if two are interleaved on
//! the same connection. Open one `Client` per task, or wrap it in a lock,
//! the same tradeoff [`crate::server::AppState`] makes explicit for its own
//! shared state.
//!
//! A server-side [`Response::Error`] surfaces as [`ClientError::Server`]
//! rather than panicking or silently discarding the message — the whole
//! point of the server encoding a real error response instead of just
//! dropping the connection.

use std::fmt;

use rusty_tokio::io::{TcpStream, ToSocketAddrs};

use crate::offset::Offset;
use crate::protocol::{self, ProtocolError, Request, Response};

/// Everything that can go wrong making a request: a real I/O failure, a
/// response that failed to decode, the server reporting an application
/// error, or the server returning a well-formed but nonsensical response
/// (e.g. `Committed` in reply to a `Fetch`) — which would itself be a
/// protocol bug, not something a caller should have to match on ad hoc.
#[derive(Debug)]
pub enum ClientError {
    Io(std::io::Error),
    Protocol(ProtocolError),
    Server(String),
    UnexpectedResponse(Response),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "io error: {e}"),
            ClientError::Protocol(e) => write!(f, "protocol error: {e:?}"),
            ClientError::Server(message) => write!(f, "server error: {message}"),
            ClientError::UnexpectedResponse(response) => {
                write!(f, "unexpected response: {response:?}")
            }
        }
    }
}

impl std::error::Error for ClientError {}

impl From<std::io::Error> for ClientError {
    fn from(e: std::io::Error) -> Self {
        ClientError::Io(e)
    }
}

impl From<ProtocolError> for ClientError {
    fn from(e: ProtocolError) -> Self {
        ClientError::Protocol(e)
    }
}

/// A connection to a `rusty_stream` server, speaking [`crate::protocol`]
/// over one [`rusty_tokio::io::TcpStream`].
pub struct Client {
    stream: TcpStream,
}

impl Client {
    /// Opens a new connection to `addr`. Nothing is sent until the first
    /// request — connecting alone doesn't touch the server's `Log` or
    /// `ConsumerOffsets`.
    pub async fn connect(addr: impl ToSocketAddrs) -> Result<Self, ClientError> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self { stream })
    }

    /// Appends `payload` to the log and returns the offset it landed at.
    pub async fn produce(&self, payload: &[u8]) -> Result<Offset, ClientError> {
        match self
            .request(Request::Produce {
                payload: payload.to_vec(),
            })
            .await?
        {
            Response::Produced { offset } => Ok(offset),
            other => Err(ClientError::UnexpectedResponse(other)),
        }
    }

    /// Reads the record at `offset` back out.
    pub async fn fetch(&self, offset: Offset) -> Result<Vec<u8>, ClientError> {
        match self.request(Request::Fetch { offset }).await? {
            Response::Fetched { payload } => Ok(payload),
            other => Err(ClientError::UnexpectedResponse(other)),
        }
    }

    /// Records that `consumer_id` has processed up to and including
    /// `offset`.
    pub async fn commit(&self, consumer_id: &str, offset: Offset) -> Result<(), ClientError> {
        match self
            .request(Request::Commit {
                consumer_id: consumer_id.to_string(),
                offset,
            })
            .await?
        {
            Response::Committed => Ok(()),
            other => Err(ClientError::UnexpectedResponse(other)),
        }
    }

    /// Reads `consumer_id`'s last-committed offset, or `None` if it's
    /// never committed.
    pub async fn last_committed(&self, consumer_id: &str) -> Result<Option<Offset>, ClientError> {
        match self
            .request(Request::LastCommitted {
                consumer_id: consumer_id.to_string(),
            })
            .await?
        {
            Response::LastCommitted { offset } => Ok(offset),
            other => Err(ClientError::UnexpectedResponse(other)),
        }
    }

    /// Encodes and frames `req`, writes it, reads and decodes one framed
    /// response back. A [`Response::Error`] is unwrapped into
    /// [`ClientError::Server`] here so every other method's match arms only
    /// have to handle the responses that actually mean success.
    async fn request(&self, req: Request) -> Result<Response, ClientError> {
        let encoded = protocol::encode_request(&req);
        let framed = protocol::frame(&encoded);
        self.stream.write_all(&framed).await?;

        let mut header = [0u8; 4];
        self.stream.read_exact(&mut header).await?;
        let len = protocol::frame_len(header) as usize;
        let mut body = vec![0u8; len];
        self.stream.read_exact(&mut body).await?;

        match protocol::decode_response(&body)? {
            Response::Error { message } => Err(ClientError::Server(message)),
            other => Ok(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rusty_tokio::io::SimDriver;

    use super::*;
    use crate::clock::SimClock;
    use crate::consumer::ConsumerOffsets;
    use crate::retention::{Log, RetentionPolicy};
    use crate::server::{self, AppState};

    fn unbounded_policy() -> RetentionPolicy {
        RetentionPolicy {
            max_segment_bytes: 1_000_000,
            max_total_bytes: None,
            max_segment_age_millis: None,
        }
    }

    async fn spawn_server() -> (
        rusty_tokio::JoinHandle<std::io::Result<()>>,
        std::net::SocketAddr,
    ) {
        let driver = SimDriver::new();
        let clock = Arc::new(SimClock::new());
        let log = Log::create(driver.clone(), clock, "/log", unbounded_policy())
            .await
            .unwrap();
        let consumer_offsets = ConsumerOffsets::create_on(driver, "/offsets")
            .await
            .unwrap();
        let state = AppState {
            log: Arc::new(rusty_tokio::sync::Mutex::new(log)),
            consumer_offsets: Arc::new(rusty_tokio::sync::Mutex::new(consumer_offsets)),
        };

        let listener = rusty_tokio::io::TcpListener::bind_addrs("127.0.0.1:0")
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = rusty_tokio::sync::watch::channel(false);
        // Every test using this helper tears down via `server.abort()`,
        // not graceful shutdown -- leaking the sender keeps
        // `shutdown_rx.changed()` pending for this server's lifetime;
        // `server.rs`'s own tests exercise graceful shutdown directly.
        std::mem::forget(shutdown_tx);
        let server = rusty_tokio::spawn(server::serve(listener, state, shutdown_rx));
        (server, addr)
    }

    /// The client's own round trip, exercised against a real socket and a
    /// real server -- not a re-test of `protocol.rs`'s encode/decode
    /// (already covered there), but proof `Client` itself drives the wire
    /// correctly end to end.
    #[rusty_tokio::test]
    async fn produce_then_fetch_round_trips_through_the_client() {
        let (server, addr) = spawn_server().await;
        let client = Client::connect(addr).await.unwrap();

        let offset = client.produce(b"hello from the client").await.unwrap();
        let payload = client.fetch(offset).await.unwrap();
        assert_eq!(payload, b"hello from the client");

        server.abort();
    }

    #[rusty_tokio::test]
    async fn commit_then_last_committed_round_trips_through_the_client() {
        let (server, addr) = spawn_server().await;
        let client = Client::connect(addr).await.unwrap();

        client.commit("reader-1", Offset(5)).await.unwrap();
        let last = client.last_committed("reader-1").await.unwrap();
        assert_eq!(last, Some(Offset(5)));

        server.abort();
    }

    #[rusty_tokio::test]
    async fn last_committed_for_an_unknown_consumer_is_none_not_an_error() {
        let (server, addr) = spawn_server().await;
        let client = Client::connect(addr).await.unwrap();

        let last = client.last_committed("never-seen").await.unwrap();
        assert_eq!(last, None);

        server.abort();
    }

    /// A `Fetch` for an offset that doesn't exist comes back as
    /// `ClientError::Server`, carrying the server's own error message --
    /// not a panic, and not silently discarded.
    #[rusty_tokio::test]
    async fn fetching_an_unknown_offset_is_a_server_error() {
        let (server, addr) = spawn_server().await;
        let client = Client::connect(addr).await.unwrap();

        let err = client.fetch(Offset(0)).await.unwrap_err();
        assert!(matches!(err, ClientError::Server(_)));

        server.abort();
    }

    /// Two independent `Client`s against the same server interleave safely
    /// -- the same cross-connection property `server.rs`'s own tests
    /// verify, now exercised through the client API a real caller would
    /// actually use.
    #[rusty_tokio::test]
    async fn two_clients_against_the_same_server_stay_independent() {
        let (server, addr) = spawn_server().await;
        let client_a = Client::connect(addr).await.unwrap();
        let client_b = Client::connect(addr).await.unwrap();

        let off_a = client_a.produce(b"from a").await.unwrap();
        let off_b = client_b.produce(b"from b").await.unwrap();
        assert_ne!(off_a, off_b);

        assert_eq!(client_a.fetch(off_a).await.unwrap(), b"from a");
        assert_eq!(client_b.fetch(off_b).await.unwrap(), b"from b");

        server.abort();
    }
}
