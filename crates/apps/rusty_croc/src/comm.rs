//! Port of `src/comm` — framed TCP messaging.
//!
//! Wire format (byte-compatible with croc v10):
//! `b"croc" || u32-little-endian payload length || payload`.
//!
//! The Go version supports SOCKS5/HTTP proxies via global variables; proxy
//! support is deferred to a later phase (see MIGRATION.md).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::RwLock;
use std::time::Duration;

pub const MAGIC_BYTES: &[u8; 4] = b"croc";

// Process-wide proxy configuration, mirroring comm's package globals
// `Socks5Proxy` / `HttpProxy`. Empty means "no proxy".
static SOCKS5_PROXY: RwLock<String> = RwLock::new(String::new());
static HTTP_PROXY: RwLock<String> = RwLock::new(String::new());

/// Set the SOCKS5 proxy (host:port, optional `socks5://` scheme). Non-local
/// destinations are dialed through it. Mirrors `comm.Socks5Proxy`.
pub fn set_socks5_proxy(addr: &str) {
    *SOCKS5_PROXY.write().unwrap() = addr.to_string();
}

/// Set the HTTP `CONNECT` proxy (host:port, optional `http://` scheme).
/// Mirrors `comm.HttpProxy`.
pub fn set_http_proxy(addr: &str) {
    *HTTP_PROXY.write().unwrap() = addr.to_string();
}

/// Matches Go's `maxReadMessageSize` guard against malformed streams.
pub const MAX_READ_MESSAGE_SIZE: u32 = 64 * 1024 * 1024;

/// Matches Go's long deadline "in case waiting for file".
pub const IDLE_READ_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);

/// Matches Go's `messageBodyReadTimeout` for reading a frame body.
pub const MESSAGE_BODY_READ_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// A framed connection. Mirrors `comm.Comm`.
pub struct Comm {
    stream: TcpStream,
}

impl Comm {
    /// Wrap an accepted/established stream, applying croc's default deadlines.
    pub fn new(stream: TcpStream) -> std::io::Result<Self> {
        stream.set_read_timeout(Some(IDLE_READ_TIMEOUT))?;
        stream.set_write_timeout(Some(IDLE_READ_TIMEOUT))?;
        Ok(Comm { stream })
    }

    /// Dial `address` with an optional connect time limit (default 30 s),
    /// mirroring `comm.NewConnection`. Routes through a configured SOCKS5 or
    /// HTTP proxy unless the destination is a local IP.
    pub fn new_connection(address: &str, time_limit: Option<Duration>) -> std::io::Result<Self> {
        let limit = time_limit.unwrap_or(Duration::from_secs(30));
        let socks5 = SOCKS5_PROXY.read().unwrap().clone();
        let http = HTTP_PROXY.read().unwrap().clone();
        let local = crate::utils::is_local_ip(address);

        if !socks5.is_empty() && !local {
            let stream = dial_socks5(&socks5, address, limit)?;
            return Comm::new(stream);
        }
        if !http.is_empty() && !local {
            let stream = dial_http_connect(&http, address, limit)?;
            return Comm::new(stream);
        }

        let mut last_err = std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("cannot resolve {address}"),
        );
        for addr in address.to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, limit) {
                Ok(stream) => return Comm::new(stream),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    pub fn stream(&self) -> &TcpStream {
        &self.stream
    }

    /// Clone the underlying socket handle (used by the relay to staple rooms).
    pub fn try_clone(&self) -> std::io::Result<Comm> {
        Ok(Comm {
            stream: self.stream.try_clone()?,
        })
    }

    /// Send one frame: magic, little-endian length, payload — in a single write.
    pub fn send(&mut self, payload: &[u8]) -> std::io::Result<()> {
        let mut frame = Vec::with_capacity(8 + payload.len());
        frame.extend_from_slice(MAGIC_BYTES);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload);
        self.stream.write_all(&frame)?;
        self.stream.flush()
    }

    /// Receive one frame, validating magic and size guard.
    pub fn receive(&mut self) -> std::io::Result<Vec<u8>> {
        self.stream.set_read_timeout(Some(IDLE_READ_TIMEOUT))?;
        let mut magic = [0u8; 4];
        self.stream.read_exact(&mut magic)?;
        if &magic != MAGIC_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("initial bytes are not magic: {:02x?}", magic),
            ));
        }
        let mut len_bytes = [0u8; 4];
        self.stream.read_exact(&mut len_bytes)?;
        let n = u32::from_le_bytes(len_bytes);
        if n > MAX_READ_MESSAGE_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("message too large: {n} > {MAX_READ_MESSAGE_SIZE}"),
            ));
        }
        // Shorten the deadline for the body, like the Go implementation.
        self.stream
            .set_read_timeout(Some(MESSAGE_BODY_READ_TIMEOUT))?;
        let mut buf = vec![0u8; n as usize];
        self.stream.read_exact(&mut buf)?;
        Ok(buf)
    }
}

/// Split `host:port`, defaulting the scheme away. Returns `(host, port)`.
fn split_host_port(address: &str) -> std::io::Result<(String, u16)> {
    let address = address
        .strip_prefix("socks5://")
        .or_else(|| address.strip_prefix("http://"))
        .unwrap_or(address);
    let (host, port) = address.rsplit_once(':').ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("no port in {address}"),
        )
    })?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port: u16 = port
        .parse()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad port"))?;
    Ok((host.to_string(), port))
}

