//! Tun surface (RFC v2 R5+, D14) live verification. Linux-only — the
//! single named consumer (rusty_tail's `ts-tun`) is itself Linux-first,
//! and there is no other backend with real behavior to keep textually
//! identical against (Windows is `Unsupported` outright).
//!
//! `#![cfg(target_os = "linux")]`: integration test files don't inherit
//! the library crate's own `#![cfg(target_os = "linux")]` — each needs
//! its own gate, or `cross-compile-check`/the Windows CI legs try to
//! build this file too, where `libc`'s TUN-specific items aren't even
//! reachable. The exact mistake `security_sandbox.rs` made and fixed
//! first, repeated for `net_nonblocking.rs` — gate the whole file.
#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::os::fd::AsRawFd;

use platform::error::ErrorKind;
use platform::tun::Tun;
use platform_linux::{LinuxTun, LinuxTunDevice};

/// Skip (not fail) when the environment lacks `CAP_NET_ADMIN` — the same
/// honest-skip discipline `security_sandbox.rs` uses for Landlock's own
/// environment gap, adapted to `Tun::create`'s hard `Err` (there is no
/// `NotEnforced`-style status to check here the way `Sandbox` has one).
/// GitHub Actions' hosted `ubuntu-latest` runners execute test steps as
/// an unprivileged user by default — this was discovered by CI itself
/// failing after this suite passed only against a root-privileged local
/// sandbox, the same "verify, don't assume" lesson the Landlock `ENOSYS`
/// finding taught earlier this project.
macro_rules! tun_or_skip {
    ($result:expr) => {
        match $result {
            Ok(device) => device,
            Err(e) if e.kind == ErrorKind::PermissionDenied => {
                eprintln!("skipping: CAP_NET_ADMIN unavailable in this environment ({e})");
                return;
            }
            Err(e) => panic!("create tun device: {e}"),
        }
    };
}

/// CGNAT space (RFC 6598, 100.64.0.0/10) — Tailscale's own convention
/// for exactly this reason (`ts-tun`'s doc comment): unused, safe to
/// claim in a test environment, never publicly routable.
///
/// Each test gets its own /24 (the third octet), not a shared one:
/// `cargo test` runs these functions on parallel threads within one
/// process by default, and each `create` installs a real, process-wide
/// connected route for its subnet — two tests sharing one subnet race
/// for which interface the kernel actually delivers to, and a blocking
/// `TunDevice::read()` on the "losing" interface then hangs forever
/// waiting for a packet the kernel sent to the other test's device
/// instead. Distinct subnets per test make that race impossible rather
/// than papering over it with `--test-threads=1`.
const TEST_PREFIX: u8 = 24;
const TEST_MTU: u32 = 1280;

