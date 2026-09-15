//! In-memory TCP implementation (RFC v2 R5+, D16). No real sockets: a
//! process-global registry of listening addresses, and a duplex byte
//! channel per accepted connection — the same "real behavior, no OS
//! calls" contract [`crate::MockDir`] has for the filesystem.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::net::{Net, TcpListener, TcpStream, UdpSocket, UnixListener, UnixStream};

type Registry = Mutex<HashMap<SocketAddr, Sender<(MockTcpStream, SocketAddr)>>>;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The `AF_UNIX` counterpart of [`registry`]: a process-global map from
/// bound path to the channel `accept` blocks on. Kept as a separate map
/// (rather than folding into one keyed by some enum) for the same
/// reason `Net`'s two `*_listen` methods are separate — TCP and Unix
/// addresses don't share a namespace in real backends either.
type UnixRegistry = Mutex<HashMap<PathBuf, Sender<(MockUnixStream, Option<PathBuf>)>>>;

fn unix_registry() -> &'static UnixRegistry {
    static REGISTRY: OnceLock<UnixRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The UDP counterpart of [`registry`]: a process-global map from bound
/// address to the channel `recv_from` reads from. A third, separate map
/// (not sharing [`registry`]'s TCP one) for the same reason `unix_registry`
/// is separate — a real OS keeps `SOCK_STREAM` and `SOCK_DGRAM` sockets
/// in independent bind spaces too; binding UDP `:9`, say, never collides
/// with a TCP listener already on `:9`.
type UdpRegistry = Mutex<HashMap<SocketAddr, Sender<(Vec<u8>, SocketAddr)>>>;

fn udp_registry() -> &'static UdpRegistry {
    static REGISTRY: OnceLock<UdpRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Ephemeral port allocator for `tcp_listen(.., port: 0)` and every
/// `tcp_connect`'s synthesized client address — mirrors a real OS's
/// ephemeral range existing, without claiming to match its actual
/// bounds (no consumer needs that precision from the mock).
fn next_ephemeral_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(40000);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn err(kind: ErrorKind, op: &'static str) -> PlatformError {
    PlatformError::new(kind, OsCode::None, op)
}

/// The mock backend's [`Net`] capability.
pub struct MockNet;

