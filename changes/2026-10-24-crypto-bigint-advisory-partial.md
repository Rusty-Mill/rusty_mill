---
category: Security
changelog: Update crypto-bigint 0.7.5 to 0.7.6; RUSTSEC-2026-0387 remains open for the 0.5.5 dependency graph.
---
## 2026-10-24 - Partial crypto-bigint timing advisory remediation

- **Security:** update the existing `crypto-bigint` 0.7 lockfile resolution to
  0.7.6, the first patched release for
  [RUSTSEC-2026-0387](https://github.com/RustSec/advisory-db/blob/main/crates/crypto-bigint/RUSTSEC-2026-0387.md).
  The existing `ctutils` 0.4.5 is retained. The patched crate requires Rust
  1.87, below both affected direct consumers' Rust 1.88 declarations and the
  workspace's pinned 1.98.1 toolchain. No manifest, feature, backend, or security
  policy changes are included.
- **Affected paths fixed:** Croc's direct bigint/SIEC backend and A2A signing's
  `p256` 0.14 / `elliptic-curve` 0.14 dependency closure now use 0.7.6.
- **Known limitations:** `crypto-bigint` 0.5.5 remains vulnerable. The
  all-features graph still reaches it through AWS SigV4a for RemindMe cloud
  backup, `jsonwebtoken` 11's RustCrypto backend for agentgateway-auth and
  rusty-mcp's test graph, and Croc's `p256`/`p384`/`p521` 0.13 curves. This is
  a partial remediation; the advisory gate must continue to fail.

### Remaining remediation investigation

Published manifests were rechecked on 2026-10-24:

- [`aws-sigv4` 1.6.3](https://docs.rs/crate/aws-sigv4/1.6.3/source/Cargo.toml)
  already uses `p256` 0.14 and `ecdsa` 0.17, but retains a direct
  `crypto-bigint = "0.5.4"` dependency under `sigv4a`. Its key derivation uses
  `U256`, checked arithmetic, comparison and byte encoding. Updating the SDK
  alone therefore cannot close this advisory.
- [`jsonwebtoken` 11.1.0](https://docs.rs/crate/jsonwebtoken/11.1.0/source/Cargo.toml)
  still uses `p256`/`p384` 0.13. An isolated manifest-only 0.14 migration probe
  fails to compile: recoverable signing changed its return type; verification
  crosses `signature` 2/3 traits; SEC1 methods were renamed; PKCS#8 decoding
  crosses 0.10/0.11 traits used by the retained Ed25519 backend. A fix must
  preserve all existing algorithms and key formats, including RSA and Ed25519.
- Croc's isolated PAKE probe with the three published 0.14 curves passes all
  eight existing PAKE tests after changing `EncodedPoint` to `Sec1Point` and
  the corresponding conversion traits/methods. This includes Go-generated
  arithmetic vectors, large-scalar reduction, all-curve exchanges, wrong
  passwords and bad-point rejection. This experiment is not a live Go interop
  validation and is not included as a production migration in this PR.

The smallest follow-up is a separately reviewed Croc 0.14 migration with
live Go-to-Rust and Rust-to-Go transfers for every supported curve, followed
by upstream AWS key-derivation and jsonwebtoken RustCrypto migrations to
0.7.6-backed arithmetic. Until compatible upstream releases exist, selecting
and maintaining pinned source patches for those upstream crates is a separate
review decision; cryptographic behavior and pure-Rust backends must remain
intact. There is no published compatible 0.5 patch. A private 0.5 backport would
also need upstream security review and an official advisory-range correction;
changing its version metadata cannot establish remediation.
