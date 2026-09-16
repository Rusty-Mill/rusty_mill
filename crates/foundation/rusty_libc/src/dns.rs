//! A minimal stub DNS resolver (RFC 1035) for A/AAAA records.
//!
//! This crate has no libc to hand `getaddrinfo` off to and takes no
//! dependencies, so hostname resolution is implemented directly against the
//! wire format: read the nameserver list from `/etc/resolv.conf`, send an A
//! or AAAA query over UDP port 53, parse the answer. See
//! [ADR-0003](../../docs/adr/0003-add-sockets-tcp-udp-and-dns-resolution.md)
//! for why.
//!
//! Deliberately narrow scope -- enough to resolve a hostname to an address
//! list for an outbound `connect`, not a general-purpose DNS client:
//! IPv4 nameservers only (the transport; the *records* queried can still be
//! AAAA), no `/etc/hosts` consultation, no caching, no CNAME chasing beyond
//! what the nameserver already resolved server-side, no EDNS0 (so answers
//! larger than 512 bytes truncate -- rare for A/AAAA lookups). No allocation:
//! results are returned in a fixed-capacity list, matching this crate's
//! no_std/no-alloc core.

use crate::arch::Errno;
use crate::fd;
use crate::rand;
use crate::socket::{self, SockAddrIn};

const QTYPE_A: u16 = 1;
const QTYPE_AAAA: u16 = 28;
const QCLASS_IN: u16 = 1;
const MAX_NAMESERVERS: usize = 3;
const MAX_ADDRS: usize = 8;
const RESPONSE_TIMEOUT_MS: i32 = 2000;

/// Up to 8 resolved IPv4 addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddrList4 {
    addrs: [[u8; 4]; MAX_ADDRS],
    len: usize,
}

impl AddrList4 {
    /// The resolved addresses, in the order the nameserver returned them.
    pub fn as_slice(&self) -> &[[u8; 4]] {
        &self.addrs[..self.len]
    }
}

/// Up to 8 resolved IPv6 addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddrList6 {
    addrs: [[u8; 16]; MAX_ADDRS],
    len: usize,
}

impl AddrList6 {
    /// The resolved addresses, in the order the nameserver returned them.
    pub fn as_slice(&self) -> &[[u8; 16]] {
        &self.addrs[..self.len]
    }
}

/// Resolve `hostname`'s A (IPv4) records.
pub fn resolve_a(hostname: &str) -> Result<AddrList4, Errno> {
    let (buf, n) = query(hostname, QTYPE_A)?;
    parse_answers_a(&buf, n)
}

/// Resolve `hostname`'s AAAA (IPv6) records.
pub fn resolve_aaaa(hostname: &str) -> Result<AddrList6, Errno> {
    let (buf, n) = query(hostname, QTYPE_AAAA)?;
    parse_answers_aaaa(&buf, n)
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut count = 0;
    for part in s.split('.') {
        if count >= 4 {
            return None;
        }
        out[count] = part.parse().ok()?;
        count += 1;
    }
    if count == 4 {
        Some(out)
    } else {
        None
    }
}

/// Read the `nameserver` lines of `/etc/resolv.conf` (IPv4 only).
fn read_nameservers() -> Result<([[u8; 4]; MAX_NAMESERVERS], usize), Errno> {
    let f = fd::open(c"/etc/resolv.conf", fd::O_RDONLY, 0)?;
    let mut raw = [0u8; 4096];
    let mut total = 0;
    loop {
        let n = match fd::read(f, &mut raw[total..]) {
            Ok(n) => n,
            Err(e) => {
                fd::close(f).ok();
                return Err(e);
            }
        };
        if n == 0 || total + n >= raw.len() {
            total += n;
            break;
        }
        total += n;
    }
    fd::close(f).ok();

    let text = core::str::from_utf8(&raw[..total]).map_err(|_| Errno::EINVAL)?;
    let mut servers = [[0u8; 4]; MAX_NAMESERVERS];
    let mut count = 0;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("nameserver") {
            if let Some(addr) = parse_ipv4(rest.trim()) {
                if count < servers.len() {
                    servers[count] = addr;
                    count += 1;
                }
            }
        }
    }
    if count == 0 {
        return Err(Errno::EINVAL);
    }
    Ok((servers, count))
}

