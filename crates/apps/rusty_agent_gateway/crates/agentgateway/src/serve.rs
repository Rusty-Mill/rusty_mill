//! Listeners and graceful shutdown, on `rusty_axum`.
//!
//! One server per port. The [`Gateway`] is shared behind an `Arc` rather than
//! cloned, since the MCP session managers inside it must be the same objects
//! across connections — a session id issued on one connection has to be
//! recognised on the next.
//!
//! Each accepted socket goes through [`Acceptor`] first: a passthrough port
//! forwards it untouched, a TLS port completes the handshake with the
//! certificate its SNI name picks, and a cleartext port serves it as it is.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use agentgateway_proxy::Scheme;
use agentgateway_tls::{Passthrough, TlsTerminator};
use rusty_axum::{Accept, Accepted, Admission, BoxFuture, Router, ServeOptions};
use rusty_tokio::io::{TcpListener, TcpStream};
use rusty_tokio::sync::CancellationToken;

use crate::convert;
use crate::gateway::Gateway;

/// Connections served at once on one port; the next is closed unserved.
/// High, since an MCP or SSE stream holds its connection for its whole life:
/// this is a bound on descriptors, not a load limit (`limits` is that).
const MAX_CONNECTIONS: usize = 16 * 1024;

/// The largest request head. Bearer tokens make heads large, so this is well
/// above the 8 KiB default.
const MAX_HEAD_BYTES: usize = 64 * 1024;

/// Bind every address and serve until SIGINT or SIGTERM.
pub async fn run(
    gateway: Gateway,
    addrs: Vec<SocketAddr>,
    limits: Admission,
) -> anyhow::Result<()> {
    let shutdown = CancellationToken::new();
    let serving = run_with_shutdown_and_limits(gateway, addrs, shutdown.clone(), limits).await?;
    signal().await;
    rusty_log::info!("shutting down");
    shutdown.cancel();
    serving.await;
    Ok(())
}

/// Bind every address and serve until `shutdown` is cancelled.
///
/// Returns once every socket is bound, so a caller knows the gateway is
/// reachable before it starts making requests. Serving does not depend on the
/// returned future being polled; it completes once serving has finished,
/// after `shutdown` is cancelled and requests in flight have been answered.
pub async fn run_with_shutdown(
    gateway: Gateway,
    addrs: Vec<SocketAddr>,
    shutdown: CancellationToken,
) -> anyhow::Result<impl Future<Output = ()> + Send> {
    run_with_shutdown_and_limits(gateway, addrs, shutdown, Admission::new()).await
}

/// As [`run_with_shutdown`], with process-wide load shedding applied.
///
/// The limits are checked by the server as soon as a request's head arrives,
/// before its body is read, so a shed request costs neither an upload held in
/// memory nor a route lookup, a token validation and a JWKS fetch. The
/// timeout covers reading the body as well as producing the response. That
/// ordering is the reason the limits are worth having: the requests they
/// refuse are exactly the ones there is no capacity to pay for.
pub async fn run_with_shutdown_and_limits(
    gateway: Gateway,
    addrs: Vec<SocketAddr>,
    shutdown: CancellationToken,
    limits: Admission,
) -> anyhow::Result<impl Future<Output = ()> + Send> {
    let gateway = Arc::new(gateway);
    let options = options(&gateway);

    let mut listeners = Vec::with_capacity(addrs.len());
    for addr in &addrs {
        // Bind every port before serving any, so a port already in use is a
        // startup failure rather than a gateway that half works.
        let listener = TcpListener::bind(addr).await?;
        rusty_log::info!(%addr, "listening");
        listeners.push((addr.port(), listener));
    }

    let mut servers = Vec::with_capacity(listeners.len());
    for (port, listener) in listeners {
        let tls = gateway.tls(port);
        let scheme = if tls.is_some() {
            Scheme::Https
        } else {
            Scheme::Http
        };
        let acceptor = Acceptor {
            port,
            tls,
            passthrough: gateway.passthrough(port),
            // Cloned per port, and clones share one count: a limit each port
            // counted separately would bound nothing in a multi-bind config.
            admission: limits.clone(),
        };
        let router = router(Arc::clone(&gateway), port, scheme);
        servers.push(rusty_axum::serve_with(
            listener,
            acceptor,
            options.clone(),
            router,
        )?);
    }

    // The servers run on their own tasks; this one holds them until told to
    // stop, so serving goes on whether or not the caller polls what it gets.
    let stopped = rusty_tokio::spawn(async move {
        shutdown.cancelled().await;
        // Every port stops accepting at once; then in-flight requests finish
        // (MCP calls can be long). Draining one port before signalling the
        // next would leave the rest accepting behind a slow request.
        let draining: Vec<_> = servers
            .into_iter()
            .map(|server| rusty_tokio::spawn(server.shutdown()))
            .collect();
        for drained in draining {
            let _ = drained.await;
        }
    });
    Ok(async move {
        let _ = stopped.await;
    })
}

