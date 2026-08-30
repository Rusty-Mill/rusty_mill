//! Session-bus connection: address discovery, `AF_UNIX` connect (both
//! real-path and Linux abstract-namespace addressing — the common case,
//! since a systemd-managed session typically binds the well-known bus at
//! an abstract address, not a plain path), the SASL `EXTERNAL`
//! handshake, and [`Connection::call`] — the one operation rustils#78's
//! Secret Service integration is built on.

#![allow(unsafe_code)]

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

use super::message::{self, Message, TYPE_ERROR, TYPE_METHOD_RETURN};
use super::value::Value;
use crate::ffi::libc_surface as c;
use crate::sys::fdio;

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn kind_of(errno: i32) -> ErrorKind {
    match errno {
        libc::ECONNREFUSED => ErrorKind::ConnectionRefused,
        libc::ENOENT => ErrorKind::NotFound,
        libc::EACCES | libc::EPERM => ErrorKind::PermissionDenied,
        libc::EINVAL => ErrorKind::InvalidInput,
        _ => ErrorKind::Other,
    }
}

fn sock_err(op: &'static str) -> PlatformError {
    let e = errno();
    PlatformError::new(kind_of(e), OsCode::Errno(e), op)
}

fn proto_err(detail: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::InvalidInput, OsCode::None, detail)
}

/// The byte offset of `sockaddr_un::sun_path` — measured the same way
/// `sys::net`'s own copy is (that one is module-private, and this
/// module's need — packing an *abstract*-namespace address too, which
/// `sys::net::to_sockaddr_un` explicitly refuses — is different enough
/// from `sys::net`'s to not share the function directly).
fn sun_path_offset() -> usize {
    // SAFETY: all-zeroes is a valid (if meaningless) `sockaddr_un`;
    // nothing here is read before being written, only field addresses
    // are taken.
    let addr: c::sockaddr_un = unsafe { std::mem::zeroed() };
    let base = std::ptr::addr_of!(addr) as usize;
    let path = std::ptr::addr_of!(addr.sun_path) as usize;
    path - base
}

/// Pack `sun_path_bytes` (already including any leading NUL for the
/// abstract-namespace form) verbatim into a kernel-layout `sockaddr_un`.
fn pack_unix_addr(sun_path_bytes: &[u8]) -> Result<(c::sockaddr_un, c::socklen_t)> {
    // SAFETY: see `sun_path_offset`.
    let mut addr: c::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = c::AF_UNIX as _;
    if sun_path_bytes.len() > addr.sun_path.len() {
        return Err(proto_err("D-Bus unix socket address too long"));
    }
    for (slot, byte) in addr.sun_path.iter_mut().zip(sun_path_bytes.iter()) {
        *slot = *byte as c::c_char;
    }
    let len = sun_path_offset() + sun_path_bytes.len();
    Ok((addr, len as c::socklen_t))
}

/// One `unix:`-transport D-Bus address, parsed from a
/// `key=value,key=value` fragment (the `dbus-send`/libdbus address
/// grammar) into the exact bytes to place in `sockaddr_un::sun_path`.
enum UnixAddr {
    /// A real filesystem path (`unix:path=...`).
    Path(Vec<u8>),
    /// A Linux abstract-namespace name (`unix:abstract=...`) — not a
    /// filesystem path at all; the kernel identifies it by the leading
    /// NUL byte in `sun_path` rather than by any inode.
    Abstract(Vec<u8>),
}

impl UnixAddr {
    fn sun_path_bytes(&self) -> Vec<u8> {
        match self {
            UnixAddr::Path(p) => p.clone(),
            UnixAddr::Abstract(name) => {
                let mut v = vec![0u8];
                v.extend_from_slice(name);
                v
            }
        }
    }
}

