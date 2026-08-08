//! Socket accept loops and graceful shutdown.
//!
//! One task per port, each accepting into a per-connection hyper service. The
//! [`Gateway`] is shared behind an `Arc` rather than cloned, since the MCP
//! session managers inside it must be the same objects across connections —
//! a session id issued on one connection has to be recognised on the next.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use hyper_util::service::TowerToHyperService;
use rusty_mcp::limits::LimitsLayer;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_layer::Layer as _;

use agentgateway_proxy::Scheme;

use crate::gateway::Gateway;

/// Bind every address and serve until SIGINT or SIGTERM.
pub async fn run(
    gateway: Gateway,
    addrs: Vec<SocketAddr>,
    limits: LimitsLayer,
) -> anyhow::Result<()> {
    let shutdown = CancellationToken::new();
    let serving = run_with_shutdown_and_limits(gateway, addrs, shutdown.clone(), limits).await?;
    signal().await;
    tracing::info!("shutting down");
    shutdown.cancel();
    serving.await;
    Ok(())
}

/// Bind every address and serve until `shutdown` is cancelled.
///
/// Returns once every socket is bound, so a caller knows the gateway is
/// reachable before it starts making requests. The returned future completes
/// when serving has finished.
pub async fn run_with_shutdown(
    gateway: Gateway,
    addrs: Vec<SocketAddr>,
    shutdown: CancellationToken,
) -> anyhow::Result<impl Future<Output = ()> + Send> {
    run_with_shutdown_and_limits(gateway, addrs, shutdown, LimitsLayer::new()).await
}

/// As [`run_with_shutdown`], with process-wide load shedding applied.
///
/// The limits sit outside everything else, so a shed request costs a semaphore
/// try-acquire rather than a route lookup, a token validation and a JWKS
/// fetch. That ordering is the reason the layer is worth having: the requests
/// it refuses are exactly the ones there is no capacity to pay for.
pub async fn run_with_shutdown_and_limits(
    gateway: Gateway,
    addrs: Vec<SocketAddr>,
    shutdown: CancellationToken,
    limits: LimitsLayer,
) -> anyhow::Result<impl Future<Output = ()> + Send> {
    let gateway = Arc::new(gateway);

    let mut listeners = Vec::with_capacity(addrs.len());
    for addr in &addrs {
        // Bind every port before serving any, so a port already in use is a
        // startup failure rather than a gateway that half works.
        let listener = TcpListener::bind(addr).await?;
        tracing::info!(%addr, "listening");
        listeners.push((addr.port(), listener));
    }

    let mut tasks = Vec::with_capacity(listeners.len());
    for (port, listener) in listeners {
        let gateway = Arc::clone(&gateway);
        let shutdown = shutdown.clone();
        // One layer, cloned per port: the permit pool is shared, which is the
        // whole point -- a limit each port counted separately would bound
        // nothing in a multi-bind config.
        let limits = limits.clone();
        tasks.push(tokio::spawn(async move {
            accept_loop(gateway, port, listener, shutdown, limits).await;
        }));
    }

    Ok(async move {
        for task in tasks {
            let _ = task.await;
        }
    })
}

async fn accept_loop(
    gateway: Arc<Gateway>,
    port: u16,
    listener: TcpListener,
    shutdown: CancellationToken,
    limits: LimitsLayer,
) {
    loop {
        let stream = tokio::select! {
            _ = shutdown.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => (stream, peer),
                Err(err) => {
                    // A failed accept is usually per-connection (fd limits,
                    // a client that vanished); tearing down the listener
                    // would turn a transient problem into an outage.
                    tracing::warn!(%port, %err, "accept failed");
                    continue;
                }
            },
        };

        let (stream, peer) = stream;
        // The client address, for the forwarded-for chain a proxied backend
        // appends to.
        let peer = Some(peer.ip());

        // A passthrough port never terminates: it forwards the bytes as they
        // arrive, which is the whole point of `protocol: TLS` in the Gateway
        // API's passthrough mode.
        if let Some(forwarder) = gateway.passthrough(port) {
            tokio::spawn(async move { forwarder.forward(stream).await });
            continue;
        }

        let terminator = gateway.tls(port);
        let gateway = Arc::clone(&gateway);
        let shutdown = shutdown.clone();
        let limits = limits.clone();

        tokio::spawn(async move {
            match terminator {
                Some(terminator) => {
                    // The handshake happens on this connection's own task, so
                    // a slow or hostile client stalls only itself rather than
                    // holding up the accept loop for everyone.
                    let tls = match terminator.accept(stream).await {
                        Ok(tls) => tls,
                        Err(err) => {
                            tracing::debug!(%port, %err, "TLS handshake failed");
                            return;
                        }
                    };
                    serve_connection(tls, gateway, port, Scheme::Https, peer, limits, shutdown)
                        .await;
                }
                None => {
                    serve_connection(stream, gateway, port, Scheme::Http, peer, limits, shutdown)
                        .await;
                }
            }
        });
    }
}

/// Serve one connection, whether it arrived over TLS or in the clear.
///
/// Generic over the stream so the TLS and cleartext paths share every line
/// after the handshake: a second copy of this would be a second place for the
/// graceful-shutdown handling to drift.
async fn serve_connection<S>(
    stream: S,
    gateway: Arc<Gateway>,
    port: u16,
    scheme: Scheme,
    peer: Option<std::net::IpAddr>,
    limits: LimitsLayer,
    shutdown: CancellationToken,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    // `tower::service_fn`, not hyper's: the limits layer is a tower layer, so
    // the stack is built in tower terms and bridged to hyper once at the
    // outside.
    let service = TowerToHyperService::new(limits.layer(tower::service_fn(move |request| {
        let gateway = Arc::clone(&gateway);
        async move { gateway.handle(port, peer, scheme, request).await }
    })));

    let builder = ConnBuilder::new(TokioExecutor::new());
    let connection = builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
    tokio::pin!(connection);

    tokio::select! {
        result = connection.as_mut() => {
            if let Err(err) = result {
                tracing::debug!(%port, %err, "connection closed with error");
            }
        }
        _ = shutdown.cancelled() => {
            // Let in-flight requests finish; MCP calls can be long.
            connection.as_mut().graceful_shutdown();
            let _ = connection.await;
        }
    }
}

/// Resolve on SIGINT or SIGTERM.
async fn signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(term) => term,
            Err(err) => {
                tracing::warn!(%err, "cannot listen for SIGTERM; only SIGINT will stop the gateway");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
