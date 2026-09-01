//! The userspace TCP/IP stack task: owns the smoltcp interface and sockets,
//! bridges each TCP connection to a tokio [`TcpStream`], and shuttles IP
//! packets to/from the engine.
//!
//! smoltcp is single-threaded (no locks): one task owns all of it. Everything
//! else talks to it through a single [`Request`] channel — control operations
//! (`SetAddr`, `Bind`) and per-connection app→net traffic (`Data`, `Close`)
//! travel the same queue, so there is never a second owner of a socket.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant as StdInstant;

use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp;
use smoltcp::time::{Duration as SmolDuration, Instant as SmolInstant};
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{mpsc, oneshot};

use crate::device::{ChannelDevice, MTU};

/// Per-socket buffer size. Generous enough for HTTP responses without the
/// stack having to fragment application writes across many segments.
const SOCKET_BUFFER: usize = 64 * 1024;
/// How many listen sockets to keep open per bound port (the accept backlog).
const BACKLOG: usize = 4;
/// The tailnet CGNAT prefix length; assigning our address with it makes the
/// whole `100.64.0.0/10` on-link, so peers are reachable with no gateway.
const TAILNET_PREFIX_LEN: u8 = 10;
/// Bound on the net→app queue depth (chunks) before we exert TCP backpressure.
const APP_QUEUE_DEPTH: usize = 16;

/// A message to the stack task.
pub(crate) enum Request {
    /// Assign our tailnet IP (once the netmap arrives).
    SetAddr(Ipv4Addr),
    /// Bind a TCP port; the reply carries the accept channel.
    Bind {
        port: u16,
        reply: oneshot::Sender<mpsc::Receiver<TcpStream>>,
    },
    /// Application bytes to send on connection `id`.
    Data { id: usize, bytes: Vec<u8> },
    /// The application closed connection `id`'s write side.
    Close { id: usize },
}

/// Spawns the stack task and returns the request sender.
pub(crate) fn spawn(
    inbound_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    outbound_tx: mpsc::UnboundedSender<Vec<u8>>,
) -> mpsc::UnboundedSender<Request> {
    let (req_tx, req_rx) = mpsc::unbounded_channel();
    let stack = Stack::new(req_tx.clone(), inbound_rx, req_rx, outbound_tx);
    tokio::spawn(stack.run());
    req_tx
}

struct Conn {
    handle: SocketHandle,
    /// net → app data.
    to_app: mpsc::Sender<Vec<u8>>,
    /// app → net bytes not yet accepted by smoltcp.
    out: Vec<u8>,
    /// Application closed its write half; close once `out` drains.
    closing: bool,
}

struct Listener {
    port: u16,
    /// Sockets currently in LISTEN, awaiting a connection.
    listening: Vec<SocketHandle>,
    accept_tx: mpsc::Sender<TcpStream>,
}

struct Stack {
    iface: Interface,
    device: ChannelDevice,
    sockets: SocketSet<'static>,
    listeners: Vec<Listener>,
    conns: HashMap<usize, Conn>,
    handle_to_id: HashMap<SocketHandle, usize>,
    next_id: usize,
    addr: Option<Ipv4Addr>,
    start: StdInstant,
    /// Cloned into each `TcpStream` so the app can send `Data`/`Close`.
    req_tx: mpsc::UnboundedSender<Request>,
    inbound_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    req_rx: mpsc::UnboundedReceiver<Request>,
}

impl Stack {
    fn new(
        req_tx: mpsc::UnboundedSender<Request>,
        inbound_rx: mpsc::UnboundedReceiver<Vec<u8>>,
        req_rx: mpsc::UnboundedReceiver<Request>,
        outbound_tx: mpsc::UnboundedSender<Vec<u8>>,
    ) -> Self {
        let start = StdInstant::now();
        let mut device = ChannelDevice::new(outbound_tx);
        // Seed smoltcp's ISN/port randomization from the wall clock; TCP
        // sequence unpredictability isn't a security boundary here (WireGuard
        // is), but a varying seed avoids identical ISNs across restarts.
        let mut config = Config::new(HardwareAddress::Ip);
        config.random_seed = seed();
        let iface = Interface::new(config, &mut device, SmolInstant::from_micros(0));
        Stack {
            iface,
            device,
            sockets: SocketSet::new(Vec::new()),
            listeners: Vec::new(),
            conns: HashMap::new(),
            handle_to_id: HashMap::new(),
            next_id: 1,
            addr: None,
            start,
            req_tx,
            inbound_rx,
            req_rx,
        }
    }

    fn now(&self) -> SmolInstant {
        SmolInstant::from_micros(self.start.elapsed().as_micros() as i64)
    }