/// Encode a DNS query for `hostname`/`qtype` into `buf`, returning the
/// encoded length.
fn build_query(buf: &mut [u8], id: u16, hostname: &str, qtype: u16) -> Result<usize, Errno> {
    if hostname.is_empty() {
        return Err(Errno::EINVAL);
    }
    let mut pos = 0;
    let put = |buf: &mut [u8], pos: &mut usize, bytes: &[u8]| -> Result<(), Errno> {
        if *pos + bytes.len() > buf.len() {
            return Err(Errno::EINVAL);
        }
        buf[*pos..*pos + bytes.len()].copy_from_slice(bytes);
        *pos += bytes.len();
        Ok(())
    };

    put(buf, &mut pos, &id.to_be_bytes())?;
    put(buf, &mut pos, &0x0100u16.to_be_bytes())?; // flags: RD=1
    put(buf, &mut pos, &1u16.to_be_bytes())?; // qdcount
    put(buf, &mut pos, &0u16.to_be_bytes())?; // ancount
    put(buf, &mut pos, &0u16.to_be_bytes())?; // nscount
    put(buf, &mut pos, &0u16.to_be_bytes())?; // arcount

    for label in hostname.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(Errno::EINVAL);
        }
        put(buf, &mut pos, &[label.len() as u8])?;
        put(buf, &mut pos, label.as_bytes())?;
    }
    put(buf, &mut pos, &[0u8])?; // root label

    put(buf, &mut pos, &qtype.to_be_bytes())?;
    put(buf, &mut pos, &QCLASS_IN.to_be_bytes())?;
    Ok(pos)
}

/// Advance past a (possibly compressed) name at `pos`, returning the
/// position just after it.
fn skip_name(buf: &[u8], mut pos: usize) -> Result<usize, Errno> {
    loop {
        if pos >= buf.len() {
            return Err(Errno::EINVAL);
        }
        let len = buf[pos];
        if len == 0 {
            pos += 1;
            return Ok(pos);
        }
        if len & 0xC0 == 0xC0 {
            if pos + 1 >= buf.len() {
                return Err(Errno::EINVAL);
            }
            return Ok(pos + 2);
        }
        pos += 1 + len as usize;
    }
}

fn read_u16(buf: &[u8], pos: usize) -> Result<u16, Errno> {
    if pos + 2 > buf.len() {
        return Err(Errno::EINVAL);
    }
    Ok(u16::from_be_bytes([buf[pos], buf[pos + 1]]))
}

/// Parsed, un-typed response header fields shared by A/AAAA parsing, plus
/// the position of the first answer record.
struct ParsedHeader {
    ancount: usize,
    first_answer: usize,
}

fn parse_header(buf: &[u8], n: usize) -> Result<ParsedHeader, Errno> {
    if n < 12 {
        return Err(Errno::EINVAL);
    }
    let flags = read_u16(buf, 2)?;
    let rcode = flags & 0x000F;
    if rcode == 3 {
        return Err(Errno::ENOENT); // NXDOMAIN
    }
    if rcode != 0 {
        return Err(Errno::EINVAL);
    }
    let qdcount = read_u16(buf, 4)? as usize;
    let ancount = read_u16(buf, 6)? as usize;

    let mut pos = 12;
    for _ in 0..qdcount {
        pos = skip_name(buf, pos)?;
        pos += 4; // qtype + qclass
        if pos > n {
            return Err(Errno::EINVAL);
        }
    }
    Ok(ParsedHeader {
        ancount,
        first_answer: pos,
    })
}

