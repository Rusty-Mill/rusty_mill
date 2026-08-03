# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- **Hand-rolled TLS engine, stage 1: the TLS 1.3 record layer** (rusty_tls#25).
  A new `handrolled::record` module implementing RFC 8446 §5 — AEAD
  protection, framing, inner content types, padding, and the §5.3 nonce
  construction — over AES-128-GCM, AES-256-GCM, and ChaCha20-Poly1305.
  Nothing in the crate's public API routes through it; `rustls` remains the
  engine behind every exported type.
- `handrolled-engine` cargo feature, **off by default and never to become a
  default**, which must be combined with `--cfg rusty_tls_handrolled` for the
  module to compile at all. Two gates rather than one because cargo features
  are unified across a dependency graph and a `--cfg` flag is not — see
  ADR-0002. Enabling the feature without the cfg compiles a documented stub
  module explaining how to enable it, rather than silently doing nothing.
- `ring` as an optional direct dependency, enabled by `handrolled-engine`.
  Already present transitively as rustls' crypto provider, so this adds a
  dependency edge rather than new code to any build.
- ADR-0002, recording the never-default guarantee as a binding decision,
  the staging order for the remaining work, and the bar each stage must
  clear before it lands.
- A second CI job that builds and tests the hand-rolled engine with the cfg
  set, including a check that the gated tests actually ran — a typo in the
  cfg name would otherwise compile the suites down to zero tests and still
  report success.
