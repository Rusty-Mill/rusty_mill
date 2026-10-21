//! The channel: what a call goes through.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rusty_h2::driver::{ClientHandle, Error as H2Error, ResponseFuture, SendStream};
use rusty_h2::error::ErrorCode;
use rusty_http::Url;
use rusty_hyper::{connect, ConnectOptions, Connection, HeaderField};
use rusty_prost::Message;
use rusty_tokio::sync::{watch, Mutex};

use super::error::{status_from_connect, status_from_h2};
use super::streaming::{frame, until, Cancel};
use super::{Reply, Request, Response, Sender, Streaming};
use crate::{MetadataMap, RequestHead, Scheme, Status, DEFAULT_MAX_RECV_MESSAGE_SIZE};

/// Runs before each call and may change its metadata (an auth token, say). An `Err` ends the
/// call with that status without sending anything.
pub type Interceptor = Arc<dyn Fn(&mut MetadataMap) -> Result<(), Status> + Send + Sync>;

/// How a [`Channel`] connects and calls.
pub struct ChannelOptions {
    /// How the connection is made (TLS trust, a client certificate, timeouts, keepalive).
    pub connect: ConnectOptions,
    /// A deadline for every call; a call's own, if shorter, wins. None by default.
    pub timeout: Option<Duration>,
    /// The longest message accepted from the server; a longer one is `RESOURCE_EXHAUSTED`.
    pub max_recv_message_size: usize,
    /// The longest message sent; a longer one fails the call with `RESOURCE_EXHAUSTED` before
    /// anything is sent. None (unlimited) by default, as in `tonic`.
    pub max_send_message_size: Option<usize>,
    /// The `user-agent` sent.
    pub user_agent: String,
    /// Runs before each call.
    pub interceptor: Option<Interceptor>,
}

impl Default for ChannelOptions {
    fn default() -> Self {
        ChannelOptions {
            connect: ConnectOptions::default(),
            timeout: None,
            max_recv_message_size: DEFAULT_MAX_RECV_MESSAGE_SIZE,
            max_send_message_size: None,
            user_agent: format!("rusty-tonic/{}", env!("CARGO_PKG_VERSION")),
            interceptor: None,
        }
    }
}

/// The connection in use, and which one it is, so a failure retires only its own.
struct Live {
    generation: u64,
    connection: Connection,
}

struct Inner {
    url: String,
    authority: String,
    scheme: Scheme,
    options: ChannelOptions,
    live: Mutex<(u64, Option<Live>)>,
}

/// A connection to one gRPC server, opened by the first call and reopened if it is lost.
/// Cloning shares it.
#[derive(Clone)]
pub struct Channel {
    inner: Arc<Inner>,
}

/// A stream opened for a call, its response to come, and the call's deadline.
struct Started {
    send: SendStream,
    response: ResponseFuture,
    deadline: Option<Instant>,
}

fn deadline_exceeded() -> Status {
    Status::deadline_exceeded("the call's deadline passed")
}

impl Channel {
    /// A channel to `url` (`http://host:port` for h2c, `https://host:port` for TLS). No I/O
    /// happens until the first call. An unusable URL is `INVALID_ARGUMENT`.
    pub fn new(url: &str, options: ChannelOptions) -> Result<Channel, Status> {
        let parsed = Url::parse(url)
            .map_err(|e| Status::invalid_argument(format!("invalid URL `{url}`: {e}")))?;
        let scheme = if parsed.scheme == "https" {
            Scheme::Https
        } else {
            Scheme::Http
        };
        Ok(Channel {
            inner: Arc::new(Inner {
                url: url.to_string(),
                authority: parsed.host_header(),
                scheme,
                options,
                live: Mutex::new((0, None)),
            }),
        })
    }

    /// One request, one reply.
    pub async fn unary<Req: Message, Resp: Message>(
        &self,
        path: &str,
        request: Request<Req>,
    ) -> Result<Response<Resp>, Status> {
        self.server_streaming(path, request).await?.single().await
    }

    /// One request, a stream of replies. Returns once the response has started.
    pub async fn server_streaming<Req: Message, Resp: Message>(
        &self,
        path: &str,
        request: Request<Req>,
    ) -> Result<Streaming<Resp>, Status> {
        let options = &self.inner.options;
        let data = frame(&request.message, options.max_send_message_size)?;
        let Started {
            mut send,
            response,
            deadline,
        } = self.start(path, request.metadata, request.timeout).await?;
        match until(deadline, send.send_data(data, true)).await {
            None => {
                send.reset(ErrorCode::Cancel);
                return Err(deadline_exceeded());
            }
            // The server may answer before it has read the request and then reset the stream
            // (NO_ERROR) or have closed it; what it answered is what the call is about.
            Some(Ok(()) | Err(H2Error::Reset(ErrorCode::NoError) | H2Error::Closed)) => {}
            Some(Err(e)) => return Err(status_from_h2(&e)),
        }
        let mut streaming = Streaming::new(
            response,
            options.max_recv_message_size,
            deadline,
            Cancel::Reset(send),
        );
        streaming.open().await?;
        match streaming.failed_at_once() {
            Some(status) => Err(status),
            None => Ok(streaming),
        }
    }

