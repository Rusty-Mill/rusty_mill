//! Methods by path, and serving them.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use rusty_h2::driver::{Request as H2Request, Respond};
use rusty_hyper::{serve, ServeOptions, Server};
use rusty_prost::Message;
use rusty_tls::TlsAcceptor;
use rusty_tokio::io::TcpListener;

use super::call::{trailers_only, RawIncoming, Shared};
use super::{Incoming, Sink};
use crate::client::{until, Interceptor, Request, Response};
use crate::{MetadataMap, RequestHead, Status, DEFAULT_MAX_RECV_MESSAGE_SIZE};

type Done = Pin<Box<dyn Future<Output = Result<(), Status>> + Send>>;

/// What a method's adapter is given: the parsed head, the request body and the response.
struct Call {
    head: RequestHead,
    incoming: RawIncoming,
    shared: Arc<Shared>,
}

type Route = Box<dyn Fn(Call) -> Done + Send + Sync>;

/// The methods of a server, each a closure registered under its `/package.Service/Method` path.
pub struct Router {
    routes: HashMap<String, Route>,
    interceptor: Option<Interceptor>,
    max_recv: usize,
    max_send: Option<usize>,
}

impl Default for Router {
    fn default() -> Self {
        Router::new()
    }
}

impl Router {
    /// A router with no methods: every call is `UNIMPLEMENTED`.
    pub fn new() -> Router {
        Router {
            routes: HashMap::new(),
            interceptor: None,
            max_recv: DEFAULT_MAX_RECV_MESSAGE_SIZE,
            max_send: None,
        }
    }

    fn route(mut self, path: &str, route: Route) -> Router {
        self.routes.insert(path.to_string(), route);
        self
    }

    /// A method with one request and one reply.
    #[must_use]
    pub fn unary<Req, Resp, F, Fut>(self, path: &str, handler: F) -> Router
    where
        Req: Message + Send + 'static,
        Resp: Message + Send + 'static,
        F: Fn(Request<Req>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response<Resp>, Status>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        self.route(
            path,
            Box::new(move |call| {
                let handler = handler.clone();
                Box::pin(async move {
                    let Call {
                        head,
                        mut incoming,
                        shared,
                    } = call;
                    let message = incoming.single::<Req>().await?;
                    let reply = handler(request(head, message)).await?;
                    let Response {
                        message,
                        metadata,
                        trailers,
                    } = reply;
                    shared.set_metadata(metadata);
                    shared.set_trailers(trailers);
                    // Framed before awaiting, so the reply is not held across it.
                    let data = shared.encode(&message);
                    drop(message);
                    shared.send_frame(data?).await
                })
            }),
        )
    }