impl Net for MockNet {
    fn tcp_connect(&self, addr: SocketAddr) -> Result<Box<dyn TcpStream>> {
        let listener_tx = {
            let reg = crate::sync::lock(registry());
            reg.get(&addr)
                .cloned()
                .ok_or_else(|| err(ErrorKind::ConnectionRefused, "connect"))?
        };
        let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), next_ephemeral_port());
        let (c2s_tx, c2s_rx) = mpsc::channel();
        let (s2c_tx, s2c_rx) = mpsc::channel();
        let server_end = MockTcpStream {
            tx: s2c_tx,
            rx: c2s_rx,
            local: addr,
            peer: client_addr,
            read_buf: Vec::new(),
            read_pos: 0,
            read_timeout: std::cell::Cell::new(None),
        };
        let client_end = MockTcpStream {
            tx: c2s_tx,
            rx: s2c_rx,
            local: client_addr,
            peer: addr,
            read_buf: Vec::new(),
            read_pos: 0,
            read_timeout: std::cell::Cell::new(None),
        };
        // The listener side may have dropped (no one accepting anymore)
        // — the same "nothing there" outcome a real refused connection
        // gives.
        listener_tx
            .send((server_end, client_addr))
            .map_err(|_| err(ErrorKind::ConnectionRefused, "connect"))?;
        Ok(Box::new(client_end))
    }

    fn tcp_listen(&self, addr: SocketAddr) -> Result<Box<dyn TcpListener>> {
        let addr = if addr.port() == 0 {
            SocketAddr::new(addr.ip(), next_ephemeral_port())
        } else {
            addr
        };
        let (tx, rx) = mpsc::channel();
        let mut reg = crate::sync::lock(registry());
        if reg.contains_key(&addr) {
            return Err(err(ErrorKind::AddrInUse, "listen"));
        }
        reg.insert(addr, tx);
        Ok(Box::new(MockTcpListener {
            addr,
            rx: Mutex::new(rx),
        }))
    }

    fn unix_connect(&self, path: &Path) -> Result<Box<dyn UnixStream>> {
        let listener_tx = {
            let reg = crate::sync::lock(unix_registry());
            reg.get(path)
                .cloned()
                .ok_or_else(|| err(ErrorKind::ConnectionRefused, "connect"))?
        };
        let (c2s_tx, c2s_rx) = mpsc::channel();
        let (s2c_tx, s2c_rx) = mpsc::channel();
        let server_end = MockUnixStream {
            tx: s2c_tx,
            rx: c2s_rx,
            // The accepted end is bound to the listener's path; the
            // connecting end below is anonymous, since this trait gives
            // `unix_connect` no way to bind its own client socket first
            // — the same "no address" state a real unnamed `AF_UNIX`
            // client has.
            local: Some(path.to_path_buf()),
            peer: None,
            read_buf: Vec::new(),
            read_pos: 0,
        };
        let client_end = MockUnixStream {
            tx: c2s_tx,
            rx: s2c_rx,
            local: None,
            peer: Some(path.to_path_buf()),
            read_buf: Vec::new(),
            read_pos: 0,
        };
        // The listener side may have dropped (no one accepting anymore)
        // — the same "nothing there" outcome a real refused connection
        // gives.
        listener_tx
            .send((server_end, None))
            .map_err(|_| err(ErrorKind::ConnectionRefused, "connect"))?;
        Ok(Box::new(client_end))
    }

    fn unix_listen(&self, path: &Path) -> Result<Box<dyn UnixListener>> {
        let path = path.to_path_buf();
        let (tx, rx) = mpsc::channel();
        let mut reg = crate::sync::lock(unix_registry());
        if reg.contains_key(&path) {
            return Err(err(ErrorKind::AddrInUse, "listen"));
        }
        reg.insert(path.clone(), tx);
        Ok(Box::new(MockUnixListener {
            path,
            rx: Mutex::new(rx),
        }))
    }

    fn udp_bind(&self, addr: SocketAddr) -> Result<Box<dyn UdpSocket>> {
        let addr = if addr.port() == 0 {
            SocketAddr::new(addr.ip(), next_ephemeral_port())
        } else {
            addr
        };
        let (tx, rx) = mpsc::channel();
        let mut reg = crate::sync::lock(udp_registry());
        if reg.contains_key(&addr) {
            return Err(err(ErrorKind::AddrInUse, "udp_bind"));
        }
        reg.insert(addr, tx);
        Ok(Box::new(MockUdpSocket {
            local: addr,
            rx: Mutex::new(rx),
        }))
    }
}

/// An in-memory duplex TCP stream: chunks written on one end arrive,
/// in order, as `read` results on the other.
pub struct MockTcpStream {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    local: SocketAddr,
    peer: SocketAddr,
    read_buf: Vec<u8>,
    read_pos: usize,
    // `Cell`, not a plain field: `set_read_timeout` takes `&self`, the
    // same "stateless-looking capability actually needs one flag"
    // shape `platform-windows`'s `ensure_wsa_started` Once-guard has,
    // just per-instance here instead of process-global.
    read_timeout: std::cell::Cell<Option<Duration>>,
}

