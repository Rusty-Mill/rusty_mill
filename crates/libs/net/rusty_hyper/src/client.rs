//! The client side: `connect(url, options)`.

use std::time::Duration;

use rusty_h2::client::builder::Builder;
use rusty_h2::driver::ClientHandle;
use rusty_tls::TrustPolicy;
use rusty_tokio::task::{AbortHandle, JoinHandle};

use crate::Error;

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

/// Aborts its task when dropped.
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// An open HTTP/2 client connection. Keep it alive for as long as the
/// connection should be: dropping it stops the keepalive task, and the
/// connection winds down once every [`ClientHandle`] is dropped too.
pub struct Connection {
    client: ClientHandle,
    closed: JoinHandle<Result<(), Error>>,
    _keepalive: Option<AbortOnDrop>,
}

impl Connection {
    /// The handle to start requests on; clone it to share the connection.
    pub fn client(&self) -> &ClientHandle {
        &self.client
    }

    /// Waits for the connection to end and says why: `Ok` for a clean wind-down.
    pub async fn closed(&mut self) -> Result<(), Error> {
        todo!()
    }

    /// Winds the connection down: stops the keepalive, drops this handle and
    /// waits for the calls in flight to finish.
    pub async fn close(self) -> Result<(), Error> {
        todo!()
    }
}

/// Connects to `url` (`http://` is plain h2c with prior knowledge; `https://`
/// is TLS with ALPN `h2`) and spawns the connection's driver.
pub async fn connect(url: &str, options: ConnectOptions) -> Result<Connection, Error> {
    let _ = (url, options);
    todo!()
}
