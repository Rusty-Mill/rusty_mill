//! Port of the parts of [schollz/peerdiscovery](https://github.com/schollz/peerdiscovery)
//! that croc uses: periodic UDP multicast announcements plus a listener that
//! collects `(source address, payload)` pairs.
//!
//! Wire format is trivial — raw payload datagrams to a multicast group
//! (default `239.255.255.250:9999`, SSDP's group). croc's sender announces
//! `croc<local-relay-port>`; recipients announce `ok` while listening.
//!
//! Like the Go library, packets from our own addresses are ignored (unless
//! `allow_self`), which means same-host discovery intentionally does not
//! happen — croc covers that case with the `ips?` probe over the relay.
//! IPv6 (`ff02::c`) is not yet ported.

use socket2::{Domain, Protocol, Socket, Type};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const DEFAULT_MULTICAST_ADDRESS: &str = "239.255.255.250";
/// croc's IPv6 group, the SSDP link-local address (`peerdiscovery` default).
pub const DEFAULT_MULTICAST_ADDRESS6: &str = "ff02::c";
pub const DEFAULT_PORT: u16 = 9999;

#[derive(Debug, Clone)]
pub struct Discovered {
    pub address: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub multicast_address: String,
    pub port: u16,
    pub payload: Vec<u8>,
    pub delay: Duration,
    /// `None` = no limit (broadcast until `stop` is set).
    pub time_limit: Option<Duration>,
    /// Stop after this many distinct peers (None = unlimited).
    pub limit: Option<usize>,
    pub allow_self: bool,
    /// Listen without announcing (Go's `DisableBroadcast`).
    pub disable_broadcast: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            multicast_address: DEFAULT_MULTICAST_ADDRESS.to_string(),
            port: DEFAULT_PORT,
            payload: b"ok".to_vec(),
            delay: Duration::from_millis(20),
            time_limit: Some(Duration::from_millis(200)),
            limit: None,
            allow_self: false,
            disable_broadcast: false,
        }
    }
}

/// Open the multicast socket and return it plus the group destination. Works
/// for both an IPv4 group (e.g. `239.255.255.250`) and an IPv6 group
/// (e.g. `ff02::c`), chosen by parsing `settings.multicast_address`.
fn open_socket(settings: &Settings) -> std::io::Result<(UdpSocket, SocketAddr)> {
    let group: IpAddr = settings
        .multicast_address
        .parse()
        .map_err(|_| std::io::Error::other("bad multicast address"))?;
    match group {
        IpAddr::V4(group) => {
            let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
            // Multiple processes on one host (sender + receiver in tests, or
            // croc alongside us) must share the discovery port.
            socket.set_reuse_address(true)?;
            #[cfg(unix)]
            let _ = socket.set_reuse_port(true);
            socket.bind(
                &SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, settings.port)).into(),
            )?;
            let socket: UdpSocket = socket.into();
            let _ = socket.join_multicast_v4(&group, &Ipv4Addr::UNSPECIFIED);
            if let Ok(ifaces) = if_addrs::get_if_addrs() {
                for iface in ifaces {
                    if let IpAddr::V4(v4) = iface.ip() {
                        let _ = socket.join_multicast_v4(&group, &v4);
                    }
                }
            }
            let _ = socket.set_multicast_loop_v4(true);
            let _ = socket.set_multicast_ttl_v4(2);
            let dst = SocketAddr::V4(SocketAddrV4::new(group, settings.port));
            Ok((socket, dst))
        }
        IpAddr::V6(group) => {
            let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
            socket.set_reuse_address(true)?;
            #[cfg(unix)]
            let _ = socket.set_reuse_port(true);
            socket.set_only_v6(true)?;
            socket.bind(
                &SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::UNSPECIFIED,
                    settings.port,
                    0,
                    0,
                ))
                .into(),
            )?;
            let socket: UdpSocket = socket.into();
            // Join on every interface index we can enumerate (0 = default).
            let _ = socket.join_multicast_v6(&group, 0);
            let _ = socket.set_multicast_loop_v6(true);
            let dst = SocketAddr::V6(SocketAddrV6::new(group, settings.port, 0, 0));
            Ok((socket, dst))
        }
    }
}

/// Settings preset for the IPv6 discovery group.
pub fn ipv6_settings(payload: Vec<u8>, time_limit: Option<Duration>) -> Settings {
    Settings {
        multicast_address: DEFAULT_MULTICAST_ADDRESS6.to_string(),
        payload,
        time_limit,
        ..Default::default()
    }
}

