//! The client side: `connect(url, options)`.

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

/// How [`connect`] reaches the peer.
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
            self.outcome = Some(finished(&mut self.closed).await);
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
            _keepalive,
        } = self;
        drop(_keepalive);
        drop(client);
        match outcome {
            Some(done) => done,
            None => finished(&mut closed).await,
        }
    }
}

async fn finished(driver: &mut Driver) -> Result<(), Error> {
    driver.await.unwrap_or(Err(Error::Aborted))
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
    let guard = keepalive.map(|ka| spawn_keepalive(client.clone(), ka, closed.abort_handle()));
    Ok(Connection {
        client,
        closed,
        outcome: None,
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

/// Pings every `interval`; a ping unanswered for `timeout` aborts the driver.
fn spawn_keepalive(client: ClientHandle, ka: Keepalive, driver: AbortHandle) -> AbortOnDrop {
    let task = rusty_tokio::spawn(async move {
        loop {
            sleep(ka.interval).await;
            match timeout(ka.timeout, client.ping()).await {
                Ok(Ok(())) => {}
                _ => return driver.abort(),
            }
        }
    });
    AbortOnDrop(Some(task.abort_handle()))
}