impl TcpStream for MockTcpStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.read_pos >= self.read_buf.len() {
            let received = match self.read_timeout.get() {
                Some(d) => self.rx.recv_timeout(d).map_err(|e| match e {
                    // A real timeout expiring is exactly this
                    // ambiguous in practice too — see
                    // `platform::net::TcpStream::set_read_timeout`'s
                    // doc comment for why this mock doesn't pretend to
                    // resolve what the real backends can't.
                    mpsc::RecvTimeoutError::Timeout => None,
                    mpsc::RecvTimeoutError::Disconnected => Some(()),
                }),
                None => self.rx.recv().map_err(|_| Some(())),
            };
            match received {
                Ok(chunk) => {
                    self.read_buf = chunk;
                    self.read_pos = 0;
                }
                // The peer dropped its stream: end of stream, like a
                // real closed socket's read returning 0.
                Err(Some(())) => return Ok(0),
                // The read timeout expired before any data arrived.
                Err(None) => return Err(err(ErrorKind::WouldBlock, "read")),
            }
        }
        let n = buf.len().min(self.read_buf.len() - self.read_pos);
        buf[..n].copy_from_slice(&self.read_buf[self.read_pos..self.read_pos + n]);
        self.read_pos += n;
        Ok(n)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.tx
            .send(buf.to_vec())
            .map_err(|_| err(ErrorKind::BrokenPipe, "write"))?;
        Ok(buf.len())
    }

    fn set_nodelay(&self, _nodelay: bool) -> Result<()> {
        // No buffering to disable in an in-memory channel.
        Ok(())
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        Ok(self.peer)
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local)
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        self.read_timeout.set(timeout);
        Ok(())
    }
}

/// An in-memory listening "socket": a registry entry plus the channel
/// `accept` blocks on.
pub struct MockTcpListener {
    addr: SocketAddr,
    rx: Mutex<Receiver<(MockTcpStream, SocketAddr)>>,
}

impl TcpListener for MockTcpListener {
    fn accept(&self) -> Result<(Box<dyn TcpStream>, SocketAddr)> {
        let rx = crate::sync::lock(&self.rx);
        match rx.recv() {
            Ok((stream, peer)) => Ok((Box::new(stream), peer)),
            Err(_) => Err(err(ErrorKind::Other, "accept")),
        }
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.addr)
    }
}

impl Drop for MockTcpListener {
    fn drop(&mut self) {
        // Free the address so a later `tcp_listen` on the same addr in
        // the same test process doesn't spuriously see AddrInUse.
        crate::sync::lock(registry()).remove(&self.addr);
    }
}

/// An in-memory duplex Unix domain stream: chunks written on one end
/// arrive, in order, as `read` results on the other. Same shape as
/// [`MockTcpStream`], with `Option<PathBuf>` addresses in place of
/// `SocketAddr` (mirroring [`UnixStream`]'s trait signatures) and no
/// `set_nodelay` — there is no Nagle buffering to disable in either the
/// real `AF_UNIX` case or this in-memory one.
pub struct MockUnixStream {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    local: Option<PathBuf>,
    peer: Option<PathBuf>,
    read_buf: Vec<u8>,
    read_pos: usize,
}

impl UnixStream for MockUnixStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.read_pos >= self.read_buf.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.read_buf = chunk;
                    self.read_pos = 0;
                }
                // The peer dropped its stream: end of stream, like a
                // real closed socket's read returning 0.
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.read_buf.len() - self.read_pos);
        buf[..n].copy_from_slice(&self.read_buf[self.read_pos..self.read_pos + n]);
        self.read_pos += n;
        Ok(n)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.tx
            .send(buf.to_vec())
            .map_err(|_| err(ErrorKind::BrokenPipe, "write"))?;
        Ok(buf.len())
    }

    fn peer_addr(&self) -> Result<Option<PathBuf>> {
        Ok(self.peer.clone())
    }

    fn local_addr(&self) -> Result<Option<PathBuf>> {
        Ok(self.local.clone())
    }
}

