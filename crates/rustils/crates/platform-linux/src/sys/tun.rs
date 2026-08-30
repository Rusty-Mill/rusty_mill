//! Raw TUN device creation and configuration (RFC v2 R5+, D14): `open`
//! `/dev/net/tun` + `TUNSETIFF`, then `SIOCSIFADDR`/`SIOCSIFNETMASK`/
//! `SIOCSIFMTU`/`SIOCGIFFLAGS`/`SIOCSIFFLAGS` over a throwaway datagram
//! socket to assign the address, netmask, MTU, and bring the interface
//! up. Mirrors rusty_tail's own `ts-tun/src/sys.rs` exactly (the donor
//! this slice converges) — the same `Ifreq` layout, the same "assigning
//! the tailnet address with a real prefix length makes the kernel
//! auto-install the connected route" reasoning, re-derived against this
//! crate's own error-mapping conventions instead of linked.

#![allow(unsafe_code)]

use std::ffi::CStr;
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

use crate::ffi::libc_surface as c;

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn kind_of(errno: i32) -> ErrorKind {
    match errno {
        libc::ENOENT => ErrorKind::NotFound,
        libc::EACCES | libc::EPERM => ErrorKind::PermissionDenied,
        libc::EINVAL => ErrorKind::InvalidInput,
        libc::ENODEV => ErrorKind::NotFound,
        _ => ErrorKind::Other,
    }
}

fn tun_err(op: &'static str) -> PlatformError {
    let e = errno();
    PlatformError::new(kind_of(e), OsCode::Errno(e), op)
}

const IFNAMSIZ: usize = 16;

/// Kernel `struct ifreq`: a 16-byte interface name followed by a 24-byte
/// union, matching every `ifreq` variant this slice needs (flags for
/// `TUNSETIFF`, a `sockaddr_in` for the `SIOCSIF{ADDR,NETMASK}` pair, a
/// plain `c_int` for `SIOCSIFMTU`). Hand-defined because the generic
/// glibc target's own `libc::ifreq` isn't admitted (see
/// `ffi::libc_surface`'s doc comment) — the same escape-hatch shape
/// every other missing-from-libc item in this crate already uses.
#[repr(C)]
struct Ifreq {
    name: [u8; IFNAMSIZ],
    data: [u8; 24],
}

impl Ifreq {
    fn new(name: &str) -> Result<Self> {
        let bytes = name.as_bytes();
        if bytes.len() >= IFNAMSIZ {
            return Err(PlatformError::new(
                ErrorKind::InvalidInput,
                OsCode::None,
                "interface name too long",
            ));
        }
        let mut n = [0u8; IFNAMSIZ];
        n[..bytes.len()].copy_from_slice(bytes);
        Ok(Ifreq {
            name: n,
            data: [0u8; 24],
        })
    }

    /// Writes an `AF_INET` sockaddr (family + address) into the union —
    /// the layout `SIOCSIFADDR`/`SIOCSIFNETMASK` both expect.
    fn set_sockaddr_in(&mut self, addr: Ipv4Addr) {
        self.data = [0u8; 24];
        self.data[0..2].copy_from_slice(&(libc::AF_INET as u16).to_ne_bytes());
        // bytes 2-3 = port (0, meaningless for this ioctl); bytes 4-7 =
        // the address, in the same in-memory byte order `octets()`
        // already gives (network order).
        self.data[4..8].copy_from_slice(&addr.octets());
    }
}

// Compile-time-checked literal, not a runtime-fallible conversion (this
// workspace's 1.75 MSRV floor predates C-string literals, stabilized in
// 1.77): a `panic!` here can only fire if this literal is ever edited to
// contain an interior NUL, in which case it fires at compile time.
const DEV_NET_TUN: &CStr = match CStr::from_bytes_with_nul(b"/dev/net/tun\0") {
    Ok(s) => s,
    Err(_) => panic!("DEV_NET_TUN literal must not contain an interior NUL"),
};