/// Internet checksum (RFC 1071) — computed independently here, not by
/// calling anything this crate's own code provides, so the crafted
/// inbound packet in `tun_delivers_a_crafted_inbound_packet_to_the_local_stack`
/// is genuinely valid from the kernel's point of view, not merely
/// "whatever this crate happened to produce."
fn ip_checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i < header.len() {
        let word = u16::from_be_bytes([header[i], header[i + 1]]);
        sum += u32::from(word);
        i += 2;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Builds a minimal (no options) IPv4 + UDP packet: `src`:`src_port` →
/// `dst`:`dst_port`, carrying `payload`. UDP checksum is left `0`
/// (legal for IPv4 — "not computed"), so only the IP header checksum
/// needs deriving.
fn build_udp_packet(
    src: Ipv4Addr,
    src_port: u16,
    dst: Ipv4Addr,
    dst_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let udp_len = 8 + payload.len();
    let total_len = 20 + udp_len;

    let mut pkt = vec![0u8; total_len];
    // IPv4 header.
    pkt[0] = 0x45; // version 4, IHL 5 (20 bytes, no options)
    pkt[1] = 0; // DSCP/ECN
    pkt[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    pkt[4..6].copy_from_slice(&0u16.to_be_bytes()); // identification
    pkt[6..8].copy_from_slice(&0u16.to_be_bytes()); // flags/fragment offset
    pkt[8] = 64; // TTL
    pkt[9] = 17; // protocol: UDP
    pkt[10..12].copy_from_slice(&0u16.to_be_bytes()); // checksum, filled below
    pkt[12..16].copy_from_slice(&src.octets());
    pkt[16..20].copy_from_slice(&dst.octets());
    let csum = ip_checksum(&pkt[0..20]);
    pkt[10..12].copy_from_slice(&csum.to_be_bytes());

    // UDP header.
    pkt[20..22].copy_from_slice(&src_port.to_be_bytes());
    pkt[22..24].copy_from_slice(&dst_port.to_be_bytes());
    pkt[24..26].copy_from_slice(&(udp_len as u16).to_be_bytes());
    pkt[26..28].copy_from_slice(&0u16.to_be_bytes()); // checksum: 0 = not computed

    pkt[28..].copy_from_slice(payload);
    pkt
}

/// Reads back the MTU the kernel actually applied via `/sys/class/net`
/// — independent of this crate's own code, the same "strace/kernel-
/// state, not just Ok()" discipline the Sandbox work established.
fn kernel_reported_mtu(name: &str) -> u32 {
    std::fs::read_to_string(format!("/sys/class/net/{name}/mtu"))
        .expect("read mtu from sysfs")
        .trim()
        .parse()
        .expect("mtu is a number")
}

/// Reads back the assigned IPv4 address via a raw `SIOCGIFADDR` ioctl,
/// issued directly by the test rather than reusing this crate's own
/// `sys::tun` code, so a bug in `configure()`'s own read-back (if it had
/// one) couldn't hide behind asserting against itself.
fn kernel_reported_addr(name: &str) -> Ipv4Addr {
    const SIOCGIFADDR: libc::c_ulong = 0x8915;
    #[repr(C)]
    struct Ifreq {
        name: [u8; 16],
        data: [u8; 24],
    }
    let mut req = Ifreq {
        name: [0u8; 16],
        data: [0u8; 24],
    };
    let bytes = name.as_bytes();
    req.name[..bytes.len()].copy_from_slice(bytes);
    // SAFETY: a throwaway AF_INET/SOCK_DGRAM socket, the documented
    // handle SIOCGIFADDR is issued through; `req` is valid and
    // exclusively borrowed for the call's duration.
    unsafe {
        let sock = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        assert!(sock >= 0, "socket");
        let r = libc::ioctl(sock, SIOCGIFADDR, &mut req as *mut Ifreq);
        assert_eq!(r, 0, "SIOCGIFADDR");
        libc::close(sock);
    }
    let octets: [u8; 4] = req.data[4..8].try_into().unwrap();
    Ipv4Addr::from(octets)
}

#[test]
fn linux_tun_create_configures_a_real_interface() {
    let addr = Ipv4Addr::new(100, 127, 0, 1);
    let tun = LinuxTun;
    let device = tun_or_skip!(tun.create("rstuntest0", addr, TEST_PREFIX, TEST_MTU));

    assert_eq!(device.name(), "rstuntest0");
    assert_eq!(kernel_reported_mtu(device.name()), TEST_MTU);
    assert_eq!(kernel_reported_addr(device.name()), addr);
}

#[test]
fn linux_tun_outbound_packet_is_readable_from_the_device() {
    let addr = Ipv4Addr::new(100, 127, 1, 1);
    let peer = Ipv4Addr::new(100, 127, 1, 2);
    let tun = LinuxTun;
    let device = tun_or_skip!(tun.create("rstuntest1", addr, TEST_PREFIX, TEST_MTU));

    // Assigning the address with a real prefix length auto-installs the
    // connected route for that subnet — no explicit route command
    // needed. A UDP send into that subnet is therefore routed through
    // this device by the kernel itself, not anything this test forces.
    let sender = UdpSocket::bind("0.0.0.0:0").expect("bind sender");
    let dest = SocketAddr::new(IpAddr::V4(peer), 44444);
    sender.send_to(b"outbound payload", dest).expect("send_to");

    // A freshly-up'd interface can carry kernel-spontaneous traffic of
    // its own (IPv6 router solicitation/neighbor discovery, most
    // commonly) ahead of what a caller just sent — a real device gives
    // no guarantee that the very first packet read is the one this
    // test is waiting for. Observed on CI: the first `read()` returned
    // a non-IPv4 packet, not the crafted UDP datagram. Skip anything
    // that doesn't match rather than asserting on read #1
    // unconditionally; bounded so a genuine regression still fails
    // fast instead of hanging.
    let mut buf = [0u8; 256];
    let n = (0..16)
        .find_map(|_| {
            let n = device.read(&mut buf).expect("read from device");
            let is_ours = n >= 28
                && buf[0] >> 4 == 4
                && buf[9] == 17
                && u16::from_be_bytes([buf[22], buf[23]]) == 44444;
            is_ours.then_some(n)
        })
        .expect("our UDP packet never arrived within 16 reads");
    let pkt = &buf[..n];
    assert!(n >= 28, "at least an IPv4 + UDP header");
    assert_eq!(pkt[9], 17, "protocol field is UDP");
    let src = Ipv4Addr::new(pkt[12], pkt[13], pkt[14], pkt[15]);
    let dst = Ipv4Addr::new(pkt[16], pkt[17], pkt[18], pkt[19]);
    assert_eq!(src, addr, "kernel sourced it from our own address");
    assert_eq!(dst, peer);
    let dst_port = u16::from_be_bytes([pkt[22], pkt[23]]);
    assert_eq!(dst_port, 44444);
    assert_eq!(&pkt[28..n], b"outbound payload");
}

#[test]
fn linux_tun_delivers_a_crafted_inbound_packet_to_the_local_stack() {
    let addr = Ipv4Addr::new(100, 127, 2, 1);
    let peer = Ipv4Addr::new(100, 127, 2, 2);
    let tun = LinuxTun;
    let device = tun_or_skip!(tun.create("rstuntest2", addr, TEST_PREFIX, TEST_MTU));

    let listener = UdpSocket::bind(SocketAddr::new(IpAddr::V4(addr), 55555))
        .expect("bind listener on the tun's own address");
    listener
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("set_read_timeout");

    let pkt = build_udp_packet(peer, 9999, addr, 55555, b"inbound payload");
    device.write(&pkt).expect("write crafted packet");

    let mut buf = [0u8; 256];
    let (n, peer_addr) = listener.recv_from(&mut buf).expect("recv_from");
    assert_eq!(&buf[..n], b"inbound payload");
    assert_eq!(peer_addr, SocketAddr::new(IpAddr::V4(peer), 9999));
}

#[test]
fn linux_tun_as_raw_fd_reports_a_real_open_fd() {
    let addr = Ipv4Addr::new(100, 127, 3, 1);
    let device = tun_or_skip!(LinuxTunDevice::create(
        "rstuntest3",
        addr,
        TEST_PREFIX,
        TEST_MTU
    ));
    let raw = device.as_raw_fd();
    assert!(raw >= 0);

    // Independent proof it's genuinely open: fstat via a raw libc call,
    // not this crate's own code.
    // SAFETY: all-zeroes is a valid (if meaningless) `stat`.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `raw` is a valid fd owned by `device`, alive for this call;
    // `st` is a valid, exclusively borrowed out-param the kernel fills.
    let rc = unsafe { libc::fstat(raw, &mut st) };
    assert_eq!(rc, 0, "fstat on the reported fd must succeed");
}