fn connect_proxy(proxy: &str, limit: Duration) -> std::io::Result<TcpStream> {
    let (host, port) = split_host_port(proxy)?;
    let mut last_err = std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("cannot resolve proxy {host}"),
    );
    for addr in (host.as_str(), port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, limit) {
            Ok(s) => return Ok(s),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// SOCKS5 CONNECT (RFC 1928), no authentication. The destination host is sent
/// as a domain name so DNS is resolved proxy-side.
fn dial_socks5(proxy: &str, dest: &str, limit: Duration) -> std::io::Result<TcpStream> {
    let (dhost, dport) = split_host_port(dest)?;
    let mut s = connect_proxy(proxy, limit)?;
    // Greeting: version 5, 1 method, "no auth".
    s.write_all(&[0x05, 0x01, 0x00])?;
    let mut resp = [0u8; 2];
    s.read_exact(&mut resp)?;
    if resp != [0x05, 0x00] {
        return Err(std::io::Error::other("socks5: no acceptable auth method"));
    }
    // CONNECT request with a domain-name address.
    let host_bytes = dhost.as_bytes();
    if host_bytes.len() > 255 {
        return Err(std::io::Error::other("socks5: hostname too long"));
    }
    let mut req = vec![0x05, 0x01, 0x00, 0x03, host_bytes.len() as u8];
    req.extend_from_slice(host_bytes);
    req.extend_from_slice(&dport.to_be_bytes());
    s.write_all(&req)?;
    // Reply: ver, rep, rsv, atyp, then bound address we skip over.
    let mut head = [0u8; 4];
    s.read_exact(&mut head)?;
    if head[1] != 0x00 {
        return Err(std::io::Error::other(format!(
            "socks5: connect failed (code {})",
            head[1]
        )));
    }
    let skip = match head[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut len = [0u8; 1];
            s.read_exact(&mut len)?;
            len[0] as usize
        }
        other => return Err(std::io::Error::other(format!("socks5: bad atyp {other}"))),
    };
    let mut buf = vec![0u8; skip + 2]; // address + 2-byte port
    s.read_exact(&mut buf)?;
    Ok(s)
}

/// HTTP `CONNECT` tunnel: send the request, expect a `2xx` status line.
fn dial_http_connect(proxy: &str, dest: &str, limit: Duration) -> std::io::Result<TcpStream> {
    let (dhost, dport) = split_host_port(dest)?;
    let mut s = connect_proxy(proxy, limit)?;
    let req = format!("CONNECT {dhost}:{dport} HTTP/1.1\r\nHost: {dhost}:{dport}\r\n\r\n");
    s.write_all(req.as_bytes())?;
    // Read headers up to the blank line.
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if s.read(&mut byte)? == 0 {
            return Err(std::io::Error::other("http proxy: connection closed"));
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 8192 {
            return Err(std::io::Error::other("http proxy: response too large"));
        }
    }
    let status = String::from_utf8_lossy(&buf);
    let ok = status
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .map(|code| code.starts_with('2'))
        .unwrap_or(false);
    if !ok {
        return Err(std::io::Error::other(format!(
            "http proxy CONNECT failed: {}",
            status.lines().next().unwrap_or("")
        )));
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn frame_round_trip() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut comm = Comm::new(stream).unwrap();
            let got = comm.receive().unwrap();
            comm.send(&got).unwrap();
        });
        let mut comm = Comm::new_connection(&addr.to_string(), None).unwrap();
        comm.send(b"ping frame").unwrap();
        assert_eq!(comm.receive().unwrap(), b"ping frame");
        handle.join().unwrap();
    }

    // Minimal no-auth SOCKS5 server: complete the handshake, then echo the
    // one frame the client sends through the tunnel.
    #[test]
    fn socks5_tunnel_round_trip() {
        // Destination echo server.
        let dest = TcpListener::bind("127.0.0.1:0").unwrap();
        let dest_addr = dest.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = dest.accept().unwrap();
            let mut buf = [0u8; 64];
            let n = s.read(&mut buf).unwrap();
            s.write_all(&buf[..n]).unwrap();
        });

        // SOCKS5 proxy that forwards to the destination.
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut c, _) = proxy.accept().unwrap();
            let mut greet = [0u8; 3];
            c.read_exact(&mut greet).unwrap();
            c.write_all(&[0x05, 0x00]).unwrap();
            let mut head = [0u8; 5];
            c.read_exact(&mut head).unwrap(); // ver,cmd,rsv,atyp,hostlen
            let mut host = vec![0u8; head[4] as usize + 2];
            c.read_exact(&mut host).unwrap();
            // Success reply with a dummy IPv4 bound address.
            c.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .unwrap();
            // Bridge proxy <-> real destination.
            let mut upstream = TcpStream::connect(dest_addr).unwrap();
            let mut cbuf = [0u8; 64];
            let n = c.read(&mut cbuf).unwrap();
            upstream.write_all(&cbuf[..n]).unwrap();
            let n = upstream.read(&mut cbuf).unwrap();
            c.write_all(&cbuf[..n]).unwrap();
        });

        set_socks5_proxy(&proxy_addr.to_string());
        // Use a non-local target so the proxy is actually used. The proxy
        // ignores the requested host and bridges to our echo server.
        let comm = Comm::new_connection("example.com:80", Some(Duration::from_secs(5))).unwrap();
        set_socks5_proxy(""); // reset global for other tests
        comm.stream().write_all(b"proxied hello").unwrap();
        let mut buf = [0u8; 13];
        comm.stream().read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"proxied hello");
    }

    // Exact frame bytes as croc's Go comm.Write would produce them.
    #[test]
    fn frame_layout_matches_go() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            stream.read_to_end(&mut raw).unwrap();
            raw
        });
        let mut comm = Comm::new_connection(&addr.to_string(), None).unwrap();
        comm.send(b"hi").unwrap();
        drop(comm);
        let raw = handle.join().unwrap();
        assert_eq!(raw, b"croc\x02\x00\x00\x00hi");
    }
}