fn parse_answers_a(buf: &[u8], n: usize) -> Result<AddrList4, Errno> {
    let header = parse_header(buf, n)?;
    let mut out = AddrList4 {
        addrs: [[0; 4]; MAX_ADDRS],
        len: 0,
    };
    let mut pos = header.first_answer;
    for _ in 0..header.ancount {
        pos = skip_name(buf, pos)?;
        if pos + 10 > n {
            return Err(Errno::EINVAL);
        }
        let rtype = read_u16(buf, pos)?;
        let rdlength = read_u16(buf, pos + 8)? as usize;
        pos += 10;
        if pos + rdlength > n {
            return Err(Errno::EINVAL);
        }
        if rtype == QTYPE_A && rdlength == 4 && out.len < MAX_ADDRS {
            out.addrs[out.len].copy_from_slice(&buf[pos..pos + 4]);
            out.len += 1;
        }
        pos += rdlength;
    }
    if out.len == 0 {
        return Err(Errno::ENOENT);
    }
    Ok(out)
}

fn parse_answers_aaaa(buf: &[u8], n: usize) -> Result<AddrList6, Errno> {
    let header = parse_header(buf, n)?;
    let mut out = AddrList6 {
        addrs: [[0; 16]; MAX_ADDRS],
        len: 0,
    };
    let mut pos = header.first_answer;
    for _ in 0..header.ancount {
        pos = skip_name(buf, pos)?;
        if pos + 10 > n {
            return Err(Errno::EINVAL);
        }
        let rtype = read_u16(buf, pos)?;
        let rdlength = read_u16(buf, pos + 8)? as usize;
        pos += 10;
        if pos + rdlength > n {
            return Err(Errno::EINVAL);
        }
        if rtype == QTYPE_AAAA && rdlength == 16 && out.len < MAX_ADDRS {
            out.addrs[out.len].copy_from_slice(&buf[pos..pos + 16]);
            out.len += 1;
        }
        pos += rdlength;
    }
    if out.len == 0 {
        return Err(Errno::ENOENT);
    }
    Ok(out)
}

