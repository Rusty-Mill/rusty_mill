//! The server side: `serve(listener, tls, options, handler)`.

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use rusty_h2::driver::{Request, Respond};
use rusty_h2::server::builder::Builder;
use rusty_tls::TlsAcceptor;
use rusty_tokio::io::TcpListener;
use rusty_tokio::sync::watch;
use rusty_tokio::task::JoinHandle;

/// How [`serve`] runs.
pub struct ServeOptions {
    /// The limit on the TLS handshake and the HTTP/2 preface together.
    pub handshake_timeout: Duration,
    /// The most connections served at once; the next is closed unserved.
    pub max_connections: usize,
    /// HTTP/2 settings.
    pub h2: Builder,
}

impl Default for ServeOptions {
    fn default() -> Self {
        ServeOptions {
            handshake_timeout: Duration::from_secs(10),
            max_connections: 1024,
            h2: Builder::new(),
        }
    }
}

/// A running server. Dropping it does not stop it; call [`Server::shutdown`].
pub struct Server {
    local_addr: SocketAddr,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl Server {
    /// The address the listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stops accepting, tells every connection to go away (GOAWAY), and waits
    /// until the calls already in flight have finished.
    pub async fn shutdown(self) {
        todo!()
    }
}

/// Serves HTTP/2 on `listener`, plain h2c when `tls` is `None`, otherwise TLS
/// that must negotiate ALPN `h2`. Each request runs `handler(request, respond)`
/// on its own task.
pub fn serve<F, Fut>(
    listener: TcpListener,
    tls: Option<TlsAcceptor>,
    options: ServeOptions,
    handler: F,
) -> Server
where
    F: Fn(Request, Respond) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let _ = (listener, tls, options, handler);
    todo!()
}
