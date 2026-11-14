//! Reaching a server over TLS (#506, rusty_term D236 and D237): a local
//! Unix socket that relays each connection to the server's TLS listener, so
//! the client then connects to it as it would to a local server, as with the
//! SSH forward ([`crate::ssh`]).
//!
//! The server is trusted by the SHA-256 fingerprint of its own certificate,
//! pinned when the host was saved; a certificate that differs is refused, with
//! both fingerprints in the error, and nothing re-pins by itself. Each relayed
//! connection starts by sending the host's token, 32 raw bytes, before the
//! protocol's Hello. The token is the caller's to keep: a file only its owner
//! can read, never the hosts file.
//!
//! The socket lives in a fresh directory only its owner can enter, which is
//! removed, with the relay stopped, when the [`Relay`] drops.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::Duration;

use rusty_tls::{TlsStream, TrustPolicy, fingerprint_sha256};
use rusty_tokio::io::{AsyncWriteExt, TcpStream as AsyncTcp, UnixListener};
use rusty_tokio::sync::oneshot;
use rusty_tokio::{Builder, LocalOptions};

/// A server's certificate fingerprint.
pub type Pin = [u8; 32];
/// A token's length in bytes.
pub const TOKEN_LEN: usize = 32;
/// How long connecting to the server's address may take.
const PATIENCE: Duration = Duration::from_secs(5);

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `text` as exactly `N` bytes of hex, either case.
pub fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.trim();
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// The address `addr` (`HOST:PORT`) names, and the host part for the TLS
/// server name.
fn resolve(addr: &str) -> Result<(SocketAddr, String), String> {
    let host = addr
        .rsplit_once(':')
        .map(|(host, _)| host.trim_matches(['[', ']']))
        .filter(|host| !host.is_empty())
        .ok_or_else(|| format!("{addr}: not HOST:PORT"))?;
    let found = addr
        .to_socket_addrs()
        .map_err(|e| format!("{addr}: {e}"))?
        .next()
        .ok_or_else(|| format!("{addr}: no address"))?;
    Ok((found, host.to_owned()))
}

/// The fingerprint of the certificate the server at `addr` presents, taken
/// without trusting it: for pinning a host the user has chosen to pin. It
/// completes the handshake and sends nothing, so no token is shown.
pub fn fingerprint(addr: &str) -> Result<Pin, String> {
    let (found, host) = resolve(addr)?;
    let tcp = TcpStream::connect_timeout(&found, PATIENCE).map_err(|e| format!("{addr}: {e}"))?;
    tcp.set_read_timeout(Some(PATIENCE))
        .and_then(|()| tcp.set_write_timeout(Some(PATIENCE)))
        .map_err(|e| e.to_string())?;
    let mut tls = TlsStream::new(tcp, &host, &TrustPolicy::DangerNoVerification)
        .map_err(|e| format!("{addr}: {e}"))?;
    tls.complete_handshake()
        .map_err(|e| format!("{addr}: {e}"))?;
    let der = tls
        .peer_certificate_der()
        .ok_or_else(|| format!("{addr}: the server sent no certificate"))?;
    fingerprint_sha256(der).map_err(|e| e.to_string())
}

/// Why a [`Relay`] did not start.
#[derive(Debug)]
pub enum StartError {
    /// The server presents another certificate than the pinned one: the
    /// caller must stop, not try another route.
    Changed(String),
    /// Anything else: the server is not there, or not usable.
    Failed(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Self::Changed(why) | Self::Failed(why)) = self;
        f.write_str(why)
    }
}

impl From<String> for StartError {
    fn from(why: String) -> Self {
        Self::Failed(why)
    }
}

