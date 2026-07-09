//! Linux TUN device creation and configuration via ioctls.
//!
//! Pure syscalls (through `libc`) — no `iproute2` dependency. Assigning the
//! tailnet address with a `/10` netmask makes the kernel auto-install the
//! connected route for `100.64.0.0/10`, so no explicit route command is
//! needed (verified: `100.64.0.0/10 proto kernel scope link dev ts0`).

use std::io;
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, OwnedFd};

// ioctl request numbers (Linux, x86_64/aarch64 share these).
const TUNSETIFF: libc::c_ulong = 0x4004_54ca;
const SIOCSIFADDR: libc::c_ulong = 0x8916;
const SIOCSIFNETMASK: libc::c_ulong = 0x891c;
const SIOCSIFMTU: libc::c_ulong = 0x8922;
const SIOCGIFFLAGS: libc::c_ulong = 0x8913;
const SIOCSIFFLAGS: libc::c_ulong = 0x8914;

const IFF_TUN: i16 = 0x0001;
const IFF_NO_PI: i16 = 0x1000u16 as i16;
const IFF_UP: i16 = 0x0001;
const IFF_RUNNING: i16 = 0x0040;

const IFNAMSIZ: usize = 16;

/// Kernel `struct ifreq`: a 16-byte name followed by a 24-byte union.
#[repr(C)]
struct Ifreq {
    name: [u8; IFNAMSIZ],
    data: [u8; 24],
}

impl Ifreq {
    fn new(name: &str) -> io::Result<Self> {
        let bytes = name.as_bytes();
        if bytes.len() >= IFNAMSIZ {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
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

    /// Writes an `AF_INET` sockaddr (family + address) into the union.
    fn set_sockaddr_in(&mut self, addr: Ipv4Addr) {
        self.data = [0u8; 24];
        self.data[0..2].copy_from_slice(&(libc::AF_INET as u16).to_ne_bytes());
        // bytes 2-3 = port (0); bytes 4-7 = address in network order.
        self.data[4..8].copy_from_slice(&addr.octets());
    }
}

/// Creates a TUN device named `name` (IFF_TUN | IFF_NO_PI) and returns its
/// file descriptor.
pub fn create_tun(name: &str) -> io::Result<OwnedFd> {
    use std::fs::OpenOptions;
    use std::os::fd::IntoRawFd;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/net/tun")?;
    let fd = file.as_raw_fd();

    let mut req = Ifreq::new(name)?;
    let flags = IFF_TUN | IFF_NO_PI;
    req.data[0..2].copy_from_slice(&flags.to_ne_bytes());
    // SAFETY: fd is a valid open file; req outlives the call.
    let rc = unsafe { libc::ioctl(fd, TUNSETIFF, &mut req as *mut Ifreq) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }

    // Transfer ownership of the fd out of `file` (which would otherwise close
    // it on drop).
    let raw = file.into_raw_fd();
    // SAFETY: raw is a valid fd we just took sole ownership of.
    Ok(unsafe { std::os::fd::FromRawFd::from_raw_fd(raw) })
}

/// Configures the device: address, netmask (from `prefix_len`), MTU, and
/// brings it up. Requires `CAP_NET_ADMIN`.
pub fn configure(name: &str, addr: Ipv4Addr, prefix_len: u8, mtu: u32) -> io::Result<()> {
    // SAFETY: a datagram socket is only used as an ioctl handle here.
    let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if sock < 0 {
        return Err(io::Error::last_os_error());
    }
    let result = configure_inner(sock, name, addr, prefix_len, mtu);
    // SAFETY: sock is a valid fd we own.
    unsafe { libc::close(sock) };
    result
}

fn configure_inner(
    sock: libc::c_int,
    name: &str,
    addr: Ipv4Addr,
    prefix_len: u8,
    mtu: u32,
) -> io::Result<()> {
    let ioctl_ifreq = |req_num: libc::c_ulong, req: &mut Ifreq| -> io::Result<()> {
        // SAFETY: sock is valid; req outlives the call.
        let rc = unsafe { libc::ioctl(sock, req_num, req as *mut Ifreq) };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    };

    // Address.
    let mut a = Ifreq::new(name)?;
    a.set_sockaddr_in(addr);
    ioctl_ifreq(SIOCSIFADDR, &mut a)?;

    // Netmask from prefix length.
    let mut m = Ifreq::new(name)?;
    m.set_sockaddr_in(netmask(prefix_len));
    ioctl_ifreq(SIOCSIFNETMASK, &mut m)?;

    // MTU.
    let mut mt = Ifreq::new(name)?;
    mt.data[0..4].copy_from_slice(&(mtu as i32).to_ne_bytes());
    ioctl_ifreq(SIOCSIFMTU, &mut mt)?;

    // Bring up: read current flags, OR in UP|RUNNING, write back.
    let mut fl = Ifreq::new(name)?;
    ioctl_ifreq(SIOCGIFFLAGS, &mut fl)?;
    let cur = i16::from_ne_bytes([fl.data[0], fl.data[1]]);
    let up = cur | IFF_UP | IFF_RUNNING;
    fl.data[0..2].copy_from_slice(&up.to_ne_bytes());
    ioctl_ifreq(SIOCSIFFLAGS, &mut fl)?;

    Ok(())
}

/// Converts a prefix length (e.g. 10) to a dotted netmask (255.192.0.0).
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
        assert_eq!(netmask(10), Ipv4Addr::new(255, 192, 0, 0));
        assert_eq!(netmask(32), Ipv4Addr::new(255, 255, 255, 255));
        assert_eq!(netmask(0), Ipv4Addr::new(0, 0, 0, 0));
        assert_eq!(netmask(24), Ipv4Addr::new(255, 255, 255, 0));
    }
}