/// `open("/dev/net/tun")` + `TUNSETIFF` (`IFF_TUN | IFF_NO_PI`), creating
/// (or attaching to) a TUN device named `name` and returning its fd.
pub fn create_tun(name: &str) -> Result<OwnedFd> {
    // SAFETY: `DEV_NET_TUN` is a valid NUL-terminated path outliving the
    // call; `O_RDWR` is a plain integer flag.
    let fd = unsafe { c::open(DEV_NET_TUN.as_ptr(), c::O_RDWR) };
    if fd < 0 {
        return Err(tun_err("open /dev/net/tun"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };

    let mut req = Ifreq::new(name)?;
    let flags = (c::IFF_TUN | c::IFF_NO_PI) as i16;
    req.data[0..2].copy_from_slice(&flags.to_ne_bytes());
    // SAFETY: `owned` is a valid, freshly opened `/dev/net/tun` fd;
    // `req` is a valid, exclusively borrowed `Ifreq` outliving the call.
    let r = unsafe { c::ioctl(owned.as_raw_fd(), c::TUNSETIFF, &mut req as *mut Ifreq) };
    if r < 0 {
        return Err(tun_err("ioctl(TUNSETIFF)"));
    }
    Ok(owned)
}

fn ioctl_socket() -> Result<OwnedFd> {
    // SAFETY: plain integer arguments, no memory referenced. A throwaway
    // `AF_INET`/`SOCK_DGRAM` socket is the documented handle every
    // `SIOCSIF*` interface-configuration ioctl is issued through — it
    // never sends or receives a single byte of its own.
    let fd = unsafe { c::socket(c::AF_INET, c::SOCK_DGRAM, 0) };
    if fd < 0 {
        return Err(tun_err("socket"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Assigns `addr`/`prefix_len` (which auto-installs the connected route
/// for that subnet — no explicit route command needed), sets `mtu`, and
/// brings the interface up (`IFF_UP | IFF_RUNNING`, OR'd onto whatever
/// flags `SIOCGIFFLAGS` reports already set, not a blind overwrite).
/// Requires `CAP_NET_ADMIN`.
pub fn configure(name: &str, addr: Ipv4Addr, prefix_len: u8, mtu: u32) -> Result<()> {
    let sock = ioctl_socket()?;

    let mut a = Ifreq::new(name)?;
    a.set_sockaddr_in(addr);
    ioctl_ifreq(&sock, c::SIOCSIFADDR, &mut a, "SIOCSIFADDR")?;

    let mut m = Ifreq::new(name)?;
    m.set_sockaddr_in(netmask(prefix_len));
    ioctl_ifreq(&sock, c::SIOCSIFNETMASK, &mut m, "SIOCSIFNETMASK")?;

    let mut mt = Ifreq::new(name)?;
    mt.data[0..4].copy_from_slice(&(mtu as i32).to_ne_bytes());
    ioctl_ifreq(&sock, c::SIOCSIFMTU, &mut mt, "SIOCSIFMTU")?;

    let mut fl = Ifreq::new(name)?;
    ioctl_ifreq(&sock, c::SIOCGIFFLAGS, &mut fl, "SIOCGIFFLAGS")?;
    let cur = i16::from_ne_bytes([fl.data[0], fl.data[1]]);
    const IFF_UP: i16 = 0x0001;
    const IFF_RUNNING: i16 = 0x0040;
    let up = cur | IFF_UP | IFF_RUNNING;
    fl.data[0..2].copy_from_slice(&up.to_ne_bytes());
    ioctl_ifreq(&sock, c::SIOCSIFFLAGS, &mut fl, "SIOCSIFFLAGS")?;

    Ok(())
}

fn ioctl_ifreq(
    sock: &OwnedFd,
    req_num: c::c_ulong,
    req: &mut Ifreq,
    op: &'static str,
) -> Result<()> {
    // SAFETY: `sock` is a valid `AF_INET`/`SOCK_DGRAM` socket; `req` is a
    // valid, exclusively borrowed `Ifreq` outliving the call.
    let r = unsafe { c::ioctl(sock.as_raw_fd(), req_num, req as *mut Ifreq) };
    if r < 0 {
        return Err(tun_err(op));
    }
    Ok(())
}

/// Converts a prefix length (e.g. 24) to a dotted netmask
/// (255.255.255.0).
fn netmask(prefix_len: u8) -> Ipv4Addr {
    let bits = prefix_len.min(32);
    let mask: u32 = if bits == 0 {
        0
    } else {
        u32::MAX << (32 - bits)
    };
    Ipv4Addr::from(mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netmask_from_prefix() {
        assert_eq!(netmask(24), Ipv4Addr::new(255, 255, 255, 0));
        assert_eq!(netmask(10), Ipv4Addr::new(255, 192, 0, 0));
        assert_eq!(netmask(32), Ipv4Addr::new(255, 255, 255, 255));
        assert_eq!(netmask(0), Ipv4Addr::new(0, 0, 0, 0));
    }
}