/// Parse the `DBUS_SESSION_BUS_ADDRESS`-style address string (possibly
/// several `;`-separated addresses, only `unix:` transports understood —
/// `tcp:`/`launchd:`/etc. are silently skipped, not this client's
/// scope) into every `unix:` candidate found, in order.
fn parse_unix_addresses(addr_str: &str) -> Vec<UnixAddr> {
    let mut out = Vec::new();
    for candidate in addr_str.split(';') {
        let Some(rest) = candidate.strip_prefix("unix:") else {
            continue;
        };
        for kv in rest.split(',') {
            if let Some(path) = kv.strip_prefix("path=") {
                out.push(UnixAddr::Path(path.as_bytes().to_vec()));
            } else if let Some(name) = kv.strip_prefix("abstract=") {
                out.push(UnixAddr::Abstract(name.as_bytes().to_vec()));
            }
        }
    }
    out
}

/// The session bus address: `DBUS_SESSION_BUS_ADDRESS` if set, else the
/// systemd-managed-session default (`unix:path=/run/user/<uid>/bus`) —
/// the well-known fallback every modern systemd-based Linux desktop or
/// server-with-a-user-session uses when nothing overrides it.
fn session_bus_addresses() -> Vec<UnixAddr> {
    if let Ok(env) = std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        let parsed = parse_unix_addresses(&env);
        if !parsed.is_empty() {
            return parsed;
        }
    }
    // SAFETY: `getuid` takes no arguments and has no preconditions.
    let uid = unsafe { c::getuid() };
    vec![UnixAddr::Path(format!("/run/user/{uid}/bus").into_bytes())]
}

