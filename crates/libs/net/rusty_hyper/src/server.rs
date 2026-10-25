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
use rusty_tokio::time::{sleep, timeout_at, Instant};

use crate::Error;

/// How [`serve`] runs.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// The limit on everything before the first call: the TLS handshake (for
    /// TLS), then the client's HTTP/2 preface and first SETTINGS, all together.
    /// A client that stalls anywhere in it is dropped.
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

/// A running server. Dropping it begins the same graceful shutdown as
/// [`Server::shutdown`] without waiting for it; call `shutdown` to wait.
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

/// Waits for `work`, or for the stop signal: `None` once stopped. `select!`
/// polls its first branch first, so the stop signal goes first: work that is
/// always ready (a full accept backlog, a steady stream of requests) cannot
/// starve a shutdown.
async fn until_stopped<F: Future>(
    work: F,
    stopped: &mut watch::Receiver<bool>,
) -> Option<F::Output> {
    // Level-triggered as well: a receiver cloned after the signal was sent
    // has already "seen" it and would otherwise never be told.
    if *stopped.borrow() {
        return None;
    }
    rusty_tokio::select! {
        _ = stopped.changed() => None,
        w = work => Some(w),
    }
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
        let Some(accepted) = until_stopped(listener.accept(), &mut stopped).await else {
            break;
        };
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
    // One deadline for everything before the first call: the TLS handshake,
    // then the client's HTTP/2 preface and first SETTINGS. A client that
    // stalls anywhere in it is dropped, and its slot is free again.
    let deadline = Instant::now() + options.handshake_timeout;
    match tls {
        None => run(tcp, deadline, options.h2, handler, stopped).await,
        Some(acceptor) => {
            let Ok(Ok(io)) = timeout_at(deadline, handshake(acceptor, tcp)).await else {
                return;
            };
            run(io, deadline, options.h2, handler, stopped).await;
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

/// Runs HTTP/2 on `io` until the connection ends, a request per task. The
/// client must have sent its preface and SETTINGS by `deadline`.
async fn run<T, F, Fut>(
    io: T,
    deadline: Instant,
    h2: Builder,
    handler: F,
    mut stopped: watch::Receiver<bool>,
) where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    F: Fn(Request, Respond) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (mut acceptor, connection) = driver::server(io, h2);
    let driver = rusty_tokio::spawn(connection);
    if !matches!(timeout_at(deadline, acceptor.ready()).await, Ok(Ok(()))) {
        // Stalled or gone before it was ready: end the connection now.
        driver.abort();
        return;
    }
    let mut stopping = false;
    loop {
        let next = if stopping {
            Some(acceptor.accept().await)
        } else {
            until_stopped(acceptor.accept(), &mut stopped).await
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use rusty_tokio::time::timeout;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[rusty_tokio::test]
    async fn the_stop_signal_wins_when_work_is_ready_too() {
        let (stop, mut stopped) = watch::channel(false);
        // Not stopped: ready work is returned.
        assert_eq!(
            until_stopped(std::future::ready(7), &mut stopped).await,
            Some(7)
        );
        // Stopped, and the work is ready as well: the stop is taken, every time.
        stop.send(true).unwrap();
        for _ in 0..20 {
            assert_eq!(
                until_stopped(std::future::ready(7), &mut stopped).await,
                None
            );
        }
        // A receiver cloned after the signal (a connection accepted just
        // before it) is told too, though it never saw the change happen.
        let mut late = stopped.clone();
        assert_eq!(until_stopped(std::future::ready(7), &mut late).await, None);
        // And a server that is gone altogether counts as stopped.
        let (gone, mut orphan) = watch::channel(false);
        drop(gone);
        assert_eq!(
            until_stopped(std::future::ready(7), &mut orphan).await,
            None
        );
    }

    #[rusty_tokio::test]
    async fn connections_queued_before_a_stop_are_never_admitted() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Three clients are already in the backlog, each having said hello.
        let mut clients = Vec::new();
        for _ in 0..3 {
            let c = TcpStream::connect(addr).await.unwrap();
            c.write_all(rusty_h2::CONNECTION_PREFACE).await.unwrap();
            c.write_all(&[0, 0, 0, 4, 0, 0, 0, 0, 0]).await.unwrap();
            clients.push(c);
        }
        let (stop, stopped) = watch::channel(false);
        stop.send(true).unwrap();
        let handled = Arc::new(AtomicUsize::new(0));
        let h = handled.clone();
        let handler = move |_: Request, _: Respond| {
            h.fetch_add(1, Ordering::SeqCst);
            async {}
        };
        timeout(
            Duration::from_secs(5),
            accept_loop(listener, None, ServeOptions::default(), handler, stopped, 8),
        )
        .await
        .expect("a stopped loop ends");
        // Nothing was admitted, so the server never spoke to them: with the
        // listener gone they see the connection end, not a SETTINGS frame.
        for c in clients {
            let mut buf = [0u8; 16];
            let n = timeout(Duration::from_secs(5), c.read(&mut buf))
                .await
                .expect("the connection ends")
                .unwrap_or(0);
            assert_eq!(n, 0, "no HTTP/2 from a server that was stopped");
        }
        assert_eq!(handled.load(Ordering::SeqCst), 0);
    }
}