    async fn run(mut self) {
        loop {
            // Drain any bursts of packets and requests before polling.
            while let Ok(pkt) = self.inbound_rx.try_recv() {
                self.device.push_rx(pkt);
            }
            while let Ok(req) = self.req_rx.try_recv() {
                self.handle_request(req);
            }

            // Poll smoltcp, service sockets (which may queue sends), poll again
            // to flush those sends onto the wire.
            let now = self.now();
            self.iface.poll(now, &mut self.device, &mut self.sockets);
            self.service_sockets();
            self.iface.poll(now, &mut self.device, &mut self.sockets);

            let delay = self.iface.poll_delay(self.now(), &self.sockets);

            tokio::select! {
                pkt = self.inbound_rx.recv() => match pkt {
                    Some(p) => self.device.push_rx(p),
                    None => break, // engine gone
                },
                req = self.req_rx.recv() => match req {
                    Some(r) => self.handle_request(r),
                    None => break, // all senders dropped
                },
                _ = sleep_for(delay), if delay.is_some() => {}
            }
        }
    }

    fn handle_request(&mut self, req: Request) {
        match req {
            Request::SetAddr(ip) => self.set_addr(ip),
            Request::Bind { port, reply } => {
                let rx = self.bind(port);
                let _ = reply.send(rx);
            }
            Request::Data { id, bytes } => {
                if let Some(conn) = self.conns.get_mut(&id) {
                    conn.out.extend_from_slice(&bytes);
                }
            }
            Request::Close { id } => {
                if let Some(conn) = self.conns.get_mut(&id) {
                    conn.closing = true;
                }
            }
        }
    }

    fn set_addr(&mut self, ip: Ipv4Addr) {
        if self.addr == Some(ip) {
            return;
        }
        self.addr = Some(ip);
        let cidr = IpCidr::new(IpAddress::Ipv4(ip), TAILNET_PREFIX_LEN);
        self.iface.update_ip_addrs(|addrs| {
            addrs.clear();
            let _ = addrs.push(cidr);
        });
        tracing::info!(%ip, "ts-net: stack address set");
    }

    fn bind(&mut self, port: u16) -> mpsc::Receiver<TcpStream> {
        let (accept_tx, accept_rx) = mpsc::channel(BACKLOG);
        let mut listening = Vec::with_capacity(BACKLOG);
        for _ in 0..BACKLOG {
            listening.push(self.open_listen_socket(port));
        }
        self.listeners.push(Listener {
            port,
            listening,
            accept_tx,
        });
        tracing::info!(port, "ts-net: listening");
        accept_rx
    }

    /// Creates a fresh TCP socket in LISTEN on `port` and returns its handle.
    fn open_listen_socket(&mut self, port: u16) -> SocketHandle {
        let mut sock = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0u8; SOCKET_BUFFER]),
            tcp::SocketBuffer::new(vec![0u8; SOCKET_BUFFER]),
        );
        // Listen on any local address so it matches our tailnet IP.
        sock.listen(port).expect("fresh socket can listen");
        self.sockets.add(sock)
    }

    /// Moves data between smoltcp sockets and the per-connection channels,
    /// promotes freshly-connected listen sockets, and reaps dead connections.
    fn service_sockets(&mut self) {
        self.accept_new();

        let ids: Vec<usize> = self.conns.keys().copied().collect();
        for id in ids {
            self.service_conn(id);
        }
    }

    /// Promotes any listen socket that has received a connection into a
    /// [`Conn`], hands a [`TcpStream`] to the listener, and replenishes the
    /// backlog.
    fn accept_new(&mut self) {
        for li in 0..self.listeners.len() {
            let port = self.listeners[li].port;
            let handles = self.listeners[li].listening.clone();
            for handle in handles {
                let sock = self.sockets.get::<tcp::Socket>(handle);
                // A socket that left LISTEN has an inbound connection.
                if sock.state() == tcp::State::Listen || sock.state() == tcp::State::Closed {
                    continue;
                }
                let peer = sock
                    .remote_endpoint()
                    .map(|ep| SocketAddr::new(smol_addr(ep.addr), ep.port))
                    .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));

                let id = self.next_id;
                self.next_id += 1;
                let (to_app, from_net) = mpsc::channel(APP_QUEUE_DEPTH);
                self.conns.insert(
                    id,
                    Conn {
                        handle,
                        to_app,
                        out: Vec::new(),
                        closing: false,
                    },
                );
                self.handle_to_id.insert(handle, id);

                let stream = TcpStream {
                    id,
                    net: self.req_tx.clone(),
                    from_net,
                    leftover: Vec::new(),
                    pos: 0,
                    peer,
                    write_closed: false,
                };
                // Replace the claimed listen socket with a fresh one.
                let replacement = self.open_listen_socket(port);
                let listener = &mut self.listeners[li];
                if let Some(slot) = listener.listening.iter_mut().find(|h| **h == handle) {
                    *slot = replacement;
                }
                // Deliver the connection; if the backlog is full or the
                // listener is gone, drop it (the peer will retry / reset).
                if listener.accept_tx.try_send(stream).is_err() {
                    tracing::debug!(port, "ts-net: accept backlog full, dropping connection");
                    self.reap(id);
                }
            }
        }
    }

    fn service_conn(&mut self, id: usize) {
        let Some(conn) = self.conns.get_mut(&id) else {
            return;
        };
        let handle = conn.handle;
        let sock = self.sockets.get_mut::<tcp::Socket>(handle);

        // app → net: push buffered bytes while smoltcp will take them.
        while !conn.out.is_empty() && sock.can_send() {
            match sock.send_slice(&conn.out) {
                Ok(0) => break,
                Ok(n) => {
                    conn.out.drain(..n);
                }
                Err(_) => break,
            }
        }
        // Graceful close once the app has closed and everything is flushed.
        if conn.closing && conn.out.is_empty() && sock.send_queue() == 0 {
            sock.close();
        }

        // net → app: drain smoltcp's receive buffer into the app channel until
        // it fills (then leave the rest to apply TCP backpressure).
        while sock.can_recv() {
            let mut buf = vec![0u8; MTU];
            match sock.recv_slice(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    buf.truncate(n);
                    match conn.to_app.try_send(buf) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => break,
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            // App dropped the read half: tear the connection down.
                            sock.abort();
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }

        // Reap fully-closed connections.
        let done =
            matches!(sock.state(), tcp::State::Closed) || (!sock.is_open() && !sock.can_recv());
        if done {
            self.reap(id);
        }
    }

    /// Removes a connection and frees its socket. Dropping `to_app` signals EOF
    /// to the application's reader.
    fn reap(&mut self, id: usize) {
        if let Some(conn) = self.conns.remove(&id) {
            self.handle_to_id.remove(&conn.handle);
            self.sockets.remove(conn.handle);
        }
    }
}