/// Send `hostname`/`qtype` to each configured nameserver (up to two rounds),
/// returning the raw response bytes.
fn query(hostname: &str, qtype: u16) -> Result<([u8; 512], usize), Errno> {
    let (servers, count) = read_nameservers()?;

    let mut id_buf = [0u8; 2];
    rand::getrandom(&mut id_buf, 0)?;
    let id = u16::from_be_bytes(id_buf);

    let mut qbuf = [0u8; 256];
    let qlen = build_query(&mut qbuf, id, hostname, qtype)?;

    let sock = socket::socket(socket::AF_INET, socket::SOCK_DGRAM, 0)?;
    let result = (|| -> Result<([u8; 512], usize), Errno> {
        for _attempt in 0..2 {
            for server in servers.iter().take(count) {
                let dest = SockAddrIn::new(*server, 53);
                if socket::sendto_in(sock, &qbuf[..qlen], 0, &dest).is_err() {
                    continue;
                }

                let mut pfd = [fd::PollFd {
                    fd: sock,
                    events: fd::POLLIN,
                    revents: 0,
                }];
                let ready = fd::poll(&mut pfd, RESPONSE_TIMEOUT_MS)?;
                if ready == 0 {
                    continue; // timed out waiting on this server
                }

                let mut rbuf = [0u8; 512];
                let (n, from) = match socket::recvfrom_in(sock, &mut rbuf, 0) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if from.octets() != *server || n < 12 {
                    continue; // not from the server we queried, or too short
                }
                let resp_id = u16::from_be_bytes([rbuf[0], rbuf[1]]);
                if resp_id != id {
                    continue; // stale/spoofed reply, keep waiting
                }
                return Ok((rbuf, n));
            }
        }
        Err(Errno::ETIMEDOUT)
    })();

    fd::close(sock).ok();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_query_encodes_expected_wire_bytes() {
        let mut buf = [0u8; 64];
        let n = build_query(&mut buf, 0x1234, "a.io", QTYPE_A).expect("build_query");
        assert_eq!(
            &buf[..n],
            &[
                0x12, 0x34, // id
                0x01, 0x00, // flags: RD
                0x00, 0x01, // qdcount
                0x00, 0x00, // ancount
                0x00, 0x00, // nscount
                0x00, 0x00, // arcount
                0x01, b'a', 0x02, b'i', b'o', 0x00, // "a.io"
                0x00, 0x01, // qtype A
                0x00, 0x01, // qclass IN
            ][..]
        );
    }

    #[test]
    fn build_query_rejects_empty_and_overlong_labels() {
        let mut buf = [0u8; 128];
        assert_eq!(build_query(&mut buf, 1, "", QTYPE_A), Err(Errno::EINVAL));
        assert_eq!(
            build_query(&mut buf, 1, "a..b", QTYPE_A),
            Err(Errno::EINVAL)
        );
        // A single label over 63 bytes is invalid regardless of overall
        // hostname length.
        let overlong_label = "a".repeat(64);
        assert_eq!(
            build_query(&mut buf, 1, &overlong_label, QTYPE_A),
            Err(Errno::EINVAL)
        );
    }

    #[test]
    fn build_query_rejects_a_hostname_too_long_for_the_buffer() {
        let mut buf = [0u8; 20]; // too small even for "a.io"'s own encoding plus header
        assert_eq!(
            build_query(&mut buf, 1, "a.io", QTYPE_A),
            Err(Errno::EINVAL)
        );
    }

    /// A hand-built, canonical response to the query built above: id
    /// 0x1234, one A answer (a compressed-name pointer back to the
    /// question) resolving "a.io" to 93.184.216.34.
    const CANNED_RESPONSE: &[u8] = &[
        0x12, 0x34, // id
        0x81, 0x80, // flags: response, RD+RA, rcode 0
        0x00, 0x01, // qdcount
        0x00, 0x01, // ancount
        0x00, 0x00, // nscount
        0x00, 0x00, // arcount
        0x01, b'a', 0x02, b'i', b'o', 0x00, // question: "a.io"
        0x00, 0x01, // qtype A
        0x00, 0x01, // qclass IN
        0xC0, 0x0C, // answer name: pointer to offset 12
        0x00, 0x01, // type A
        0x00, 0x01, // class IN
        0x00, 0x00, 0x01, 0x2C, // ttl 300
        0x00, 0x04, // rdlength
        93, 184, 216, 34, // rdata
    ];

    #[test]
    fn parses_a_canned_response() {
        let addrs = parse_answers_a(CANNED_RESPONSE, CANNED_RESPONSE.len()).expect("parse");
        assert_eq!(addrs.as_slice(), &[[93, 184, 216, 34]]);
    }

    #[test]
    fn nxdomain_rcode_is_enoent() {
        let mut resp = CANNED_RESPONSE.to_vec();
        resp[3] = 0x83; // rcode 3 = NXDOMAIN
        assert_eq!(parse_answers_a(&resp, resp.len()), Err(Errno::ENOENT));
    }

    #[test]
    fn truncated_response_is_einval() {
        assert_eq!(
            parse_answers_a(&CANNED_RESPONSE[..20], 20),
            Err(Errno::EINVAL)
        );
    }

    #[test]
    fn resolve_a_example_com_returns_at_least_one_address() {
        let addrs = resolve_a("example.com").expect("resolve_a example.com");
        assert!(!addrs.as_slice().is_empty());
    }

    #[test]
    fn resolve_a_nxdomain_is_enoent() {
        // RFC 2606 reserves .invalid for exactly this purpose.
        let err = resolve_a("this-name-should-not-exist-rusty-libc.invalid").unwrap_err();
        assert_eq!(err, Errno::ENOENT);
    }

    #[test]
    fn resolve_a_empty_hostname_is_einval() {
        assert_eq!(resolve_a(""), Err(Errno::EINVAL));
    }
}
