# Issue #418: address-class conformance

Run the actual caller predicates through their Cargo unit tests:

```sh
cargo test --locked -p rusty_a2a -p nexus-memory -p nexus-linkpreview --features rusty_a2a/server --lib ssrf_address_conformance -- --nocapture
cargo test --locked -p rusty_ip
```

The first command runs three tests. Each crate includes `policy.rs` in its
test module and passes its production predicate directly. No classifier is
copied or extracted by the test. These tests perform no DNS or socket operations.

`addresses.tsv` records 81 explicitly reviewed literals from the three policies
at main commit `dab8499d034c5ccc61182f3c9c40ef23f4157bb1`, before extraction.
The separate expected columns retain intentional caller differences. The 55
IPv4 fixtures also run as mapped IPv6 (dotted and hex), well-known NAT64 /96
(dotted and binary), and deprecated IPv4-compatible IPv6. This produces 356
classifier decisions per caller, or 1,068 total, including equivalent spellings.
Prefix endpoints and their neighbors, limited broadcast, documentation ranges,
and lookalike embedded-address prefixes cover accidental broadening and
relaxation. `pass` means the classifier returns false; it does not claim global
routability or permission to connect.

The tests passed against the old predicates before extraction and against the
new shared facts plus caller-local policies. A2A keeps additional exclusions
and NAT64 interpretation; neither Nexus caller acquires them. All three keep
mapped IPv4 interpretation. The foundation crate separately tests the facts API,
including overlapping classes and embedded forms.

The fixtures do not exercise URL parsing, DNS, redirects, opt-ins, private-hub
overrides, or connection pinning. Existing caller regression tests cover those
contracts and remain necessary. No CI workflow is changed; the fixture tests
are ordinary crate unit tests, with A2A requiring the `server` feature.

These fixtures live inside `rusty_ip` so every fixture-only edit has a Cargo
package owner. The existing all-features impact graph selects `rusty_ip` plus
its reverse dependents, including `rusty_a2a`, `nexus-memory`, and
`nexus-linkpreview`. The existing component jobs run tests and Clippy with
`--all-features`, activating A2A's `server` feature and its conformance test.
This remains scoped selection; it does not depend on a simultaneous manifest
or lockfile change to force a full workspace run.

Verify the affected set from the repository root:

```sh
cargo metadata --locked --format-version=1 --all-features > metadata.json
printf '%s\n' crates/foundation/rusty_ip/tests/ssrf_conformance/addresses.tsv | python3 .github/scripts/affected_crates.py metadata.json
```

The same ownership applies to `policy.rs` and this README.

See the [scope and validation ledger](../../../../../docs/Monorepo_Reviews/ISSUE-418-SSRF-CONFORMANCE.md)
for the preserved policy table, transport boundaries, and local check results.
