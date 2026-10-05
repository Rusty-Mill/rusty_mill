# rusty_ip

Allocation-free, dependency-free `no_std` address classification for the
workspace's outbound-address guards. `classify(IpAddr)` returns an
`AddressClass`; callers retain their own allow/deny policy.

The facts distinguish IPv4-mapped from well-known NAT64 embedded IPv4. They do
not automatically normalize either form, classify every IANA special-use block,
or decide whether a connection is safe. `Other` includes documentation ranges.
For overlapping ranges, unspecified wins over `0/8` and limited broadcast wins
over reserved `240/4`.

This crate owns no DNS, URL, redirect, proxy, credential, or private-address
override behavior. Those remain in the callers. Its first consumers are A2A,
Nexus memory, and Nexus link-preview, with their existing policy differences
covered by this package's [`tests/ssrf_conformance`](tests/ssrf_conformance/README.md).
Keeping the fixtures inside the package lets CI select all callers through
its existing reverse-dependency analysis, even for fixture-only edits.

Run `cargo test -p rusty_ip`; caller tests named `ssrf_address_conformance`
exercise the shared policy fixtures through the actual production predicates.
