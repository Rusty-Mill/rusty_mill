//! The client side: `connect(url, options)`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rusty_h2::client::builder::Builder;
use rusty_h2::driver::{self, ClientHandle};
use rusty_http::Url;
use rusty_tls::{AsyncTlsStream, TrustPolicy};
use rusty_tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, TcpStream};
use rusty_tokio::task::{AbortHandle, JoinHandle};
use rusty_tokio::time::{sleep, timeout};

use crate::Error;

type Driver = JoinHandle<Result<(), Error>>;

/// A client certificate and its private key, DER-encoded, leaf first.
#[derive(Clone)]
pub struct ClientIdentity {
    /// The certificate chain, leaf first.
    pub cert_chain_der: Vec<Vec<u8>>,
    /// The leaf's private key (PKCS#8, PKCS#1 or SEC1).
    pub key_der: Vec<u8>,
}

/// Pings an idle connection and closes it when a ping goes unanswered.
#[derive(Debug, Clone, Copy)]
pub struct Keepalive {
    /// How long the connection may sit between pings.
    pub interval: Duration,
    /// How long a ping may go unanswered before the connection is closed.
    pub timeout: Duration,
}

/// How [`connect`] reaches the peer. Cloneable, so a caller can reconnect with the same options.
#[derive(Clone)]
pub struct ConnectOptions {
    /// Who to trust for `https` (ignored for `http`). Default: the system store.
    pub trust: TrustPolicy,
    /// A client certificate to present for `https`, for mutual TLS.
    pub identity: Option<ClientIdentity>,
    /// The limit on TCP connect, TLS handshake and HTTP/2 preface together.
    pub connect_timeout: Duration,
    /// Keepalive pings; off by default.
    pub keepalive: Option<Keepalive>,
    /// HTTP/2 settings.
    pub h2: Builder,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        ConnectOptions {
            trust: TrustPolicy::default(),
            identity: None,
            connect_timeout: Duration::from_secs(10),
            keepalive: None,
            h2: Builder::new(),
        }
    }
}

/// Aborts its task when dropped, unless disarmed.
struct AbortOnDrop(Option<AbortHandle>);

impl AbortOnDrop {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

/// An open HTTP/2 client connection. Keep it alive for as long as the
/// connection should be: dropping it stops the keepalive task, and the
/// connection winds down once every [`ClientHandle`] is dropped too.
pub struct Connection {
    client: ClientHandle,
    closed: Driver,
    /// How the connection ended, once it has been awaited.
    outcome: Option<Result<(), Error>>,
    /// Set by the keepalive before it aborts the driver over an
    /// unanswered ping, so the abort reads as that timeout.
    ping_timed_out: Arc<AtomicBool>,
    _keepalive: Option<AbortOnDrop>,
}

impl Connection {
    /// The handle to start requests on; clone it to share the connection.
    pub fn client(&self) -> &ClientHandle {
        &self.client
    }

    /// Waits for the connection to end and says why: `Ok` for a clean
    /// wind-down. Asking again returns the same answer.
    pub async fn closed(&mut self) -> Result<(), &Error> {
        if self.outcome.is_none() {
            self.outcome = Some(finished(&mut self.closed, &self.ping_timed_out).await);
        }
        match &self.outcome {
            Some(Err(e)) => Err(e),
            _ => Ok(()),
        }
    }

    /// Winds the connection down: stops the keepalive, drops this handle and
    /// waits for the calls in flight to finish. After [`Connection::closed`]
    /// it returns that answer.
    pub async fn close(self) -> Result<(), Error> {
        let Connection {
            client,
            mut closed,
            outcome,
            ping_timed_out,
            _keepalive,
        } = self;
        drop(_keepalive);
        drop(client);
        match outcome {
            Some(done) => done,
            None => finished(&mut closed, &ping_timed_out).await,
        }
    }
}

/// The driver's own result; a cancelled driver is a keepalive timeout when
/// the keepalive cancelled it, else [`Error::Aborted`].
async fn finished(driver: &mut Driver, ping_timed_out: &AtomicBool) -> Result<(), Error> {
    match driver.await {
        Ok(done) => done,
        Err(_) if ping_timed_out.load(Ordering::Acquire) => Err(Error::Timeout("keepalive")),
        Err(_) => Err(Error::Aborted),
    }
}

/// Connects to `url` (`http://` is plain h2c with prior knowledge; `https://`
/// is TLS with ALPN `h2`) and spawns the connection's driver. It returns once
/// the server's first SETTINGS have arrived, which shows the peer speaks
/// HTTP/2, all within `connect_timeout`.
pub async fn connect(url: &str, options: ConnectOptions) -> Result<Connection, Error> {
    let url = Url::parse(url).map_err(|e| match e {
        rusty_http::Error::UnsupportedScheme(s) => Error::UnsupportedScheme(s),
        _ => Error::InvalidUrl(url.to_string()),
    })?;
    let ConnectOptions {
        trust,
        identity,
        connect_timeout,
        keepalive,
        h2,
    } = options;
    let (client, closed) = timeout(connect_timeout, establish(&url, trust, identity, h2))
        .await
        .map_err(|_| Error::Timeout("connect"))??;
    let ping_timed_out = Arc::new(AtomicBool::new(false));
    let guard = keepalive.map(|ka| {
        let driver = closed.abort_handle();
        spawn_keepalive(client.clone(), ka, driver, ping_timed_out.clone())
    });
    Ok(Connection {
        client,
        closed,
        outcome: None,
        ping_timed_out,
        _keepalive: guard,
    })
}

/// TCP, then (for `https`) TLS with ALPN `h2`, then the HTTP/2 driver.
async fn establish(
    url: &Url,
    trust: TrustPolicy,
    identity: Option<ClientIdentity>,
    h2: Builder,
) -> Result<(ClientHandle, Driver), Error> {
    let tcp = TcpStream::connect((url.host.as_str(), url.port)).await?;
    tcp.set_nodelay(true)?;
    if url.scheme != "https" {
        return start(tcp, h2).await;
    }
    let alpn = vec![b"h2".to_vec()];
    let mut tls = match identity {
        Some(id) => AsyncTlsStream::new_with_client_identity_and_alpn(
            tcp,
            &url.host,
            &trust,
            id.cert_chain_der,
            id.key_der,
            alpn,
        )?,
        None => AsyncTlsStream::new_with_alpn(tcp, &url.host, &trust, alpn)?,
    };
    // A flush drives the handshake to completion.
    tls.flush().await?;
    if tls.negotiated_alpn_protocol() != Some(&b"h2"[..]) {
        return Err(Error::AlpnNotH2);
    }
    start(tls, h2).await
}

/// Spawns the driver and waits for the server's first SETTINGS, which only an
/// HTTP/2 server sends and which a request needs before it can start. (An
/// acknowledged PING is not enough: a peer can answer PINGs and never send
/// SETTINGS.)
async fn start<T>(io: T, h2: Builder) -> Result<(ClientHandle, Driver), Error>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (client, connection) = driver::client(io, h2);
    let task = rusty_tokio::spawn(async move { connection.await.map_err(Error::from) });
    // Until the peer is ready the driver dies with this scope.
    let mut guard = AbortOnDrop(Some(task.abort_handle()));
    client.ready().await?;
    guard.disarm();
    Ok((client, task))
}