/// An in-memory listening Unix domain "socket": a registry entry plus
/// the channel `accept` blocks on. No mode bits to narrow and no stale
/// socket *file* to leave behind — this backend has no filesystem
/// backing its registry entries, unlike a real `AF_UNIX` bind, so
/// [`platform::net::Net::unix_listen`]'s mode-0600 narrowing has
/// nothing to do here. The one piece of that contract that *does* carry
/// over is "a path a live listener already holds fails `AddrInUse`",
/// which the `unix_listen` check above enforces the same way
/// `tcp_listen`'s does.
pub struct MockUnixListener {
    path: PathBuf,
    rx: Mutex<Receiver<(MockUnixStream, Option<PathBuf>)>>,
}

impl UnixListener for MockUnixListener {
    fn accept(&self) -> Result<(Box<dyn UnixStream>, Option<PathBuf>)> {
        let rx = crate::sync::lock(&self.rx);
        match rx.recv() {
            Ok((stream, peer)) => Ok((Box::new(stream), peer)),
            Err(_) => Err(err(ErrorKind::Other, "accept")),
        }
    }

    fn local_addr(&self) -> Result<Option<PathBuf>> {
        Ok(Some(self.path.clone()))
    }
}

impl Drop for MockUnixListener {
    fn drop(&mut self) {
        // Free the path so a later `unix_listen` on the same path in
        // the same test process succeeds instead of seeing `AddrInUse`
        // — this backend's in-memory equivalent of the real backends'
        // stale-cleanup bind (`platform::net::Net::unix_listen`'s doc
        // comment). A real dropped listener leaves a socket *file*
        // behind that a later `bind` must actively probe and unlink;
        // this mock has no such file — its registry entry *is* the
        // only trace of the listener ever existing — so plain removal
        // here already reaches the same end state a real backend's
        // probe-then-unlink dance reaches, without needing to fake the
        // probe.
        crate::sync::lock(unix_registry()).remove(&self.path);
    }
}

/// An in-memory UDP datagram socket: a registry entry (its `Sender`
/// half, so others' `send_to` can reach it) plus the channel
/// `recv_from` reads from. Unlike [`MockTcpStream`]/[`MockUnixStream`],
/// there is no peer-specific duplex pair — one socket, addressed per
/// call, matching the real thing's connectionless shape.
pub struct MockUdpSocket {
    local: SocketAddr,
    rx: Mutex<Receiver<(Vec<u8>, SocketAddr)>>,
}

impl UdpSocket for MockUdpSocket {
    fn send_to(&self, buf: &[u8], addr: SocketAddr) -> Result<usize> {
        // Fire-and-forget, matching a real UDP socket: nothing bound at
        // `addr` is not an error (there is no listen/accept handshake
        // to fail), the datagram is just dropped, exactly as it would
        // be by a real OS routing it nowhere.
        if let Some(tx) = crate::sync::lock(udp_registry()).get(&addr) {
            let _ = tx.send((buf.to_vec(), self.local));
        }
        Ok(buf.len())
    }

    fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        let rx = crate::sync::lock(&self.rx);
        match rx.recv() {
            Ok((data, peer)) => {
                // Truncate to `buf`'s length, matching a real
                // `recvfrom(2)`/`WSARecvFrom`'s truncation behavior for
                // an oversized datagram.
                let n = buf.len().min(data.len());
                buf[..n].copy_from_slice(&data[..n]);
                Ok((n, peer))
            }
            Err(_) => Err(err(ErrorKind::Other, "recv_from")),
        }
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local)
    }
}