/// Sleeps for a smoltcp poll delay; `None` (from `poll_delay`) means "no timer
/// pending", handled by the `select!` guard.
async fn sleep_for(delay: Option<SmolDuration>) {
    match delay {
        Some(d) => tokio::time::sleep(std::time::Duration::from_micros(d.total_micros())).await,
        None => std::future::pending().await,
    }
}

fn smol_addr(addr: IpAddress) -> IpAddr {
    match addr {
        IpAddress::Ipv4(v4) => IpAddr::V4(v4),
        #[allow(unreachable_patterns)]
        _ => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
    }
}

/// A wall-clock-derived seed for smoltcp's randomization.
fn seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15)
}

/// A TCP connection accepted on the tailnet, usable as an ordinary async
/// stream. Reads come from the stack's per-connection channel; writes are sent
/// to the stack task as [`Request::Data`].
pub struct TcpStream {
    id: usize,
    net: mpsc::UnboundedSender<Request>,
    from_net: mpsc::Receiver<Vec<u8>>,
    /// Unread bytes from the last chunk.
    leftover: Vec<u8>,
    pos: usize,
    peer: SocketAddr,
    write_closed: bool,
}

impl TcpStream {
    /// The remote peer's tailnet address.
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // Serve leftover bytes first.
        if self.pos < self.leftover.len() {
            let n = (self.leftover.len() - self.pos).min(buf.remaining());
            let start = self.pos;
            buf.put_slice(&self.leftover[start..start + n]);
            self.pos += n;
            return Poll::Ready(Ok(()));
        }
        match self.from_net.poll_recv(cx) {
            Poll::Ready(Some(chunk)) => {
                let n = chunk.len().min(buf.remaining());
                buf.put_slice(&chunk[..n]);
                if n < chunk.len() {
                    self.leftover = chunk;
                    self.pos = n;
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => Poll::Ready(Ok(())), // EOF
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.write_closed {
            return Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe)));
        }
        match self.net.send(Request::Data {
            id: self.id,
            bytes: buf.to_vec(),
        }) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(_) => Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.write_closed {
            self.write_closed = true;
            let _ = self.net.send(Request::Close { id: self.id });
        }
        Poll::Ready(Ok(()))
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        // Ensure the stack closes the socket even if the app forgot to.
        let _ = self.net.send(Request::Close { id: self.id });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;