/// Pings every `interval`; a ping unanswered for `timeout` sets
/// `timed_out` and aborts the driver. A ping that fails means the driver
/// has failed, but it may still be flushing its GOAWAY to a peer that
/// stopped reading: it gets one `timeout` to end with its own error, then
/// is aborted (a no-op if it already ended), so teardown stays bounded.
fn spawn_keepalive(
    client: ClientHandle,
    ka: Keepalive,
    driver: AbortHandle,
    timed_out: Arc<AtomicBool>,
) -> AbortOnDrop {
    let task = rusty_tokio::spawn(async move {
        loop {
            sleep(ka.interval).await;
            match timeout(ka.timeout, client.ping()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => {
                    sleep(ka.timeout).await;
                    return driver.abort();
                }
                Err(_) => {
                    timed_out.store(true, Ordering::Release);
                    return driver.abort();
                }
            }
        }
    });
    AbortOnDrop(Some(task.abort_handle()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use rusty_tokio::io::{duplex, DuplexStream, ReadBuf};
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    /// A transport whose writes, flushes and shutdowns stall for good once
    /// `stalled` is set: a peer that stopped reading.
    struct Stall {
        io: DuplexStream,
        stalled: Arc<AtomicBool>,
    }

    impl Stall {
        fn stalled(&self) -> bool {
            self.stalled.load(Ordering::Acquire)
        }
    }

    impl AsyncRead for Stall {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Pin::new(&mut self.io).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for Stall {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.stalled() {
                return Poll::Pending;
            }
            Pin::new(&mut self.io).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            if self.stalled() {
                return Poll::Pending;
            }
            Pin::new(&mut self.io).poll_flush(cx)
        }

        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            if self.stalled() {
                return Poll::Pending;
            }
            Pin::new(&mut self.io).poll_shutdown(cx)
        }
    }

    /// An empty SETTINGS frame: the server's preface.
    const SETTINGS: [u8; 9] = [0, 0, 0, 4, 0, 0, 0, 0, 0];
    /// SETTINGS with a 5-byte payload, a connection error (FRAME_SIZE_ERROR).
    const BAD_SETTINGS: [u8; 14] = [0, 0, 5, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    /// Connects over an in-memory transport with a keepalive, then has the
    /// peer break the connection, stopping its reads first when `stall`.
    /// Returns how the connection ended.
    async fn broken_by_the_peer(stall: bool) -> String {
        let (near, mut far) = duplex(64 * 1024);
        far.write_all(&SETTINGS).await.unwrap();
        let stalled = Arc::new(AtomicBool::new(false));
        let io = Stall {
            io: near,
            stalled: stalled.clone(),
        };
        let (client, closed) = start(io, Builder::new()).await.unwrap();
        stalled.store(stall, Ordering::Release);
        far.write_all(&BAD_SETTINGS).await.unwrap();

        let ping_timed_out = Arc::new(AtomicBool::new(false));
        let ka = Keepalive {
            interval: Duration::from_millis(20),
            timeout: Duration::from_millis(100),
        };
        let guard = spawn_keepalive(
            client.clone(),
            ka,
            closed.abort_handle(),
            ping_timed_out.clone(),
        );
        let mut conn = Connection {
            client,
            closed,
            outcome: None,
            ping_timed_out,
            _keepalive: Some(guard),
        };
        let ended = timeout(Duration::from_secs(5), conn.closed())
            .await
            .expect("the connection ends");
        drop(far);
        format!("{ended:?}")
    }

    #[rusty_tokio::test]
    async fn a_failed_driver_reports_its_own_error() {
        let ended = broken_by_the_peer(false).await;
        assert!(ended.starts_with("Err(H2("), "{ended}");
    }

    #[rusty_tokio::test]
    async fn the_keepalive_ends_a_failed_driver_stuck_on_its_last_flush() {
        // The driver marks the connection closed (so pings fail), then
        // stalls flushing its GOAWAY to a peer that stopped reading.
        assert_eq!(broken_by_the_peer(true).await, "Err(Aborted)");
    }
}
