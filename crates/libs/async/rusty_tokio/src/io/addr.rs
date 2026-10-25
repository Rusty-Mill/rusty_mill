//! [`ToSocketAddrs`]: what [`TcpStream::connect`](super::TcpStream::connect),
//! [`TcpListener::bind`](super::TcpListener::bind),
//! [`UdpSocket::bind`](super::UdpSocket::bind),
//! [`UdpSocket::connect`](super::UdpSocket::connect) and
//! [`UdpSocket::send_to`](super::UdpSocket::send_to) accept -- not just a
//! concrete [`SocketAddr`], but a `"host:port"` string, an `(&str, u16)`
//! pair, a reference to any of these, or anything else covered below,
//! resolved via the existing [`lookup_host`] (itself a
//! [`crate::spawn_blocking`] round trip -- DNS resolution is genuinely
//! blocking, no portable non-blocking `getaddrinfo` exists) for whichever
//! forms actually need it. An IP literal skips the lookup. Mirrors
//! tokio's own `net::ToSocketAddrs`, and the methods above match tokio's
//! signatures: they are `async fn`s generic over this trait, because
//! resolving a hostname has to `.await` (#897 follow-up; they used to be
//! synchronous and `SocketAddr`-only, with `bind_addrs`/`connect_addrs`
//! alongside).
//!
//! Sealed (an implementation detail of exactly which forms are
//! supported, not a trait meant for downstream `impl`s) for the same
//! reason real tokio's own is: its associated `Future`'s exact
//! (unnameable) type is stabilization-pending, the same
//! `AsyncReadExt::read`-style reasoning [`super::async_io`]'s own module
//! docs give for using `-> impl Future + Send` rather than a plain
//! `async fn` here too.

use super::lookup_host;
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::vec;

mod sealed {
    use super::SocketAddr;
    use std::future::Future;
    use std::io;

    pub trait ToSocketAddrsSealed {
        type Iter: Iterator<Item = SocketAddr> + Send + 'static;

        fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send;
    }
}

/// Resolves `addr` and returns the first candidate `attempt` accepts,
/// trying them in order -- what tokio's `bind`/`connect` do with a form
/// that resolves to more than one address. The last error wins if every
/// candidate fails.
pub(crate) async fn first_ok<T>(
    addr: impl ToSocketAddrs,
    mut attempt: impl FnMut(SocketAddr) -> io::Result<T>,
) -> io::Result<T> {
    let mut last_err = None;
    for addr in addr.to_socket_addrs().await? {
        match attempt(addr) {
            Ok(value) => return Ok(value),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "could not resolve to any address",
        )
    }))
}

/// See the module docs.
pub trait ToSocketAddrs: sealed::ToSocketAddrsSealed {}

impl<T: sealed::ToSocketAddrsSealed + ?Sized> ToSocketAddrs for T {}

/// Any reference to a supported form -- `&str` (the overwhelmingly
/// common case, `TcpStream::connect("example.com:443")`), `&String`,
/// `&SocketAddr`, `&[SocketAddr]` -- defers to the referent's own impl,
/// as tokio's blanket `impl ToSocketAddrs for &T` does.
impl<T: sealed::ToSocketAddrsSealed + ?Sized + Sync> sealed::ToSocketAddrsSealed for &T {
    type Iter = T::Iter;

    fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send {
        (**self).to_socket_addrs()
    }
}

/// A single already-known address, wrapped to implement `Iterator` --
/// backs every impl below that doesn't need [`lookup_host`] at all
/// (nothing to resolve -- it's already a concrete address).
#[doc(hidden)]
pub struct Once(std::option::IntoIter<SocketAddr>);

impl Iterator for Once {
    type Item = SocketAddr;

    fn next(&mut self) -> Option<SocketAddr> {
        self.0.next()
    }
}

fn once(addr: SocketAddr) -> Once {
    Once(Some(addr).into_iter())
}

impl sealed::ToSocketAddrsSealed for SocketAddr {
    type Iter = Once;

    fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send {
        std::future::ready(Ok(once(*self)))
    }
}

impl sealed::ToSocketAddrsSealed for SocketAddrV4 {
    type Iter = Once;

    fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send {
        std::future::ready(Ok(once(SocketAddr::V4(*self))))
    }
}

impl sealed::ToSocketAddrsSealed for SocketAddrV6 {
    type Iter = Once;

    fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send {
        std::future::ready(Ok(once(SocketAddr::V6(*self))))
    }
}

impl sealed::ToSocketAddrsSealed for (IpAddr, u16) {
    type Iter = Once;

    fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send {
        std::future::ready(Ok(once(SocketAddr::new(self.0, self.1))))
    }
}

impl sealed::ToSocketAddrsSealed for (Ipv4Addr, u16) {
    type Iter = Once;

    fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send {
        std::future::ready(Ok(once(SocketAddr::from((self.0, self.1)))))
    }
}

impl sealed::ToSocketAddrsSealed for (Ipv6Addr, u16) {
    type Iter = Once;

    fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send {
        std::future::ready(Ok(once(SocketAddr::from((self.0, self.1)))))
    }
}

impl sealed::ToSocketAddrsSealed for [SocketAddr] {
    type Iter = vec::IntoIter<SocketAddr>;

    fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send {
        let addrs = self.to_vec();
        std::future::ready(Ok(addrs.into_iter()))
    }
}

impl sealed::ToSocketAddrsSealed for str {
    type Iter = super::LookupHost;

    fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send {
        // The fast path real tokio's own `str` impl takes too: a
        // `"host:port"` string that already parses directly as a
        // `SocketAddr` (the overwhelmingly common case -- an IP literal
        // rather than a real hostname) needs no DNS lookup at all, so
        // skip the `spawn_blocking` round trip entirely rather than pay
        // for a needless thread hop.
        let fast = self.parse::<SocketAddr>().ok();
        let owned = self.to_owned();
        async move {
            if let Some(addr) = fast {
                return Ok(super::LookupHost::single(addr));
            }
            lookup_host(owned).await
        }
    }
}

impl sealed::ToSocketAddrsSealed for String {
    type Iter = super::LookupHost;

    fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send {
        <str as sealed::ToSocketAddrsSealed>::to_socket_addrs(self.as_str())
    }
}

impl sealed::ToSocketAddrsSealed for (&str, u16) {
    type Iter = super::LookupHost;

    fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send {
        // Same fast path as the plain `str` impl, but the host and port
        // arrive already split apart rather than needing their own
        // `"host:port"` parse first.
        let fast = self
            .0
            .parse::<IpAddr>()
            .ok()
            .map(|ip| SocketAddr::new(ip, self.1));
        let owned = (self.0.to_owned(), self.1);
        async move {
            if let Some(addr) = fast {
                return Ok(super::LookupHost::single(addr));
            }
            lookup_host(owned).await
        }
    }
}

impl sealed::ToSocketAddrsSealed for (String, u16) {
    type Iter = super::LookupHost;

    fn to_socket_addrs(&self) -> impl Future<Output = io::Result<Self::Iter>> + Send {
        // Same fast path as `(&str, u16)` -- not delegated to it
        // directly, since that would borrow through a `(&str, u16)`
        // temporary that doesn't outlive the returned future.
        let fast = self
            .0
            .parse::<IpAddr>()
            .ok()
            .map(|ip| SocketAddr::new(ip, self.1));
        let owned = self.clone();
        async move {
            if let Some(addr) = fast {
                return Ok(super::LookupHost::single(addr));
            }
            lookup_host(owned).await
        }
    }
}