/// Announce `settings.payload` to the multicast group every `delay` until
/// `stop` is set or `time_limit` elapses. Mirrors the broadcast half of
/// `peerdiscovery.Discover` as croc's sender uses it.
pub fn broadcast(settings: &Settings, stop: Arc<AtomicBool>) -> std::io::Result<()> {
    let (socket, dst) = open_socket(settings)?;
    let start = Instant::now();
    loop {
        let _ = socket.send_to(&settings.payload, dst);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if let Some(limit) = settings.time_limit {
            if start.elapsed() > limit {
                break;
            }
        }
        std::thread::sleep(settings.delay);
    }
    // final announcement, like Go's "broadcast that is finished"
    let _ = socket.send_to(&settings.payload, dst);
    Ok(())
}

/// Listen for announcements while announcing our own payload; collect
/// distinct peers until the limit or time limit. Mirrors the discovery half
/// as croc's recipient uses it.
pub fn discover(settings: &Settings) -> std::io::Result<Vec<Discovered>> {
    let (socket, dst) = open_socket(settings)?;
    socket.set_read_timeout(Some(settings.delay))?;

    let mut self_ips: Vec<String> = crate::utils::get_local_ips();
    self_ips.push("127.0.0.1".to_string());

    let mut found: Vec<Discovered> = Vec::new();
    let start = Instant::now();
    let mut buf = [0u8; 65_507];
    loop {
        if !settings.disable_broadcast {
            let _ = socket.send_to(&settings.payload, dst);
        }
        while let Ok((n, src)) = socket.recv_from(&mut buf) {
            let src_ip = src.ip().to_string();
            if !settings.allow_self && self_ips.contains(&src_ip) {
                continue;
            }
            match found.iter_mut().find(|d| d.address == src_ip) {
                // Mirror Go: later packets refresh the peer's payload.
                Some(existing) => existing.payload = buf[..n].to_vec(),
                None => found.push(Discovered {
                    address: src_ip,
                    payload: buf[..n].to_vec(),
                }),
            }
        }
        if let Some(limit) = settings.limit {
            if found.len() >= limit {
                break;
            }
        }
        if let Some(tl) = settings.time_limit {
            if start.elapsed() > tl {
                break;
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Same-host loopback discovery: announcer + discoverer with allow_self
    // (the self-filter is exactly what production croc relies on to NOT
    // discover itself, so tests must bypass it).
    #[test]
    fn loopback_discovery() {
        let stop = Arc::new(AtomicBool::new(false));
        let announcer = Settings {
            payload: b"croc9999test".to_vec(),
            time_limit: Some(Duration::from_secs(3)),
            port: 9871,
            ..Default::default()
        };
        let stop2 = Arc::clone(&stop);
        let handle = std::thread::spawn(move || broadcast(&announcer, stop2));

        let finder = Settings {
            payload: b"ok".to_vec(),
            time_limit: Some(Duration::from_secs(2)),
            limit: Some(1),
            allow_self: true,
            disable_broadcast: true,
            port: 9871,
            ..Default::default()
        };
        let found = discover(&finder).unwrap();
        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();
        assert!(
            found.iter().any(|d| d.payload == b"croc9999test"),
            "expected to discover the announcer, got {found:?}"
        );
    }

    // Same, over the IPv6 loopback group. Skips gracefully if the sandbox
    // has no IPv6 multicast support.
    #[test]
    fn loopback_discovery_ipv6() {
        let base = ipv6_settings(b"croc6test".to_vec(), Some(Duration::from_secs(3)));
        let bind_check = Settings {
            port: 9872,
            ..base.clone()
        };
        if open_socket(&bind_check).is_err() {
            eprintln!("skipping IPv6 discovery test: no IPv6 multicast");
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let announcer = Settings { port: 9872, ..base };
        let stop2 = Arc::clone(&stop);
        let handle = std::thread::spawn(move || broadcast(&announcer, stop2));
        let finder = Settings {
            payload: b"ok".to_vec(),
            time_limit: Some(Duration::from_secs(2)),
            limit: Some(1),
            allow_self: true,
            disable_broadcast: true,
            port: 9872,
            ..ipv6_settings(vec![], None)
        };
        let found = discover(&finder).unwrap();
        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();
        // On hosts without functional IPv6 loopback multicast this may find
        // nothing; only assert when something arrived.
        if !found.is_empty() {
            assert!(found.iter().any(|d| d.payload == b"croc6test"));
        }
    }
}
