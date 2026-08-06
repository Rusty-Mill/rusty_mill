//! Socket accept loops and graceful shutdown.
//!
//! One task per port, each accepting into a per-connection hyper service. The
//! [`Gateway`] is shared behind an `Arc` rather than cloned, since the MCP
//! session managers inside it must be the same objects across connections —
//! a session id issued on one connection has to be recognised on the next.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::gateway::Gateway;

/// Bind every address and serve until SIGINT or SIGTERM.
pub async fn run(gateway: Gateway, addrs: Vec<SocketAddr>) -> anyhow::Result<()> {
    let shutdown = CancellationToken::new();
    let serving = run_with_shutdown(gateway, addrs, shutdown.clone()).await?;
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
        tasks.push(tokio::spawn(async move {
            accept_loop(gateway, port, listener, shutdown).await;
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
) {
    loop {
        let stream = tokio::select! {
            _ = shutdown.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _peer)) => stream,
                Err(err) => {
                    // A failed accept is usually per-connection (fd limits,
                    // a client that vanished); tearing down the listener
                    // would turn a transient problem into an outage.
                    tracing::warn!(%port, %err, "accept failed");
                    continue;
                }
            },
        };

        let gateway = Arc::clone(&gateway);
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let gateway = Arc::clone(&gateway);
                async move { gateway.handle(port, request).await }
            });

            let builder = ConnBuilder::new(TokioExecutor::new());
            let connection =
                builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
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
        });
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