impl Drop for MockUdpSocket {
    fn drop(&mut self) {
        // Free the address so a later `udp_bind` on the same addr in
        // the same test process doesn't spuriously see `AddrInUse`.
        crate::sync::lock(udp_registry()).remove(&self.local);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_accept_and_echo_round_trip() {
        let net = MockNet;
        let listener = net
            .tcp_listen(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .expect("listen");
        let addr = listener.local_addr().expect("local_addr");
        assert_ne!(addr.port(), 0, "an ephemeral port was assigned");

        let handle = std::thread::spawn(move || {
            let (mut stream, _peer) = listener.accept().expect("accept");
            let mut buf = [0u8; 16];
            let n = stream.read(&mut buf).expect("read");
            assert_eq!(&buf[..n], b"ping");
            stream.write(b"pong").expect("write");
        });

        let mut client = net.tcp_connect(addr).expect("connect");
        assert_eq!(client.peer_addr().unwrap(), addr);
        client
            .set_nodelay(true)
            .expect("set_nodelay is a no-op, not an error");
        client.write(b"ping").expect("write");
        let mut buf = [0u8; 16];
        let n = client.read(&mut buf).expect("read");
        assert_eq!(&buf[..n], b"pong");
        handle.join().expect("server thread");
    }

    #[test]
    fn connect_with_nothing_listening_is_connection_refused() {
        let net = MockNet;
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 59999);
        // `Box<dyn TcpStream>` isn't `Debug`, so `expect_err` (which
        // needs the `Ok` side to be) doesn't fit — `.err().expect(..)`
        // sidesteps that the same way `fs.rs`'s own tests do.
        let e = net.tcp_connect(addr).err().expect("nothing is listening");
        assert_eq!(e.kind, ErrorKind::ConnectionRefused);
    }

    #[test]
    fn listen_twice_on_the_same_addr_is_addr_in_use() {
        let net = MockNet;
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 59998);
        let first = net.tcp_listen(addr).expect("first listen");
        let e = net.tcp_listen(addr).err().expect("already listening");
        assert_eq!(e.kind, ErrorKind::AddrInUse);
        drop(first);
        // Dropping the first listener frees the address for reuse.
        net.tcp_listen(addr).expect("listen again after drop");
    }