fn new_unix_socket() -> Result<OwnedFd> {
    // SAFETY: plain integer arguments, no memory referenced.
    let fd = unsafe { c::socket(c::AF_UNIX, c::SOCK_STREAM | c::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(sock_err("socket"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn connect_one(addr: &UnixAddr) -> Result<OwnedFd> {
    let fd = new_unix_socket()?;
    let (sockaddr, len) = pack_unix_addr(&addr.sun_path_bytes())?;
    // SAFETY: `sockaddr` holds a valid `sockaddr_un` for exactly the
    // first `len` bytes (`pack_unix_addr`'s contract); `fd` is a freshly
    // created, valid socket.
    let r = unsafe {
        c::connect(
            fd.as_raw_fd(),
            (&sockaddr as *const c::sockaddr_un).cast::<c::sockaddr>(),
            len,
        )
    };
    if r < 0 {
        return Err(sock_err("connect"));
    }
    Ok(fd)
}

fn connect_session_bus() -> Result<OwnedFd> {
    let addrs = session_bus_addresses();
    let mut last_err = None;
    for addr in &addrs {
        match connect_one(addr) {
            Ok(fd) => return Ok(fd),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        PlatformError::new(
            ErrorKind::NotFound,
            OsCode::None,
            "no usable D-Bus session bus address found",
        )
    }))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Read from `fd` until `buf` contains a `\r\n`-terminated line, capped
/// at a generous size so a misbehaving peer can't grow this unboundedly.
/// Returns the line (without the `\r\n`) and any bytes read past it —
/// the SASL protocol guarantees the server doesn't send anything more
/// until we send our next line, so this is normally empty, but a
/// correct reader doesn't assume that.
fn read_sasl_line(fd: &OwnedFd) -> Result<(Vec<u8>, Vec<u8>)> {
    const MAX_LINE: usize = 4096;
    let mut buf = Vec::new();
    loop {
        if let Some(pos) = buf.windows(2).position(|w| w == b"\r\n") {
            let line = buf[..pos].to_vec();
            let leftover = buf[pos + 2..].to_vec();
            return Ok((line, leftover));
        }
        if buf.len() >= MAX_LINE {
            return Err(proto_err("SASL line exceeded the maximum accepted length"));
        }
        let mut chunk = [0u8; 256];
        let n = fdio::read(fd, &mut chunk)?;
        if n == 0 {
            return Err(proto_err(
                "D-Bus peer closed the connection during SASL auth",
            ));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn write_all(fd: &OwnedFd, mut bytes: &[u8]) -> Result<()> {
    while !bytes.is_empty() {
        let n = fdio::write(fd, bytes)?;
        if n == 0 {
            return Err(proto_err("write to D-Bus socket accepted zero bytes"));
        }
        bytes = &bytes[n..];
    }
    Ok(())
}

/// `AUTH EXTERNAL` (RFC v2 R5+, D15, rustils#77): the SASL mechanism for
/// a local unix-socket peer — the kernel already knows who's on the
/// other end (`SO_PEERCRED`-verifiable, though this client doesn't need
/// to check it itself; the *server* does the verifying), so the only
/// payload is the caller's own UID, ASCII-decimal then hex-encoded (the
/// SASL wire encoding every `AUTH` mechanism argument uses). Returns any
/// bytes the server sent past the final `\r\n` line — belongs to the
/// start of the binary message stream, not consumed here.
fn sasl_external_handshake(fd: &OwnedFd) -> Result<Vec<u8>> {
    // The SASL protocol requires an initial NUL byte before the first
    // command, present so the OS can associate the connecting
    // credentials with the byte on systems that need an explicit
    // credential-passing write (Linux's `SO_PEERCRED` doesn't require
    // it, but every D-Bus implementation still sends it — required by
    // the spec regardless of platform).
    write_all(fd, &[0])?;

    // SAFETY: `getuid` takes no arguments and has no preconditions.
    let uid = unsafe { c::getuid() };
    let uid_hex = hex_encode(uid.to_string().as_bytes());
    write_all(fd, format!("AUTH EXTERNAL {uid_hex}\r\n").as_bytes())?;

    let (line, leftover) = read_sasl_line(fd)?;
    if !line.starts_with(b"OK ") {
        return Err(proto_err("D-Bus SASL EXTERNAL authentication was rejected"));
    }

    write_all(fd, b"BEGIN\r\n")?;
    Ok(leftover)
}

/// An authenticated connection to the session bus.
pub struct Connection {
    fd: OwnedFd,
    next_serial: u32,
    /// Bytes read from `fd` but not yet consumed into a decoded
    /// [`Message`] — a D-Bus connection is a byte stream, not a
    /// datagram socket, so a single `read` can return a partial message,
    /// more than one message, or (after `BEGIN`) leftover SASL bytes.
    read_buf: Vec<u8>,
    /// This connection's own bus name (`:1.N`-style), assigned by the
    /// mandatory post-auth `Hello` call. Not currently consumed by any
    /// caller, but keeping it (rather than discarding the `Hello` reply)
    /// costs nothing and documents that the call happened.
    #[allow(dead_code)]
    unique_name: String,
}

impl Connection {
    /// Connect to the session bus (`DBUS_SESSION_BUS_ADDRESS`, or the
    /// systemd-session fallback — `session_bus_addresses`'s contract) and
    /// complete the SASL `EXTERNAL` handshake. `Err(ErrorKind::
    /// ConnectionRefused | NotFound)` when no bus is reachable at all —
    /// the caller (rustils#78) is expected to turn that into
    /// `CredentialStoreStatus::Unavailable`, not a hard failure.
    pub fn session() -> Result<Self> {
        let fd = connect_session_bus()?;
        Self::from_fd(fd)
    }

    /// Connect to one explicit `unix:...` address (the same grammar
    /// `DBUS_SESSION_BUS_ADDRESS` uses) rather than discovering the
    /// session bus's own address — the entry point this module's own
    /// integration tests use, so a test-spawned `dbus-daemon` never has
    /// to mutate the process-wide `DBUS_SESSION_BUS_ADDRESS` environment
    /// variable (unsound to do from parallel test threads).
    pub fn connect_to(unix_address: &str) -> Result<Self> {
        let addrs = parse_unix_addresses(unix_address);
        let addr = addrs
            .first()
            .ok_or_else(|| proto_err("no unix: address found in the given D-Bus address string"))?;
        let fd = connect_one(addr)?;
        Self::from_fd(fd)
    }

    fn from_fd(fd: OwnedFd) -> Result<Self> {
        let leftover = sasl_external_handshake(&fd)?;
        let mut conn = Connection {
            fd,
            next_serial: 1,
            read_buf: leftover,
            unique_name: String::new(),
        };
        // D-Bus spec, "Message Bus": the very first message any client
        // sends after authentication must be `Hello`, registering the
        // connection and assigning it a unique bus name — every other
        // call is refused with `AccessDenied` until this happens.
        let reply = conn.call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "Hello",
            "",
            vec![],
        )?;
        conn.unique_name = match reply.body.first() {
            Some(Value::String(name)) => name.clone(),
            _ => return Err(proto_err("Hello did not return the expected unique name")),
        };
        Ok(conn)
    }

    fn read_one_message(&mut self) -> Result<Message> {
        loop {
            match message::decode(&self.read_buf) {
                Ok((msg, consumed)) => {
                    self.read_buf.drain(..consumed);
                    return Ok(msg);
                }
                Err(e) if e.kind == ErrorKind::WouldBlock => {
                    // Not a real error — `decode` just needs more bytes
                    // than `read_buf` currently holds; fall through to
                    // read more from the socket below.
                }
                Err(e) => return Err(e),
            }
            let mut chunk = [0u8; 4096];
            let n = fdio::read(&self.fd, &mut chunk)?;
            if n == 0 {
                return Err(proto_err("D-Bus peer closed the connection"));
            }
            self.read_buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// Call `interface.member` at `path` on `destination`, with `body`
    /// marshaled per `signature`, and block for the matching reply
    /// (`REPLY_SERIAL` equal to this call's own serial) — any signal or
    /// unrelated message read in the meantime is discarded, since this
    /// client doesn't yet subscribe to anything (no `AddMatch` call
    /// exists in this slice). `Err` for an `ERROR` reply, decoded from
    /// its `ERROR_NAME`/first body string if present.
    pub fn call(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        signature: &str,
        body: Vec<Value>,
    ) -> Result<Message> {
        let serial = self.next_serial;
        self.next_serial += 1;
        let msg = Message::method_call(
            serial,
            destination,
            path,
            interface,
            member,
            signature,
            body,
        );
        let bytes = message::encode(&msg)?;
        write_all(&self.fd, &bytes)?;

        loop {
            let reply = self.read_one_message()?;
            if reply.reply_serial != Some(serial) {
                continue;
            }
            return match reply.message_type {
                TYPE_METHOD_RETURN => Ok(reply),
                TYPE_ERROR => {
                    let detail = reply.error_name.unwrap_or_else(|| "unknown".to_string());
                    Err(PlatformError::new(
                        ErrorKind::Other,
                        OsCode::None,
                        "D-Bus call returned an error",
                    )
                    .with_path(detail))
                }
                _ => Err(proto_err("unexpected reply message type to a method call")),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_path_and_abstract_addresses() {
        let addrs = parse_unix_addresses("unix:path=/run/user/1000/bus");
        assert!(matches!(&addrs[0], UnixAddr::Path(p) if p == b"/run/user/1000/bus"));

        let addrs = parse_unix_addresses("unix:abstract=/tmp/dbus-XYZ,guid=abc123");
        assert!(matches!(&addrs[0], UnixAddr::Abstract(n) if n == b"/tmp/dbus-XYZ"));
    }

    #[test]
    fn abstract_sun_path_bytes_start_with_a_nul() {
        let addr = UnixAddr::Abstract(b"foo".to_vec());
        let bytes = addr.sun_path_bytes();
        assert_eq!(bytes[0], 0);
        assert_eq!(&bytes[1..], b"foo");
    }

    #[test]
    fn hex_encode_matches_known_values() {
        assert_eq!(hex_encode(b"1000"), "31303030");
    }
}
