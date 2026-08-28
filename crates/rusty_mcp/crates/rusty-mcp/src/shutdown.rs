//! Graceful shutdown signalling.

use tokio_util::sync::CancellationToken;

/// Resolves on `SIGINT`, or on `SIGTERM` where Unix signals are available.
///
/// Used to drive graceful shutdown on both transports.
pub async fn signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(%err, "failed to listen for ctrl-c");
            // Never resolve: let the other branch (or the server) decide.
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(err) => {
                tracing::error!(%err, "failed to listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("received ctrl-c, shutting down"),
        () = terminate => tracing::info!("received SIGTERM, shutting down"),
    }
}

/// A [`CancellationToken`] that fires on the first shutdown [`signal`].
///
/// The watcher task holds only a clone, so it does not keep the process alive.
pub fn token() -> CancellationToken {
    let token = CancellationToken::new();
    tokio::spawn({
        let token = token.clone();
        async move {
            signal().await;
            token.cancel();
        }
    });
    token
}