/// How every port is served.
fn options(gateway: &Gateway) -> ServeOptions {
    ServeOptions {
        max_connections: MAX_CONNECTIONS,
        max_head_bytes: MAX_HEAD_BYTES,
        // The server reads a body whole before routing, so its cap is the
        // largest any route accepts. Routes with lower caps refuse past them
        // with their own 413.
        max_body_bytes: gateway.max_request_body() as u64,
        // How long a keep-alive connection may sit between requests, and
        // each read or blocked write may take. A client may well idle between
        // MCP calls; five minutes still closes a dead peer. A response being
        // produced is not bounded by it: the route's timeout is.
        idle_timeout: Duration::from_secs(300),
        handshake_timeout: Duration::from_secs(10),
        // The TLS listeners advertise `h2` over ALPN, so they must speak it.
        http2: true,
    }
}

/// The router for one port: every request goes to the gateway.
fn router(gateway: Arc<Gateway>, port: u16, scheme: Scheme) -> Router {
    Router::new().fallback(move |request: rusty_axum::Request| {
        let gateway = Arc::clone(&gateway);
        async move {
            // The client address, for the forwarded-for chain a proxied
            // backend appends to.
            let peer = request.peer_addr().map(|addr| addr.ip());
            let request = match convert::request(request) {
                Ok(request) => request,
                Err(refused) => return refused,
            };
            let response = match gateway.handle(port, peer, scheme, request).await {
                Ok(response) => response,
                Err(never) => match never {},
            };
            convert::response(response).await
        }
    })
}

/// How a port treats an accepted socket.
struct Acceptor {
    port: u16,
    tls: Option<Arc<TlsTerminator>>,
    passthrough: Option<Arc<Passthrough>>,
    admission: Admission,
}

impl Accept for Acceptor {
    fn accept(&self, tcp: TcpStream, _peer: SocketAddr) -> BoxFuture<'static, Accepted> {
        // A passthrough port never terminates: it forwards the bytes as they
        // arrive, which is the whole point of `protocol: TLS` in the Gateway
        // API's passthrough mode.
        if let Some(forwarder) = self.passthrough.clone() {
            rusty_tokio::spawn(async move { forwarder.forward(tcp).await });
            return Box::pin(async { Accepted::Done });
        }
        let Some(terminator) = self.tls.clone() else {
            return Box::pin(async move { Accepted::Plain(tcp) });
        };
        let port = self.port;
        // The handshake runs on the connection's own task, so a slow or
        // hostile client stalls only itself.
        Box::pin(async move {
            match terminator.accept(tcp).await {
                Ok(tls) => Accepted::Tls(Box::new(tls)),
                Err(err) => {
                    rusty_log::debug!(%port, %err, "TLS handshake failed");
                    Accepted::Done
                }
            }
        })
    }

    fn admission(&self) -> Option<&Admission> {
        Some(&self.admission)
    }
}

/// Resolve on SIGINT or SIGTERM.
async fn signal() {
    #[cfg(unix)]
    {
        use rusty_tokio::signal::{SignalKind, signal};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(term) => term,
            Err(err) => {
                rusty_log::warn!(%err, "cannot listen for SIGTERM; only SIGINT will stop the gateway");
                let _ = rusty_tokio::signal::ctrl_c().await;
                return;
            }
        };
        rusty_tokio::select! {
            _ = rusty_tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = rusty_tokio::signal::ctrl_c().await;
    }
}