    /// RFC 1071 Internet checksum.
    fn checksum(data: &[u8]) -> u16 {
        let mut sum: u32 = 0;
        let mut chunks = data.chunks_exact(2);
        for c in &mut chunks {
            sum += u16::from_be_bytes([c[0], c[1]]) as u32;
        }
        if let [last] = chunks.remainder() {
            sum += (*last as u32) << 8;
        }
        while sum >> 16 != 0 {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        !(sum as u16)
    }

    /// Builds an IPv4+TCP segment (no options) with correct checksums.
    #[allow(clippy::too_many_arguments)]
    fn tcp_segment(
        src: Ipv4Addr,
        dst: Ipv4Addr,
        sport: u16,
        dport: u16,
        seq: u32,
        ack: u32,
        flags: u8,
        payload: &[u8],
    ) -> Vec<u8> {
        let tcp_len = 20 + payload.len();
        let total = 20 + tcp_len;
        let mut p = vec![0u8; total];
        // IPv4 header.
        p[0] = 0x45;
        p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        p[8] = 64; // TTL
        p[9] = 6; // TCP
        p[12..16].copy_from_slice(&src.octets());
        p[16..20].copy_from_slice(&dst.octets());
        let ipcsum = checksum(&p[..20]);
        p[10..12].copy_from_slice(&ipcsum.to_be_bytes());
        // TCP header.
        let t = &mut p[20..];
        t[0..2].copy_from_slice(&sport.to_be_bytes());
        t[2..4].copy_from_slice(&dport.to_be_bytes());
        t[4..8].copy_from_slice(&seq.to_be_bytes());
        t[8..12].copy_from_slice(&ack.to_be_bytes());
        t[12] = 5 << 4; // data offset = 5 words
        t[13] = flags;
        t[14..16].copy_from_slice(&0xffffu16.to_be_bytes()); // window
        t[20..].copy_from_slice(payload);
        // TCP checksum over the pseudo-header + segment.
        let mut pseudo = Vec::with_capacity(12 + tcp_len);
        pseudo.extend_from_slice(&src.octets());
        pseudo.extend_from_slice(&dst.octets());
        pseudo.push(0);
        pseudo.push(6);
        pseudo.extend_from_slice(&(tcp_len as u16).to_be_bytes());
        pseudo.extend_from_slice(&p[20..]);
        let tcsum = checksum(&pseudo);
        p[36..38].copy_from_slice(&tcsum.to_be_bytes());
        p
    }

    const SYN: u8 = 0x02;
    const ACK: u8 = 0x10;
    const PSH: u8 = 0x08;

    fn tcp_flags(segment: &[u8]) -> u8 {
        segment[33]
    }

    /// Drives the stack with no engine and no network: inject a TCP handshake
    /// for a bound port and confirm we accept the connection and can read the
    /// bytes the peer sent — the whole userspace stack, exercised hermetically.
    #[tokio::test]
    async fn accepts_a_handshake_and_reads_payload() {
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
        let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let stack = Stack::new(req_tx.clone(), inbound_rx, req_rx, outbound_tx);
        tokio::spawn(stack.run());

        let our = Ipv4Addr::new(100, 64, 0, 1);
        let peer = Ipv4Addr::new(100, 64, 0, 9);
        req_tx.send(Request::SetAddr(our)).unwrap();
        let (reply, rx) = oneshot::channel();
        req_tx.send(Request::Bind { port: 8080, reply }).unwrap();
        let mut accept_rx = rx.await.unwrap();

        // Client SYN → expect SYN-ACK.
        inbound_tx
            .send(tcp_segment(peer, our, 40000, 8080, 100, 0, SYN, &[]))
            .unwrap();
        let synack = tokio::time::timeout(Duration::from_secs(2), outbound_rx.recv())
            .await
            .expect("stack should emit a SYN-ACK")
            .expect("outbound channel open");
        assert_eq!(tcp_flags(&synack), SYN | ACK, "SYN-ACK flags");
        // The SYN-ACK's sequence is the server ISN; ack should be our seq+1.
        let server_isn = u32::from_be_bytes([synack[24], synack[25], synack[26], synack[27]]);

        // Complete the handshake (ACK) and send a data segment.
        inbound_tx
            .send(tcp_segment(
                peer,
                our,
                40000,
                8080,
                101,
                server_isn.wrapping_add(1),
                ACK,
                &[],
            ))
            .unwrap();
        inbound_tx
            .send(tcp_segment(
                peer,
                our,
                40000,
                8080,
                101,
                server_isn.wrapping_add(1),
                ACK | PSH,
                b"hello ts-net",
            ))
            .unwrap();

        // The listener should hand us the accepted connection.
        let mut stream = tokio::time::timeout(Duration::from_secs(2), accept_rx.recv())
            .await
            .expect("accept should complete")
            .expect("a connection");
        assert_eq!(stream.peer_addr().ip(), IpAddr::V4(peer));

        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
            .await
            .expect("read should complete")
            .expect("read ok");
        assert_eq!(&buf[..n], b"hello ts-net");
        // Keep the outbound receiver alive to the end so the stack's device
        // never sees a closed channel mid-handshake.
        drop(outbound_rx);
    }
}