    #[test]
    fn closing_the_peer_reports_end_of_stream() {
        let net = MockNet;
        let listener = net
            .tcp_listen(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .expect("listen");
        let addr = listener.local_addr().expect("local_addr");
        let handle = std::thread::spawn(move || {
            let (_stream, _peer) = listener.accept().expect("accept");
            // Dropped immediately: the client's next read must see EOF.
        });
        let mut client = net.tcp_connect(addr).expect("connect");
        handle.join().expect("server thread");
        let mut buf = [0u8; 16];
        // The server thread has already exited and dropped its stream
        // by the time `join` returns, so this read is deterministic,
        // not a race against the drop.
        let n = client.read(&mut buf).expect("read after peer close");
        assert_eq!(n, 0);
    }

    #[test]
    fn unix_connect_accept_and_echo_round_trip() {
        let net = MockNet;
        let path = PathBuf::from("/mock/sock/echo.sock");
        let listener = net.unix_listen(&path).expect("listen");
        assert_eq!(listener.local_addr().unwrap(), Some(path.clone()));

        let handle = std::thread::spawn(move || {
            let (mut stream, peer) = listener.accept().expect("accept");
            // The mock client never binds its own address, matching a
            // real unnamed `AF_UNIX` client.
            assert_eq!(peer, None);
            let mut buf = [0u8; 16];
            let n = stream.read(&mut buf).expect("read");
            assert_eq!(&buf[..n], b"ping");
            stream.write(b"pong").expect("write");
        });

        let mut client = net.unix_connect(&path).expect("connect");
        assert_eq!(client.peer_addr().unwrap(), Some(path.clone()));
        assert_eq!(client.local_addr().unwrap(), None);
        client.write(b"ping").expect("write");
        let mut buf = [0u8; 16];
        let n = client.read(&mut buf).expect("read");
        assert_eq!(&buf[..n], b"pong");
        handle.join().expect("server thread");
    }

    #[test]
    fn unix_connect_with_nothing_listening_is_connection_refused() {
        let net = MockNet;
        let path = PathBuf::from("/mock/sock/nobody-home.sock");
        // Same `Box<dyn UnixStream>`-isn't-`Debug` workaround as the TCP
        // version of this test above.
        let e = net.unix_connect(&path).err().expect("nothing is listening");
        assert_eq!(e.kind, ErrorKind::ConnectionRefused);
    }

    #[test]
    fn unix_listen_twice_on_the_same_path_is_addr_in_use() {
        let net = MockNet;
        let path = PathBuf::from("/mock/sock/dup.sock");
        let first = net.unix_listen(&path).expect("first listen");
        let e = net.unix_listen(&path).err().expect("already listening");
        assert_eq!(e.kind, ErrorKind::AddrInUse);
        drop(first);
        // Dropping the first listener frees the path for reuse (see
        // `MockUnixListener`'s `Drop` doc comment for why this mock
        // differs from a real backend here).
        net.unix_listen(&path).expect("listen again after drop");
    }

    #[test]
    fn unix_closing_the_peer_reports_end_of_stream() {
        let net = MockNet;
        let path = PathBuf::from("/mock/sock/eof.sock");
        let listener = net.unix_listen(&path).expect("listen");
        let handle = std::thread::spawn(move || {
            let (_stream, _peer) = listener.accept().expect("accept");
            // Dropped immediately: the client's next read must see EOF.
        });
        let mut client = net.unix_connect(&path).expect("connect");
        handle.join().expect("server thread");
        let mut buf = [0u8; 16];
        // The server thread has already exited and dropped its stream
        // by the time `join` returns, so this read is deterministic,
        // not a race against the drop.
        let n = client.read(&mut buf).expect("read after peer close");
        assert_eq!(n, 0);
    }

    #[test]
    fn udp_send_to_and_recv_from_round_trip() {
        let net = MockNet;
        let server = net
            .udp_bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .expect("bind");
        let server_addr = server.local_addr().expect("local_addr");
        assert_ne!(server_addr.port(), 0, "an ephemeral port was assigned");

        let client = net
            .udp_bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .expect("bind");
        let client_addr = client.local_addr().expect("local_addr");

        client.send_to(b"ping", server_addr).expect("send_to");
        let mut buf = [0u8; 16];
        let (n, peer) = server.recv_from(&mut buf).expect("recv_from");
        assert_eq!(&buf[..n], b"ping");
        assert_eq!(peer, client_addr);

        server.send_to(b"pong", client_addr).expect("send_to");
        let (n, peer) = client.recv_from(&mut buf).expect("recv_from");
        assert_eq!(&buf[..n], b"pong");
        assert_eq!(peer, server_addr);
    }

    #[test]
    fn udp_send_to_nothing_bound_is_not_an_error() {
        // Fire-and-forget: unlike tcp_connect/unix_connect, there is no
        // handshake to fail — a datagram to an address nothing is bound
        // to is simply dropped, the same as a real UDP socket.
        let net = MockNet;
        let sender = net
            .udp_bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .expect("bind");
        let nobody = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 59997);
        sender
            .send_to(b"into the void", nobody)
            .expect("send_to must not fail just because nothing is bound there");
    }

    #[test]
    fn udp_bind_twice_on_the_same_addr_is_addr_in_use() {
        let net = MockNet;
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 59996);
        let first = net.udp_bind(addr).expect("first bind");
        let e = net.udp_bind(addr).err().expect("already bound");
        assert_eq!(e.kind, ErrorKind::AddrInUse);
        drop(first);
        // Dropping the first socket frees the address for reuse.
        net.udp_bind(addr).expect("bind again after drop");
    }

    #[test]
    fn udp_bind_port_zero_and_tcp_listen_port_zero_can_collide_by_number() {
        // TCP and UDP bind spaces are independent (see `udp_registry`'s
        // doc comment) — binding the same port number on each must not
        // spuriously collide, unlike two `udp_bind`s on the same addr.
        let net = MockNet;
        let tcp = net
            .tcp_listen(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 59995))
            .expect("tcp listen");
        net.udp_bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 59995))
            .expect("udp bind on the same port number must not collide with the tcp listener");
        drop(tcp);
    }
}