    /// A stream of requests, one reply. Send with the [`Sender`] and call
    /// [`finish`](Sender::finish); then await the [`Reply`]. Dropping the sender without
    /// finishing cancels the call.
    pub async fn client_streaming<Req: Message, Resp: Message>(
        &self,
        path: &str,
        request: Request<()>,
    ) -> Result<(Sender<Req>, Reply<Resp>), Status> {
        let (sender, streaming) = self.bidirectional(path, request).await?;
        Ok((sender, Reply { streaming }))
    }

    /// A stream of requests and a stream of replies, independently. Returns once the request
    /// has been sent; replies are read from the [`Streaming`] as they come.
    pub async fn bidirectional<Req: Message, Resp: Message>(
        &self,
        path: &str,
        request: Request<()>,
    ) -> Result<(Sender<Req>, Streaming<Resp>), Status> {
        let options = &self.inner.options;
        let Started {
            send,
            response,
            deadline,
        } = self.start(path, request.metadata, request.timeout).await?;
        let (signal, cancelled) = watch::channel(false);
        let streaming = Streaming::new(
            response,
            options.max_recv_message_size,
            deadline,
            Cancel::Signal(signal),
        );
        let sender = Sender::new(send, cancelled, options.max_send_message_size, deadline);
        Ok((sender, streaming))
    }

    /// Runs the interceptor, works out the deadline and opens a stream with the request headers.
    async fn start(
        &self,
        path: &str,
        mut metadata: MetadataMap,
        timeout: Option<Duration>,
    ) -> Result<Started, Status> {
        let options = &self.inner.options;
        if let Some(interceptor) = &options.interceptor {
            interceptor(&mut metadata)?;
        }
        let timeout = match (timeout, options.timeout) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let deadline = timeout.map(|t| Instant::now() + t);
        match until(deadline, self.open(path, metadata, deadline)).await {
            None => Err(deadline_exceeded()),
            Some(opened) => opened.map(|(send, response)| Started {
                send,
                response,
                deadline,
            }),
        }
    }

    /// Opens a stream on the connection, connecting first if there is none. If the connection
    /// turns out to be closed or going away, nothing was sent on it, so the call is tried once
    /// more on a fresh one.
    async fn open(
        &self,
        path: &str,
        metadata: MetadataMap,
        deadline: Option<Instant>,
    ) -> Result<(SendStream, ResponseFuture), Status> {
        let mut head = RequestHead::new(path, self.inner.authority.clone());
        head.scheme = self.inner.scheme;
        head.user_agent = Some(self.inner.options.user_agent.clone());
        head.metadata = metadata;
        let mut retry = true;
        loop {
            let (generation, handle) = self.handle().await?;
            // The timeout sent is what is left of the deadline now, after any connecting.
            head.timeout = deadline.map(|d| d.saturating_duration_since(Instant::now()));
            let fields = head
                .to_headers()
                .into_iter()
                .map(|(name, value)| HeaderField::new(name.as_str(), value.as_slice()))
                .collect();
            match handle.send_request(fields, false).await {
                Ok(opened) => return Ok(opened),
                Err(e @ (H2Error::Closed | H2Error::GoAway(_))) => {
                    self.retire(generation).await;
                    if !std::mem::take(&mut retry) {
                        return Err(status_from_h2(&e));
                    }
                }
                Err(e) => {
                    if matches!(e, H2Error::Io(_)) {
                        self.retire(generation).await;
                    }
                    return Err(status_from_h2(&e));
                }
            }
        }
    }

    /// A handle on the live connection, opening one if needed.
    async fn handle(&self) -> Result<(u64, ClientHandle), Status> {
        let mut live = self.inner.live.lock().await;
        if let Some(l) = &live.1 {
            return Ok((l.generation, l.connection.client().clone()));
        }
        let connection = connect(&self.inner.url, self.inner.options.connect.clone())
            .await
            .map_err(|e| status_from_connect(&e))?;
        live.0 += 1;
        let generation = live.0;
        let handle = connection.client().clone();
        live.1 = Some(Live {
            generation,
            connection,
        });
        Ok((generation, handle))
    }

    /// Drops connection `generation`, if it is still the live one, so the next call connects.
    async fn retire(&self, generation: u64) {
        let mut live = self.inner.live.lock().await;
        if live.1.as_ref().is_some_and(|l| l.generation == generation) {
            live.1 = None;
        }
    }
}
