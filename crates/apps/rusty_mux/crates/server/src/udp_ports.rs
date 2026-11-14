//! `--udp`'s address: one port, or a range of them for an account's server
//! behind the login front door, which binds the first free one.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;

use rusty_tokio::io::UdpSocket;

/// The port when an address is given alone.
pub const DEFAULT_PORT: u16 = 60001;

/// The lowest port a range may start at: an account's server is not root.
pub const RANGE_FLOOR: u16 = 1024;

/// An address and the ports to try on it, `lo` to `hi`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpPorts {
    pub ip: IpAddr,
    pub lo: u16,
    pub hi: u16,
}

impl UdpPorts {
    /// Just `addr`.
    pub fn single(addr: SocketAddr) -> Self {
        Self {
            ip: addr.ip(),
            lo: addr.port(),
            hi: addr.port(),
        }
    }

    /// Binds the first free port. One port busy is an error as it always
    /// was; a range with none free is `AddrInUse` too, for the caller to
    /// decide what that means.
    pub async fn bind(&self) -> io::Result<UdpSocket> {
        for port in self.lo..=self.hi {
            match UdpSocket::bind(SocketAddr::new(self.ip, port)).await {
                Err(e) if e.kind() == io::ErrorKind::AddrInUse => {}
                other => return other,
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("no free UDP port in {self}"),
        ))
    }
}

impl UdpPorts {
    /// Only the `ADDR:LO-HI` form: what the front door hands every account
    /// server, so one port or none would leave all but the first without.
    pub fn range(text: &str) -> Result<Self, String> {
        let ports: Self = text.parse()?;
        if ports.lo == ports.hi && !text.contains('-') {
            return Err(format!("{text}: not a range, ADDR:LO-HI"));
        }
        if ports.lo < RANGE_FLOOR {
            return Err(format!("{text}: a range starts at {RANGE_FLOOR} or above"));
        }
        Ok(ports)
    }
}

impl FromStr for UdpPorts {
    type Err = String;

    /// `ADDR`, `ADDR:PORT` or `ADDR:LO-HI`; an IPv6 address with a port goes
    /// in brackets. A range must start at [`RANGE_FLOOR`] or above.
    fn from_str(text: &str) -> Result<Self, String> {
        let bad = || format!("{text}: not an address");
        if let Ok(addr) = text.parse::<SocketAddr>() {
            return Ok(Self::single(addr));
        }
        if let Ok(ip) = text.parse::<IpAddr>() {
            return Ok(Self::single(SocketAddr::new(ip, DEFAULT_PORT)));
        }
        let (host, ports) = text.rsplit_once(':').ok_or_else(bad)?;
        // IPv6 only in a matched pair of brackets, IPv4 bare.
        let ip = match host.strip_prefix('[') {
            Some(inner) => inner
                .strip_suffix(']')
                .and_then(|v6| v6.parse::<Ipv6Addr>().ok())
                .map(IpAddr::V6),
            None => host.parse::<Ipv4Addr>().ok().map(IpAddr::V4),
        }
        .ok_or_else(bad)?;
        let (lo, hi) = ports.split_once('-').ok_or_else(bad)?;
        let (lo, hi): (u16, u16) = (
            lo.parse().map_err(|_| bad())?,
            hi.parse().map_err(|_| bad())?,
        );
        if lo > hi {
            return Err(format!("{text}: the range runs backwards"));
        }
        if lo < RANGE_FLOOR {
            return Err(format!("{text}: a range starts at {RANGE_FLOOR} or above"));
        }
        Ok(Self { ip, lo, hi })
    }
}

impl std::fmt::Display for UdpPorts {
    /// The form [`FromStr`] reads back.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let host = match self.ip {
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) => format!("[{ip}]"),
        };
        match self.lo == self.hi {
            true => write!(f, "{host}:{}", self.lo),
            false => write!(f, "{host}:{}-{}", self.lo, self.hi),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<UdpPorts, String> {
        text.parse()
    }

    #[test]
    fn an_address_alone_takes_the_default_port() {
        let want = UdpPorts::single("0.0.0.0:60001".parse().unwrap());
        assert_eq!(parse("0.0.0.0"), Ok(want));
        assert_eq!(parse("::1").map(|p| p.lo), Ok(DEFAULT_PORT));
    }

    #[test]
    fn a_port_or_a_range_is_read_and_printed_back() {
        for text in [
            "0.0.0.0:7000",
            "[::1]:7000",
            "10.0.0.1:2000-2009",
            "[::1]:2000-2001",
        ] {
            assert_eq!(parse(text).unwrap().to_string(), text);
        }
        let range = parse("127.0.0.1:2000-2009").unwrap();
        assert_eq!((range.lo, range.hi), (2000, 2009));
    }

    #[test]
    fn the_front_door_takes_only_a_real_range_above_the_floor() {
        assert!(UdpPorts::range("10.0.0.1:2000-2009").is_ok());
        for text in [
            "10.0.0.1",
            "10.0.0.1:2000",
            "10.0.0.1:80",
            "10.0.0.1:0",
            "[::1]:80",
            "10.0.0.1:80-90",
        ] {
            assert!(UdpPorts::range(text).is_err(), "{text}");
        }
    }

    #[test]
    fn an_ipv6_address_needs_its_brackets_matched() {
        for text in [
            "::1:2000-2009",
            "[::1:2000-2009",
            "[[::1]]:2000-2009",
            "::1]:2000-2009",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
        assert!(parse("[::1]:2000-2009").is_ok());
    }

    #[test]
    fn a_bad_range_is_refused() {
        for text in [
            "nowhere",
            "1.2.3.4:",
            "1.2.3.4:5000-4000",
            "1.2.3.4:80-90",
            "1.2.3.4:1000-",
            "1.2.3.4:a-b",
            "1.2.3.4:2000-99999",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }
}
