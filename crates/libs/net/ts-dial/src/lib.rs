//! The outbound stream both ts2021 clients open: TCP, wrapped in TLS when the
//! URL says `https://`, behind one boxed type. `ts-derp` and `ts-control` call
//! [`connect`] and keep their protocol code over plain `AsyncRead + AsyncWrite`.

pub mod pem;

use std::io;

use rusty_tls::{AsyncTlsStream, TrustPolicy};
use rusty_tokio::io::{AsyncRead, AsyncWrite};
use rusty_tokio::net::TcpStream;

pub use rusty_tls::TrustPolicy as Trust;

/// What a client can read from and write to.
pub trait Io: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Io for T {}

/// An open connection, TLS or not.
pub type BoxIo = Box<dyn Io>;

/// Splits `http://rest` or `https://rest` into (is TLS, rest). `None` for any
/// other scheme.
pub fn scheme(url: &str) -> Option<(bool, &str)> {
    url.strip_prefix("https://")
        .map(|rest| (true, rest))
        .or_else(|| url.strip_prefix("http://").map(|rest| (false, rest)))
}

/// `host` of a `host`, `host:port` or `[v6]:port` authority: the name TLS
/// checks the certificate against.
pub fn server_name(authority: &str) -> &str {
    if let Some(v6) = authority.strip_prefix('[') {
        return v6.split(']').next().unwrap_or(v6);
    }
    authority
        .rsplit_once(':')
        .map_or(authority, |(host, _)| host)
}

/// A failure to open the connection.
#[derive(Debug, thiserror::Error)]
pub enum DialError {
    #[error("connect: {0}")]
    Io(#[from] io::Error),
    #[error("TLS: {0}")]
    Tls(#[from] rusty_tls::Error),
}

/// Trusts exactly the certificates in `ca_pem` (a private CA, or the hub's own
/// self-signed certificate) and no system anchor. A file with no certificate
/// is an error: it would trust nothing.
pub fn trust_from_pem(ca_pem: &str) -> Result<Trust, pem::PemError> {
    let roots: Vec<_> = pem::certificates(ca_pem)?
        .into_iter()
        .map(Into::into)
        .collect();
    if roots.is_empty() {
        return Err(pem::PemError {
            file: "certificate",
            reason: "no CERTIFICATE block",
        });
    }
    Ok(Trust::PinnedAnchors(roots))
}

/// Opens `host_port` over TCP; with `tls` it also starts a TLS session to
/// `server_name` trusting `policy`. The TLS handshake runs on the first read
/// or write, so a refused certificate surfaces there, inside the caller's own
/// timeout.
pub async fn connect(
    host_port: &str,
    tls: Option<(&str, &TrustPolicy)>,
) -> Result<BoxIo, DialError> {
    let tcp = TcpStream::connect(host_port).await?;
    tcp.set_nodelay(true).ok();
    match tls {
        None => Ok(Box::new(tcp)),
        Some((name, policy)) => Ok(Box::new(AsyncTlsStream::new(tcp, name, policy)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_picks_tls_and_rejects_the_rest() {
        assert_eq!(scheme("https://h:1/x"), Some((true, "h:1/x")));
        assert_eq!(scheme("http://h:1"), Some((false, "h:1")));
        assert_eq!(scheme("ftp://h"), None);
        assert_eq!(scheme("h:1"), None);
    }

    #[test]
    fn server_name_drops_the_port_and_v6_brackets() {
        assert_eq!(server_name("hub.example:443"), "hub.example");
        assert_eq!(server_name("hub.example"), "hub.example");
        assert_eq!(server_name("[::1]:443"), "::1");
        assert_eq!(server_name("127.0.0.1:80"), "127.0.0.1");
    }

    #[test]
    fn trust_from_pem_needs_a_certificate() {
        let pem = "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n";
        assert!(matches!(trust_from_pem(pem), Ok(Trust::PinnedAnchors(r)) if r.len() == 1));
        assert_eq!(
            trust_from_pem("").unwrap_err().reason,
            "no CERTIFICATE block"
        );
    }

    #[rusty_tokio::test]
    async fn plain_connect_reaches_a_listener() {
        let listener = rusty_tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = rusty_tokio::spawn(async move { listener.accept().await.is_ok() });
        connect(&addr.to_string(), None).await.unwrap();
        assert!(accepted.await.unwrap());
    }
}