- **Hand-rolled engine, stage 2a: DER decoding and X.509 certificate
  parsing** (rusty_tls#25). `handrolled::der` is a strict DER reader that
  refuses every non-canonical encoding DER forbids; `handrolled::x509` parses
  certificates on top of it, keeping `tbsCertificate`, `issuer`, and
  `subject` as borrows of the original bytes rather than re-encoding them.
  Understood extensions are `basicConstraints`, `keyUsage`,
  `extendedKeyUsage`, and `subjectAltName`; critical extensions that are
  *not* understood are collected and reported so a validator can comply with
  RFC 5280 §6.1.3(f).

  **This validates nothing** — no signature check, no clock, no chain, no
  name matching. That is stage 2b, which does not exist yet. ADR-0002's
  staging table is updated to record the split and why.
- **Fuzzing for both hand-rolled parsers.** `tests/handrolled_fuzz.rs` runs on
  every pull request on stable: a deterministic, seeded fuzzer that mutates
  the machine's real trust anchors rather than generating random noise, and
  asserts canonicality (anything the DER reader accepts must re-encode to
  exactly the accepted bytes), field provenance, determinism, and termination.
  It measures its own reach and fails if too few mutants get past the outer
  framing, so it cannot silently degrade into testing nothing.
  `fuzz/` adds coverage-guided libFuzzer targets for deliberate longer runs
  (nightly; 7.1M executions clean at the time of landing).
- **Hand-rolled engine, stage 2b-i: certificate signature verification**
  (rusty_tls#25). `handrolled::verify` answers whether a certificate was
  signed by a given key, over RSA PKCS#1 v1.5 with SHA-256/384/512, ECDSA with
  SHA-256/384 on P-256 and P-384, and Ed25519. SHA-1, MD5, and RSASSA-PSS are
  refused rather than verified, and refusal is always an error and never a
  qualified success.

  **This proves authorship, not authority.** It builds no chain, reads no
  clock, and checks no constraint, so it cannot make a trust decision by
  itself — an attacker can sign their own certificate. Path validation is
  stage 2b-ii. ADR-0002's staging table records the split.
- **Hand-rolled engine, stage 2b-ii: path building and RFC 5280 §6.1
  validation** (rusty_tls#25). `handrolled::path` finds and validates a chain
  from a peer's certificate to a trust anchor: name chaining, signatures at
  every link, validity periods, `basicConstraints` `cA`, `keyUsage`
  `keyCertSign`, `pathLenConstraint`, unhandled critical extensions, and the
  end-entity certificate's extended key usage. Path search is bounded by both
  depth and a total signature-verification budget, since the intermediates are
  attacker-supplied.

  **Still not a complete trust decision**: it does not check that the
  certificate is valid for any particular name. Hostname and IP matching is
  stage 2b-iii. Name constraints are also unimplemented, and are fail-closed
  by construction — `nameConstraints` is critical, unknown critical extensions
  are refused, so a name-constrained intermediate is rejected rather than
  having its constraint ignored.
- **Hand-rolled engine, stage 2b-iii: name matching and name constraints**
  (rusty_tls#25). `handrolled::name` matches a server name against a
  certificate's `subjectAltName` — DNS names with single-label wildcards, and
  IP addresses by octet — and enforces RFC 5280 §4.2.1.10 name constraints
  across a path. `path::verify_peer_certificate` combines path validation and
  name matching in one call, because the two are easy to separate and
  disastrous to separate.

  There is **no Common Name fallback**, ever: a certificate with no
  `subjectAltName` matches nothing. Partial wildcards, wildcards outside the
  leftmost label, whole-TLD wildcards, and names containing a NUL
  (CVE-2009-2408) are all refused.

  Name constraints are now enforced rather than being fail-closed by accident
  of the unknown-critical-extension rule. A constraint type this
  implementation cannot evaluate — `directoryName`, `rfc822Name`, URI — is an
  error rather than a skipped entry, because recognising an extension without
  enforcing it is worse than not recognising it. `TrustAnchor` carries its own
  constraints, so a constrained root stays constrained.

  With this, path validation is complete.
- **Hand-rolled engine, stage 3a: the TLS 1.3 key schedule** (rusty_tls#25).
  `handrolled::schedule` implements RFC 8446 §7.1 — `HKDF-Expand-Label`,
  `Derive-Secret`, the early/handshake/master secret schedule, traffic key and
  IV derivation, `finished_key`, the Finished MAC with constant-time
  verification, and the key-update step. SHA-256 and SHA-384.

  Checked against RFC 8448's published intermediate values rather than by
  round trip: every extracted secret, every `"derived"` step, every traffic
  secret, and the traffic keys the record-layer suite was independently
  verified against — so the two suites now meet in the middle, and keys
  derived here decrypt the RFC's own wire record through the hand-rolled
  record layer.

  This is arithmetic over byte strings only. The handshake messages, the
  transcript's accumulation, the key exchange, and anything that talks to a
  peer are stages 3b and 3c.
- **Hand-rolled engine, stage 3b: handshake messages and the transcript hash**
  (rusty_tls#25). `handrolled::wire` implements TLS's presentation language
  (RFC 8446 §3) — fixed-width integers and length-prefixed vectors, read
  strictly and written through closures that backfill lengths, so a prefix
  cannot disagree with what follows it. `handrolled::handshake` implements the
  messages a TLS 1.3 client sends and receives: the handshake header,
  ClientHello, ServerHello (including HelloRetryRequest detection),
  EncryptedExtensions, Certificate, CertificateVerify, Finished, the
  extensions block with duplicates refused, the CertificateVerify signed
  content, and `Transcript`.

  Parsing and encoding are required to be inverses on the RFC's own bytes.
  That is a correctness property, not a tidiness one: the transcript hash
  covers encoded messages, so a parser and encoder that disagree compute a
  transcript the peer does not share. `Transcript` accordingly accepts encoded
  bytes and never a parsed message.

  This closes a gap stage 3a landed with and documented: the server's Finished
  `verify_data` could not be asserted, because RFC 8448 publishes no labelled
  value for the transcript hash it covers. With the messages parseable that
  hash is computable, and the MAC over it matches the RFC — which requires the
  transcript, the key schedule, and the Finished MAC to be simultaneously
  correct.

  Still no state machine, no key exchange, and nothing that decides whether a
  handshake should proceed. Those are stage 3c.
- **Handshake fuzzing** in `tests/handrolled_fuzz.rs`, seeded from RFC 8448's
  exchange, asserting that anything accepted re-encodes to the bytes accepted
  and that the message spans tile their input exactly. It independently caught
  six of the eleven mutants in stage 3b's mutation run.
- **`tests/handrolled_wire.rs`**, which pins the distinction between a
  truncated stream and a length prefix claiming more than its container holds.
  Message-level tests cannot: the two are refused either way, and only the
  error differs.
- **Hand-rolled engine, stage 3c-i: key exchange and handshake signatures**
  (rusty_tls#25). `handrolled::kx` does ephemeral key exchange over X25519,
  P-256, and P-384. A key is used exactly once — `agree` takes `self` by value
  — and the agreed secret is handed to a closure rather than returned, so no
  long-lived copy exists for this crate to promise (and fail) to erase.
- **The TLS `SignatureScheme` namespace** in `handrolled::verify`, alongside
  the X.509 `AlgorithmIdentifier` namespace it has carried since 2b-i, plus
  `verify_tls13_signature` for checking a CertificateVerify.

  **RSASSA-PSS is now supported here**, which matters because RFC 8446 §4.4.3
  *requires* it for an RSA handshake signature — until this stage the engine
  could not have verified a single real RSA handshake. It remains refused in
  the certificate namespace, and that is not an inconsistency: X.509 carries
  the PSS hash, MGF, salt length, and trailer as parameters that can be
  misread, while a TLS scheme number fixes all four.

  The two namespaces are separate types because they disagree. Most sharply:
  an X.509 ECDSA identifier names only a hash, so the curve comes from the key
  (the correction real certificates forced on 2b-i), while a TLS scheme names
  both, so a P-384 key under `ecdsa_secp256r1_sha256` is refused.

  Also refused: `rsa_pkcs1_*` for handshake signatures, with its own error
  variant rather than "unsupported" — the algorithm works and is being turned
  away on the RFC's instruction, and a reader who mistook that for a gap might
  helpfully close it.
- **Verifier fuzzing.** Every one of the 65 536 `SignatureScheme` values is
  walked against a real key with a random signature, and none verifies; mutated
  certificates never make a signature verify; random key shares never panic.

- **Hand-rolled engine, stage 3c-ii: the TLS 1.3 client** (rusty_tls#25).
  `handrolled::client` is a sans-IO client handshake: `ClientHandshake` takes
  one record at a time and returns the bytes to send back, and `Connection`
  carries application data afterwards. It builds the ClientHello, processes
  the ServerHello, derives every key, validates the peer's chain, name, and
  handshake signature, checks the server's Finished, sends its own, and
  handles HelloRetryRequest including the transcript substitution RFC 8446
  §4.4.1 requires.

  **This is the first part of the hand-rolled engine that interoperates.** It
  completes handshakes against a real `rustls` server across every offered
  cipher suite and key-exchange group, including a HelloRetryRequest, and
  carries application data both ways.

  Deliberately refused rather than half-implemented: client certificates
  (a CertificateRequest ends the handshake), session resumption, PSK, 0-RTT,
  and anything below TLS 1.3. Post-handshake NewSessionTicket and KeyUpdate
  are handled — the first discarded, the second answered and rekeyed.

  Still behind both gates, and `rustls` remains the engine behind every
  exported type. Nothing in the public API routes through this.
- `handshake::complete_prefix`, which reports how many leading bytes of a
  buffer form whole messages. `messages` requires its input to end on a
  boundary, which is right for a finished buffer and wrong for one still
  filling up.

### Changed
- **`handrolled::verify` now checks a key's `AlgorithmIdentifier` parameters
  on the TLS path too.** Previously only the certificate path did, which meant
  a leaf's own key — the one a CertificateVerify is checked against, and which
  path validation never inspects — was held to a lower standard than the CA
  above it.
- The parameter rules ("`NULL` or absent" for RSA, "absent" for Ed25519 and
  ECDSA identifiers) are now one implementation used by both namespaces rather
  than two inline copies.
### Fixed
- **An infinite loop in `handrolled::x509::ExtendedKeyUsage`'s iterator**,
  found by the new fuzzer. `Reader::read` does not consume a value whose tag
  is wrong — that is what makes `OPTIONAL` fields work — so an
  `extendedKeyUsage` extension containing anything that is not an `OBJECT
  IDENTIFIER` yielded the same error forever and the iterator never returned
  `None`. A denial of service reachable from any certificate a peer sends.
  Both this iterator and `GeneralNames` now stop when a failed read leaves the
  cursor where it was. Regression tests added.

  Reachable only with the `handrolled-engine` feature *and*
  `--cfg rusty_tls_handrolled`, so no released configuration was affected.

### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
