//! Methods by path, and serving them.

use std::future::Future;
use std::io;

use rusty_hyper::{ServeOptions, Server};
use rusty_prost::Message;
use rusty_tls::TlsAcceptor;
use rusty_tokio::io::TcpListener;

use super::{Incoming, Sink};
use crate::client::{Interceptor, Request, Response};
use crate::Status;

/// The methods of a server, each a closure registered under its `/package.Service/Method` path.
#[derive(Default)]
pub struct Router {
    _private: (),
}

impl Router {
    /// A router with no methods: every call is `UNIMPLEMENTED`.
    pub fn new() -> Router {
        Router::default()
    }

    /// A method with one request and one reply.
    #[must_use]
    pub fn unary<Req, Resp, F, Fut>(self, path: &str, handler: F) -> Router
    where
        Req: Message + 'static,
        Resp: Message + 'static,
        F: Fn(Request<Req>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response<Resp>, Status>> + Send + 'static,
    {
        let _ = (path, handler);
        todo!()
    }

    /// A method with one request and a stream of replies.
    #[must_use]
    pub fn server_streaming<Req, Resp, F, Fut>(self, path: &str, handler: F) -> Router
    where
        Req: Message + 'static,
        Resp: Message + 'static,
        F: Fn(Request<Req>, Sink<Resp>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), Status>> + Send + 'static,
    {
        let _ = (path, handler);
        todo!()
    }

    /// A method with a stream of requests and one reply.
    #[must_use]
    pub fn client_streaming<Req, Resp, F, Fut>(self, path: &str, handler: F) -> Router
    where
        Req: Message + 'static,
        Resp: Message + 'static,
        F: Fn(Request<Incoming<Req>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response<Resp>, Status>> + Send + 'static,
    {
        let _ = (path, handler);
        todo!()
    }

    /// A method with a stream of requests and a stream of replies.
    #[must_use]
    pub fn bidirectional<Req, Resp, F, Fut>(self, path: &str, handler: F) -> Router
    where
        Req: Message + 'static,
        Resp: Message + 'static,
        F: Fn(Request<Incoming<Req>>, Sink<Resp>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), Status>> + Send + 'static,
    {
        let _ = (path, handler);
        todo!()
    }

    /// Runs before each call, on the request's metadata; an `Err` ends the call with that
    /// status without running the handler.
    #[must_use]
    pub fn with_interceptor(self, interceptor: Interceptor) -> Router {
        let _ = interceptor;
        todo!()
    }

    /// The longest request message accepted (default 4 MiB); a longer one is
    /// `RESOURCE_EXHAUSTED`.
    #[must_use]
    pub fn with_max_recv_message_size(self, max: usize) -> Router {
        let _ = max;
        todo!()
    }

    /// The longest reply message sent (default unlimited); a longer one fails the `send`.
    #[must_use]
    pub fn with_max_send_message_size(self, max: usize) -> Router {
        let _ = max;
        todo!()
    }

    /// Serves the methods on `listener`, over TLS if `tls` is given (it must offer ALPN `h2`).
    pub fn serve(
        self,
        listener: TcpListener,
        tls: Option<TlsAcceptor>,
        options: ServeOptions,
    ) -> io::Result<Server> {
        let _ = (listener, tls, options);
        todo!()
    }
}
