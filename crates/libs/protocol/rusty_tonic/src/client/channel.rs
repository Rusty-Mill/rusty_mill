//! The channel: what a call goes through.

use std::sync::Arc;
use std::time::Duration;

use rusty_hyper::ConnectOptions;
use rusty_prost::Message;

use super::{Reply, Request, Response, Sender, Streaming};
use crate::{MetadataMap, Status, DEFAULT_MAX_RECV_MESSAGE_SIZE};

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

/// A connection to one gRPC server, opened by the first call. Cloning shares it.
#[derive(Clone)]
pub struct Channel {
    _inner: Arc<()>,
}

impl Channel {
    /// A channel to `url` (`http://host:port` for h2c, `https://host:port` for TLS). No I/O
    /// happens until the first call. An unusable URL is `INVALID_ARGUMENT`.
    pub fn new(url: &str, options: ChannelOptions) -> Result<Channel, Status> {
        let _ = (url, options);
        todo!()
    }

    /// One request, one reply.
    pub async fn unary<Req: Message, Resp: Message>(
        &self,
        path: &str,
        request: Request<Req>,
    ) -> Result<Response<Resp>, Status> {
        let _ = (path, request);
        todo!()
    }

    /// One request, a stream of replies. Returns once the response has started.
    pub async fn server_streaming<Req: Message, Resp: Message>(
        &self,
        path: &str,
        request: Request<Req>,
    ) -> Result<Streaming<Resp>, Status> {
        let _ = (path, request);
        todo!()
    }

    /// A stream of requests, one reply. Send with the [`Sender`] and call
    /// [`finish`](Sender::finish); then await the [`Reply`]. Dropping the sender without
    /// finishing cancels the call.
    pub async fn client_streaming<Req: Message, Resp: Message>(
        &self,
        path: &str,
        request: Request<()>,
    ) -> Result<(Sender<Req>, Reply<Resp>), Status> {
        let _ = (path, request);
        todo!()
    }

    /// A stream of requests and a stream of replies, independently. Returns once the request
    /// has been sent; replies are read from the [`Streaming`] as they come.
    pub async fn bidirectional<Req: Message, Resp: Message>(
        &self,
        path: &str,
        request: Request<()>,
    ) -> Result<(Sender<Req>, Streaming<Resp>), Status> {
        let _ = (path, request);
        todo!()
    }
}