/// A relay from a local socket to a server's TLS listener.
pub struct Relay {
    socket: PathBuf,
    dir: PathBuf,
    /// Dropping it stops the thread.
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Relay {
    /// Starts relaying to `addr`, which must present the certificate `pin`
    /// names, sending `token` first on each connection. Checks the pin now,
    /// so a changed certificate fails here, as [`StartError::Changed`], with both
    /// fingerprints.
    pub fn start(addr: &str, pin: Pin, token: [u8; TOKEN_LEN]) -> Result<Self, StartError> {
        let seen = fingerprint(addr)?;
        if seen != pin {
            return Err(StartError::Changed(format!(
                "the certificate of {addr} changed: pinned {}, now {}; re-pin only if you trust the change",
                hex(&pin),
                hex(&seen)
            )));
        }
        let (found, host) = resolve(addr)?;
        let dir = crate::ssh::private_dir().map_err(|e| e.to_string())?;
        let socket = dir.join("tls.sock");
        let (stop, stopped) = oneshot::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let bind = socket.clone();
        let thread = std::thread::Builder::new()
            .name("rusty-mux-tls".into())
            .spawn(move || serve(bind, found, host, pin, token, stopped, ready_tx))
            .map_err(|e| e.to_string())?;
        let mut relay = Self {
            socket,
            dir,
            stop: Some(stop),
            thread: Some(thread),
        };
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(relay),
            Ok(Err(e)) => {
                relay.stop();
                Err(e.into())
            }
            Err(_) => {
                relay.stop();
                Err("the TLS relay stopped before it was ready"
                    .to_owned()
                    .into())
            }
        }
    }

    /// The local socket that reaches the server.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    fn stop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            // A panic in the relay has nothing more to say than its message.
            let _ = thread.join();
        }
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            eprintln!("rusty-mux: removing {}: {e}", self.dir.display());
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The relay thread: accepts local connections until `stopped`.
fn serve(
    socket: PathBuf,
    addr: SocketAddr,
    host: String,
    pin: Pin,
    token: [u8; TOKEN_LEN],
    stopped: oneshot::Receiver<()>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) {
    let runtime = match Builder::new_current_thread().build_local(&LocalOptions::default()) {
        Ok(runtime) => runtime,
        Err(e) => return drop(ready.send(Err(e.to_string()))),
    };
    runtime.block_on(async move {
        let listener = match UnixListener::bind(&socket) {
            Ok(listener) => listener,
            Err(e) => return drop(ready.send(Err(format!("{}: {e}", socket.display())))),
        };
        let _ = ready.send(Ok(()));
        let accepting = async {
            loop {
                match listener.accept().await {
                    Ok((local, _)) => {
                        rusty_tokio::task::spawn_local(relay(
                            local,
                            addr,
                            host.clone(),
                            pin,
                            token,
                        ));
                    }
                    Err(e) => {
                        eprintln!("rusty-mux: tls relay: accept: {e}");
                        rusty_tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                }
            }
        };
        rusty_tokio::select! {
            () = accepting => {},
            _ = stopped => {},
        }
    });
}

/// One connection: dial, pin, token, then bytes both ways.
async fn relay(
    mut local: rusty_tokio::io::UnixStream,
    addr: SocketAddr,
    host: String,
    pin: Pin,
    token: [u8; TOKEN_LEN],
) {
    let result = async {
        let tcp = rusty_tokio::time::timeout(PATIENCE, AsyncTcp::connect(addr))
            .await
            .map_err(|_| "connecting timed out".to_owned())?
            .map_err(|e| e.to_string())?;
        let mut tls =
            rusty_tls::AsyncTlsStream::new(tcp, &host, &TrustPolicy::PinnedFingerprints(vec![pin]))
                .map_err(|e| e.to_string())?;
        tls.write_all(&token).await.map_err(|e| e.to_string())?;
        rusty_tokio::io::copy_bidirectional(&mut local, &mut tls)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    .await;
    // A normal close is not worth a line; a refused pin or token is.
    if let Err(e) = result {
        eprintln!("rusty-mux: tls relay: {e}");
    }
}

#[cfg(test)]
mod tests;
