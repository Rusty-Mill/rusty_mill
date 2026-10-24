---
category: Security
changelog: Resolve RUSTSEC-2026-0387 across AWS SigV4a, JWT RustCrypto and Croc using crypto-bigint 0.7.6 and pinned source patches.
---
## 2026-10-24 - Remove the vulnerable crypto-bigint dependency graph

- **Security:** update `crypto-bigint` 0.7.5 to 0.7.6 and remove its vulnerable
  0.5.5 dependency graph. `ctutils` remains at 0.4.5. The all-features advisory
  policy is unchanged; RUSTSEC-2026-0387 is not suppressed.
- **AWS SigV4a:** temporarily patch the already-locked `aws-sigv4` 1.6.0 source
  to use published P-256 0.14 and fixed bigint 0.7.6. Key generation, signing,
  public APIs and the `sigv4a` feature remain intact; RemindMe cloud backup
  retains its existing capabilities.
- **JWT:** temporarily patch the already-locked `jsonwebtoken` 11.0.0 source
  to use published P-256/P-384 0.14. Preserve its public signature-2 provider
  API, RustCrypto backend, RSA/Ed25519/HMAC algorithms, PEM/DER key formats
  and raw ECDSA signature encoding while adapting the internal signature-3
  verifier, signing return type, SEC1 names and PKCS#8 trait imports.
- **Croc:** migrate the three NIST curves to published 0.14 releases using
  their renamed SEC1 conversion APIs. Scalar reduction, PAKE transcript and
  affine wire coordinates are unchanged. Add bidirectional stock-Go transfers
  for p256/p384/p521/siec to the existing interoperability suite, pin its
  default Go oracle, and resolve binaries from Cargo's actual target directory.
- **Maintenance:** [vendor/README.md](../vendor/README.md) records archive
  checksums, exact source revisions, complete patch diffs, reproduction,
  validation, ownership and removal conditions. Source-patched package
  versions remain truthful. No crypto-bigint source/version metadata is
  forged and no crypto backend is switched.
- **Known limitations:** temporary vendored patches must be maintained until
  compatible upstream releases remove the old graph. This PR still needs
  exact-head CI and independent security review. The workspace's unrelated
  existing advisory exceptions and yanked-package warning are unchanged.
