// Shared characterization fixtures, included by each caller's Cargo unit tests.

use std::net::{IpAddr, Ipv6Addr};

/// Check a real caller predicate against its pre-extraction decisions.
pub fn assert_policy(caller: &str, classify: fn(IpAddr) -> bool) {
    let column = match caller {
        "a2a" => 1,
        "memory" => 2,
        "linkpreview" => 3,
        other => panic!("unknown fixture caller: {other}"),
    };
    let mut literals = 0;
    let mut ipv4 = 0;
    for row in include_str!("addresses.tsv")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let cells: Vec<_> = row.split('\t').collect();
        assert_eq!(cells.len(), 5, "invalid fixture row: {row}");
        let ip: IpAddr = cells[0].parse().expect("fixture IP must parse");
        let expected = match cells[column] {
            "block" => true,
            "pass" => false,
            other => panic!("invalid decision {other}: {row}"),
        };
        let check = |address, blocked| {
            assert_eq!(
                classify(address),
                blocked,
                "{caller}: {address} ({}); true=block, false=pass",
                cells[4]
            );
        };
        check(ip, expected);
        literals += 1;
        let IpAddr::V4(v4) = ip else { continue };
        ipv4 += 1;
        let [a, b, c, d] = v4.octets();
        let dotted: IpAddr = format!("::ffff:{v4}").parse().unwrap();
        let hex: IpAddr = format!("::ffff:{a:02x}{b:02x}:{c:02x}{d:02x}")
            .parse()
            .unwrap();
        assert_eq!(dotted, hex);
        assert_eq!(dotted, IpAddr::V6(v4.to_ipv6_mapped()));
        check(dotted, expected);
        check(hex, expected);

        let nat64 = Ipv6Addr::new(
            0x64,
            0xff9b,
            0,
            0,
            0,
            0,
            u16::from_be_bytes([a, b]),
            u16::from_be_bytes([c, d]),
        );
        let dotted: IpAddr = format!("64:ff9b::{v4}").parse().unwrap();
        assert_eq!(dotted, IpAddr::V6(nat64));
        // Only A2A interprets well-known NAT64 as its embedded IPv4 today.
        check(dotted, caller == "a2a" && expected);
        check(IpAddr::V6(nat64), caller == "a2a" && expected);

        // Compatible IPv6 is not mapped IPv6. Only zero/one become ::/::1.
        let compatible: IpAddr = format!("::{v4}").parse().unwrap();
        check(compatible, u32::from(v4) <= 1);
    }
    assert!(literals > 0 && ipv4 > 0, "fixtures must not disappear");
    println!("{caller}: {literals} literals + {} derived forms", ipv4 * 5);
}
