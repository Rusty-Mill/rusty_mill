//! The server side: `serve(listener, tls, options, handler)`.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rusty_h2::driver::{self, Request, Respond};
use rusty_h2::server::builder::Builder;
use rusty_tls::{AsyncTlsServerStream, TlsAcceptor};
use rusty_tokio::io::{AsyncRead, AsyncWrite, TcpListener, TcpStream};
use rusty_tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use rusty_tokio::task::JoinHandle;
use rusty_tokio::time::{sleep, timeout};

use crate::Error;

/// How [`serve`] runs.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// The limit on the TLS handshake or, for plain h2c, on the client's
    /// first bytes. A client that stays silent longer is dropped.
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
        // A send fails only when every connection and the accept loop are
        // already gone, which is what is being asked for.
        let _ = self.stop.send(true);
        let _ = self.task.await;
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
) -> io::Result<Server>
where
    F: Fn(Request, Respond) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let local_addr = listener.local_addr()?;
    let (stop, stopped) = watch::channel(false);
    let max = options.max_connections.min(u32::MAX as usize);
    let accepting = accept_loop(listener, tls, options, handler, stopped, max);
    Ok(Server {
        local_addr,
        stop,
        task: rusty_tokio::spawn(accepting),
    })
}

/// Accepts connections until told to stop, then waits for them all to end.
async fn accept_loop<F, Fut>(
    listener: TcpListener,
    tls: Option<TlsAcceptor>,
    options: ServeOptions,
    handler: F,
    mut stopped: watch::Receiver<bool>,
    max: usize,
) where
    F: Fn(Request, Respond) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let permits = Arc::new(Semaphore::new(max));
    loop {
        let accepted = rusty_tokio::select! {
            a = listener.accept() => Some(a),
            _ = stopped.changed() => None,
        };
        let Some(accepted) = accepted else { break };
        let Ok((tcp, _)) = accepted else {
            // Out of descriptors, say: back off rather than spin.
            sleep(Duration::from_millis(10)).await;
            continue;
        };
        // At the cap the connection is closed unserved.
        let Ok(permit) = permits.try_acquire_owned() else {
            continue;
        };
        rusty_tokio::spawn(connection(
            tcp,
            tls.clone(),
            options.clone(),
            handler.clone(),
            stopped.clone(),
            permit,
        ));
    }
    drop(listener);
    // Every connection holds a permit until it ends.
    let _ = permits.acquire_many(max as u32).await;
}

/// One accepted connection: the handshake, then HTTP/2.
async fn connection<F, Fut>(
    tcp: TcpStream,
    tls: Option<TlsAcceptor>,
    options: ServeOptions,
    handler: F,
    stopped: watch::Receiver<bool>,
    _permit: OwnedSemaphorePermit,
) where
    F: Fn(Request, Respond) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let _ = tcp.set_nodelay(true);
    let limit = options.handshake_timeout;
    match tls {
        None => {
            // Plain: the client must say something within the limit.
            let mut first = [0u8; 1];
            if !matches!(timeout(limit, tcp.peek(&mut first)).await, Ok(Ok(n)) if n > 0) {
                return;
            }
            run(tcp, options.h2, handler, stopped).await;
        }
        Some(acceptor) => {
            let Ok(Ok(io)) = timeout(limit, handshake(acceptor, tcp)).await else {
                return;
            };
            run(io, options.h2, handler, stopped).await;
        }
    }
}

/// The TLS handshake, which must end with ALPN `h2`.
async fn handshake(
    acceptor: TlsAcceptor,
    tcp: TcpStream,
) -> Result<AsyncTlsServerStream<TcpStream>, Error> {
    let mut tls = acceptor.accept_async(tcp)?;
    tls.complete_handshake().await?;
    if tls.negotiated_alpn_protocol() != Some(&b"h2"[..]) {
        return Err(Error::AlpnNotH2);
    }
    Ok(tls)
}

/// Runs HTTP/2 on `io` until the connection ends, a request per task.
async fn run<T, F, Fut>(io: T, h2: Builder, handler: F, mut stopped: watch::Receiver<bool>)
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    F: Fn(Request, Respond) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (mut acceptor, connection) = driver::server(io, h2);
    let driver = rusty_tokio::spawn(connection);
    let mut stopping = false;
    loop {
        let next = if stopping {
            Some(acceptor.accept().await)
        } else {
            rusty_tokio::select! {
                n = acceptor.accept() => Some(n),
                _ = stopped.changed() => None,
            }
        };
        match next {
            // Told to stop (or the server is gone): no new calls, finish those queued.
            None => {
                acceptor.graceful_shutdown();
                stopping = true;
            }
            Some(Some(Ok((request, respond)))) => {
                rusty_tokio::spawn(handler.clone()(request, respond));
            }
            Some(_) => break,
        }
    }
    // The connection ends once the calls in flight have.
    let _ = driver.await;
}
