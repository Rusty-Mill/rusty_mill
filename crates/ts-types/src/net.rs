//! Network address types not covered by `std::net`.

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

/// An IP prefix in CIDR notation (`netip.Prefix` in Go), e.g.
/// `"100.64.0.1/32"` or `"fd7a:115c:a1e0::1/128"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IpPrefix {
    pub addr: IpAddr,
    pub bits: u8,
}

/// Error parsing an [`IpPrefix`] from CIDR notation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrefixParseError {
    #[error("prefix has no '/'")]
    MissingSlash,
    #[error("invalid IP address in prefix")]
    BadAddr,
    #[error("invalid prefix length")]
    BadBits,
}

impl IpPrefix {
    /// Maximum valid prefix length for the address family.
    fn max_bits(addr: IpAddr) -> u8 {
        match addr {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        }
    }
}

impl FromStr for IpPrefix {
    type Err = PrefixParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr, bits) = s.split_once('/').ok_or(PrefixParseError::MissingSlash)?;
        let addr: IpAddr = addr.parse().map_err(|_| PrefixParseError::BadAddr)?;
        let bits: u8 = bits.parse().map_err(|_| PrefixParseError::BadBits)?;
        if bits > Self::max_bits(addr) {
            return Err(PrefixParseError::BadBits);
        }
        Ok(IpPrefix { addr, bits })
    }
}

impl fmt::Display for IpPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.bits)
    }
}

impl serde::Serialize for IpPrefix {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for IpPrefix {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = IpPrefix;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an IP prefix in CIDR notation")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<IpPrefix, E> {
                v.parse().map_err(E::custom)
            }
        }
        d.deserialize_str(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_display_round_trip() {
        for s in [
            "100.64.0.1/32",
            "100.64.0.0/10",
            "fd7a:115c:a1e0::1/128",
            "0.0.0.0/0",
        ] {
            let p: IpPrefix = s.parse().unwrap();
            assert_eq!(p.to_string(), s);
        }
    }

    #[test]
    fn rejects_invalid() {
        assert_eq!(
            "100.64.0.1".parse::<IpPrefix>(),
            Err(PrefixParseError::MissingSlash)
        );
        assert_eq!(
            "bogus/8".parse::<IpPrefix>(),
            Err(PrefixParseError::BadAddr)
        );
        assert_eq!(
            "10.0.0.0/33".parse::<IpPrefix>(),
            Err(PrefixParseError::BadBits)
        );
        assert_eq!("::/129".parse::<IpPrefix>(), Err(PrefixParseError::BadBits));
        assert_eq!(
            "10.0.0.0/x".parse::<IpPrefix>(),
            Err(PrefixParseError::BadBits)
        );
    }
}