    /// A method with one request and a stream of replies.
    #[must_use]
    pub fn server_streaming<Req, Resp, F, Fut>(self, path: &str, handler: F) -> Router
    where
        Req: Message + Send + 'static,
        Resp: Message + Send + 'static,
        F: Fn(Request<Req>, Sink<Resp>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), Status>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        self.route(
            path,
            Box::new(move |call| {
                let handler = handler.clone();
                Box::pin(async move {
                    let Call {
                        head,
                        mut incoming,
                        shared,
                    } = call;
                    let message = incoming.single::<Req>().await?;
                    handler(request(head, message), Sink::new(shared)).await
                })
            }),
        )
    }

    /// A method with a stream of requests and one reply.
    #[must_use]
    pub fn client_streaming<Req, Resp, F, Fut>(self, path: &str, handler: F) -> Router
    where
        Req: Message + Send + 'static,
        Resp: Message + Send + 'static,
        F: Fn(Request<Incoming<Req>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response<Resp>, Status>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        self.route(
            path,
            Box::new(move |call| {
                let handler = handler.clone();
                Box::pin(async move {
                    let Call {
                        head,
                        incoming,
                        shared,
                    } = call;
                    let reply = handler(request(head, Incoming::new(incoming))).await?;
                    let Response {
                        message,
                        metadata,
                        trailers,
                    } = reply;
                    shared.set_metadata(metadata);
                    shared.set_trailers(trailers);
                    // Framed before awaiting, so the reply is not held across it.
                    let data = shared.encode(&message);
                    drop(message);
                    shared.send_frame(data?).await
                })
            }),
        )
    }

    /// A method with a stream of requests and a stream of replies.
    #[must_use]
    pub fn bidirectional<Req, Resp, F, Fut>(self, path: &str, handler: F) -> Router
    where
        Req: Message + Send + 'static,
        Resp: Message + Send + 'static,
        F: Fn(Request<Incoming<Req>>, Sink<Resp>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), Status>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        self.route(
            path,
            Box::new(move |call| {
                let handler = handler.clone();
                Box::pin(async move {
                    let Call {
                        head,
                        incoming,
                        shared,
                    } = call;
                    handler(request(head, Incoming::new(incoming)), Sink::new(shared)).await
                })
            }),
        )
    }

    /// Runs before each call, on the request's metadata; an `Err` ends the call with that
    /// status without running the handler.
    #[must_use]
    pub fn with_interceptor(mut self, interceptor: Interceptor) -> Router {
        self.interceptor = Some(interceptor);
        self
    }

    /// The longest request message accepted (default 4 MiB); a longer one is
    /// `RESOURCE_EXHAUSTED`.
    #[must_use]
    pub fn with_max_recv_message_size(mut self, max: usize) -> Router {
        self.max_recv = max;
        self
    }

    /// The longest reply message sent (default unlimited); a longer one fails the `send`.
    #[must_use]
    pub fn with_max_send_message_size(mut self, max: usize) -> Router {
        self.max_send = Some(max);
        self
    }

    /// Serves the methods on `listener`, over TLS if `tls` is given (it must offer ALPN `h2`).
    pub fn serve(
        self,
        listener: TcpListener,
        tls: Option<TlsAcceptor>,
        options: ServeOptions,
    ) -> io::Result<Server> {
        let router = Arc::new(self);
        serve(listener, tls, options, move |req, respond| {
            let router = router.clone();
            async move { router.dispatch(req, respond).await }
        })
    }

    /// One call, from its headers to its status.
    async fn dispatch(&self, req: H2Request, respond: Respond) {
        let H2Request { headers, body } = req;
        let pairs: Vec<(&str, &[u8])> = headers
            .iter()
            .filter_map(|h| Some((std::str::from_utf8(&h.name).ok()?, h.value.as_slice())))
            .collect();
        let mut head = match RequestHead::from_headers(pairs) {
            Ok(head) => head,
            Err(status) => return refuse(respond, &status).await,
        };
        let Some(route) = self.routes.get(&head.path) else {
            let status = Status::unimplemented(format!("unknown method {}", head.path));
            return refuse(respond, &status).await;
        };
        if let Some(interceptor) = &self.interceptor {
            if let Err(status) = interceptor(&mut head.metadata) {
                return refuse(respond, &status).await;
            }
        }
        let deadline = head.timeout.map(|t| Instant::now() + t);
        // Taken before `respond` moves into the call: it tells us when the client is gone.
        let closed = respond.closed();
        let shared = Shared::new(respond, self.max_send);
        let call = Call {
            head,
            incoming: RawIncoming::new(body, self.max_recv),
            shared: shared.clone(),
        };
        // A handler that is only waiting (on a lock, on work elsewhere) would never notice the
        // client leave: nothing it does reads or writes. So the call ends when the stream does,
        // and the handler is dropped, as `tonic` drops it.
        let ended = rusty_tokio::select! {
            outcome = until(deadline, route(call)) => Some(outcome),
            () = closed => None,
        };
        let Some(outcome) = ended else {
            return;
        };
        let status = match outcome {
            None => Status::deadline_exceeded("the call's deadline passed"),
            Some(Ok(())) => Status::ok(),
            Some(Err(status)) => status,
        };
        shared.finish(status).await;
    }
}

fn request<T>(head: RequestHead, message: T) -> Request<T> {
    Request {
        message,
        metadata: head.metadata,
        timeout: head.timeout,
    }
}

/// Ends a call that never reached its handler. A failure to write means the client is gone.
async fn refuse(respond: Respond, status: &Status) {
    let _ = trailers_only(respond, &MetadataMap::new(), status).await;
}
