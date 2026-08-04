# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]

## [0.8.0] - 2026-08-04

Built and tested against the same sibling revs as 0.2.x–0.7.0. Neither moved.

`y` under §2: `handshake` and `client` both gained public surface.

### Added
- **A handshake actually resumes** (rusty_tls#43, client side). This is the
  measurement the last three versions were missing.
  - `handshake::BinderHello` encodes a ClientHello in two phases: build it with
    zeroed binder placeholders, hash the truncated prefix, splice the real
    binders in. `ClientHello::encode` is unchanged, and the bytes a binder
    covers are a *literal prefix* of the message that is sent rather than a
    second serialisation that has to be kept in agreement with the first.
  - `handshake::PresharedKeyOffer` parses the offer from the other direction,
    and `truncated` enforces "`pre_shared_key` is the last extension" by
    re-encoding the binder block and requiring it to be the tail of the message
    — so anything at all after the offer is refused rather than left uncovered.
  - `ClientConfig::resumption` takes a `Resumption { session, age_ms }` and
    offers it as a `pre_shared_key`. `Connection::resumed()` reports whether
    the server accepted it.
  - `Session` now carries `peer_certificates` from the handshake it came out
    of, so `Connection::peer_certificates()` on a resumed connection answers
    with the chain the peer was actually validated on instead of nothing.

### Changed
- **`ClientConfig` gained a required `resumption` field.** There is no
  `Default`, so this is a breaking change for every caller — `resumption: None`
  restores the previous behaviour exactly.

### Security
- **`early_data` is refused, not ignored** (ADR-0003). A server that sends the
  extension in EncryptedExtensions gets `ClientError::UnexpectedEarlyData`.
  This client never offers early data, so accepting the extension would be
  agreeing to a replay property nothing here implements.
- **A `pre_shared_key` in a ServerHello that was never offered is refused**, as
  is a `selected_identity` past the end of the offer, and a selected cipher
  suite whose hash is not the PSK's.
- **A CertificateRequest in a resumed handshake is refused** (§4.4.2). Signing
  over a transcript in a handshake where the server proved nothing about its own
  identity is not something to do on request.

**What this finally verifies.** The key material from 0.6.0 and 0.7.0 was tested
for shape and not for value — the issue measured that a `"res binder"` →
`"ext binder"` swap passed all five binder tests. A `rustls` server accepting a
resumption checks four separate things at once, because it computes them
independently: the `"resumption"` expansion, the `res master` transcript point,
the `"res binder"` label, and the truncation point. All four were mutated and
all four now fail the resumption test.
`rustls_refuses_a_corrupted_binder` is what makes that evidence rather than
coincidence — without it, the positive test would pass even if `rustls` ignored
binders entirely.

**Known limitations, stated rather than implied:**
- **The server half still does not resume.** It parses no `pre_shared_key`,
  verifies no binder, and issues no NewSessionTicket. **#43 stays open** for
  that and for the ticket-sealing key ADR-0003 flags as being as sensitive as
  the certificate's private key.
- **`obfuscated_ticket_age` is pinned by arithmetic, not by a peer.** A server
  uses it for 0-RTT anti-replay and for nothing else, so `rustls` accepts a
  1-RTT resumption whatever it says — measured: the mutation zeroing `age_add`
  survives every interop test here.
  `the_offer_carries_the_obfuscated_ticket_age` checks the formula directly and
  is a regression guard rather than an interop result.
- **Only one identity is ever offered**, so `selected_identity` is only ever
  accepted as `0`.

## [0.7.0] - 2026-08-03

`y` under §2: `schedule` gained two public functions.

### Added
- **PSK binder derivation** (rusty_tls#43, stage three, partial).
  `schedule::binder_key` computes `Derive-Secret(Early-Secret(psk),
  "res binder", "")` — the one place the key schedule's first step differs
  between a fresh handshake and a resumed one, because the early secret is
  extracted from the PSK rather than from zeroes. `schedule::psk_binder`
  applies the Finished construction over the truncated ClientHello.

**Known limitation, and it is a real one.** The tests cover the *shape* of
these functions — deterministic, dependent on both the PSK and the transcript,
the right length, not the PSK passed through. **They do not pin the values:**
a mutation swapping `"res binder"` for `"ext binder"` passes all five. Checking
the value needs RFC 8448's resumption vectors or a handshake that actually
resumes, and neither exists here yet.

Nothing calls these functions in the handshake path. They are covered by tests
rather than left unreachable, but no ClientHello offers a `pre_shared_key` and
no server accepts one, so **still nothing resumes.** #43 stays open for the
offer path, the two-phase ClientHello encoding the binder requires, and the
server side.

## [0.6.0] - 2026-08-03

`y` under §2: `Incoming` gained a variant and `Session` is new public surface.

### Added
- **Session tickets are kept, not discarded** (rusty_tls#43, stage two). A
  NewSessionTicket now arrives as `Incoming::Ticket(Box<Session>)` rather than
  `Incoming::Handled`. The `Session` carries the ticket, its lifetime, the
  cipher suite it belongs to, and the PSK derived from it —
  `HKDF-Expand-Label(res master, "resumption", nonce, Hash.length)` per §4.6.1.
  - `res master` is derived over the transcript through the **client's**
    Finished, which is why it can only be computed once that message exists.
  - `Session`'s `Debug` redacts the key, and the key is behind `psk()` rather
    than a public field: a struct with one public secret and one private one
    reads as an oversight rather than a decision.
  - A ticket arriving in the same fragment as a KeyUpdate that needs a reply is
    refused rather than silently dropped. Losing a session key quietly surfaces
    much later as "resumption never works".

**Known limitation, stated rather than implied:** nothing offers the PSK back
yet, so **no handshake is resumed, and the derived key's value is verified by
nothing.** The tests show a key of the correct length from the correct inputs;
only a completed resumption proves the transcript point and the expansion are
right. #43 stays open for that.

## [0.5.0] - 2026-08-03

Built and tested against the same sibling revs as 0.2.x–0.4.0. Neither moved.

`y` under `docs/versioning.md` §2: `handshake::extension` gained three public
constants. Additive, but additive is still `y` at `0.y.z`.

### Added
- **ADR-0003: session resumption without 0-RTT.** Records the decision that
  resumption is in scope and **early data is not**, until a named consumer asks
  and an anti-replay design lands in a follow-up ADR. Early data is replay-safe
  only if the application above it is, and TLS cannot tell a request that reads
  from one that charges a card.
- **The client offers `psk_key_exchange_modes`** (rusty_tls#43, stage one).
  RFC 8446 §4.2.9 means a conforming server sends no NewSessionTicket unless
  the client asks — so until now the client's ticket-handling branch **could
  not be reached at all**, and the test that claimed to cover it was green and
  vacuous. Measured, not assumed: a `rustls` server now sends one ticket where
  it previously sent none, and removing the extension makes that test fail.
- `extension::PRE_SHARED_KEY`, `extension::EARLY_DATA`, and
  `extension::PSK_KEY_EXCHANGE_MODES`.

Only `psk_dhe_ke` is offered, never `psk_ke`: resuming without fresh key
material trades forward secrecy for one saved key exchange, which is a bad
trade in an engine that is not the default and never will be.

**#43 is not closed by this.** Offering a PSK on a later connection, binders
over the truncated ClientHello, and the server side of both remain.

## [0.4.0] - 2026-08-03

Built and tested against the same sibling revs as 0.2.x and 0.3.0:
`rusty_tokio` rev `6d3bb05a45a393e4cf902013b05189dd168f6106` and `rustils` rev
`93b00ce964284d93ea6cec2581b3543f08df8f2d`. Neither moved.

`y` under `docs/versioning.md` §2, and this one is genuinely breaking rather
than merely additive: `ClientConfig` and `ServerConfig` each gained a field, so
every construction of them needs updating, and `ClientError` lost a variant.

### Added
- **Client certificates, both halves** (rusty_tls#42). The hand-rolled client
  can authenticate itself, and the hand-rolled server can require it.
  - `ClientConfig::identity` takes a `ClientIdentity` — a chain and the key for
    it. With one configured, a CertificateRequest is answered with a
    Certificate and a CertificateVerify signed using RFC 8446 §4.4.3's
    **client** context string, which differs from the server's so a signature
    made in one direction can never be replayed as the other.
  - With no identity, or none matching the schemes the server named, the answer
    is an empty Certificate. §4.4.2 makes that the conforming way to say "I
    have nothing", and it leaves the accept-or-refuse decision with the server.
    The client no longer aborts on a CertificateRequest.
  - `ServerConfig::client_auth` takes a `ClientAuth` — anchors, path options,
    and an explicit `required` flag. The server checks **two** things about the
    answer: that the chain validates, and that the CertificateVerify was made
    by the key in the leaf. A certificate proves nothing on its own.
  - No name is matched against a client certificate: a client is not a
    hostname. `Connection::peer_certificates` reports what arrived, so the
    application can decide who it is.
  - New: `AlertDescription::CERTIFICATE_REQUIRED`,
    `handshake::CertificateRequestMessage`,
    `HandshakeError::MissingSignatureAlgorithms`, and four `ServerError`
    variants for the ways a client's certificate can be refused.

### Removed
- `ClientError::ClientCertificateRequested`. It existed to report that client
  certificates were unimplemented, and is unreachable now that they are.

### Changed
- A CertificateRequest with no `signature_algorithms` is now refused as
  malformed (§4.3.2 requires it). Previously every CertificateRequest was
  refused, so this is narrower rather than newly strict.

## [0.3.0] - 2026-08-03

Built and tested against the same sibling revs as 0.2.x: `rusty_tokio` rev
`6d3bb05a45a393e4cf902013b05189dd168f6106` and `rustils` rev
`93b00ce964284d93ea6cec2581b3543f08df8f2d`. Neither moved.

`y` rather than `z` under `docs/versioning.md` §2: `ServerError` gained two
variants, and `ServerError::NoSharedGroup` now means something narrower than it
did. Both are behind the `handrolled-engine` gate, and gated items are still
public API — see `docs/versioning.md`.

### Added
- **The hand-rolled server generates HelloRetryRequest** (rusty_tls#44). A
  client that supports a group this server does but sent no `key_share` for it
  is now asked to try again, rather than refused with `handshake_failure`. The
  refusal was correct but was the wrong answer to the question asked: it turned
  away a client that would have completed after one extra round trip, for a
  reason that was not its fault.
  - The post-retry transcript uses RFC 8446 §4.4.1's synthetic `message_hash`
    substitution, so the server does not retain ClientHello1 across the round
    trip. The client half has done this since #25; the two now agree.
  - §4.1.4's "never a second retry" is enforced: a client that comes back
    without the share it was asked for is refused, not asked again.
  - §4.1.2 is checked **in part**: the second hello must carry the same
    `random` and `legacy_session_id`, still offer the negotiated cipher suite
    and TLS 1.3, and now carry a share for the requested group. It is not
    diffed field by field, because that means retaining the first hello — the
    thing the `message_hash` substitution exists to avoid.
  - `ServerError::RetriedHelloStillHasNoShare` and
    `ServerError::RetriedHelloChangedIdentity` are new; the latter sends
    `illegal_parameter` rather than `handshake_failure`, which tells the client
    which of its two hellos to look at.
- **The hand-rolled server is now driven over a real socket by OpenSSL**
  (rusty_tls#45). `tests/handrolled_socket_interop.rs` binds a loopback
  listener, runs `ServerHandshake` over it, and points `openssl s_client` at
  it. Until now every server test was `rustls`-the-client, in memory, with
  whole records handed across as byte slices — one implementation rather than
  an independent one, and no socket.
  - OpenSSL **verifies the chain**: `-CAfile` with the generated root,
    `-verify_return_error` so a bad chain is a non-zero exit rather than a
    printed warning, and `-verify_hostname` so the certificate must be for the
    name asked for rather than the address dialled. Without those three,
    "it connected" would stand in for "it was trusted".
  - Covers what an in-memory harness cannot: a record's header and body
    arriving in separate reads, application data both ways over a socket, and
    `close_notify` from an independent peer seen as an orderly close.
  - `#[ignore]`d because the `openssl` binary cannot be assumed present — but
    **CI runs them explicitly**, since the runner has it and the tests are
    otherwise hermetic. `#[ignore]` here means "not everywhere", not "never".
    The step asserts a non-zero pass count rather than trusting the exit code:
    `-- --ignored` runs *only* ignored tests, so removing the attribute would
    otherwise leave a green step that ran nothing.

### Changed
- `ServerError::NoSharedGroup` now means the client named no group this server
  implements, rather than "sent no usable share". The latter is a retry, and
  telling the two apart is the whole of #44.
- The hermetic rejection cases are now **one table run by two drivers** rather
  than two hand-written suites that happened to agree (rusty_tls#46). The cases
  live in `tests/rejection/mod.rs`; `handshake.rs` drives them through `rustls`
  and `handrolled_client.rs` through the hand-rolled client, against
  byte-identical certificates. Adding a row now covers both engines by
  construction instead of by someone remembering the other file.
  - The expectation is recorded **per engine**, not shared, because there are
    places the hand-rolled engine is deliberately stricter than `rustls`. A
    divergence must carry a written reason; the table refuses one that does
    not. Every row agrees today — the structure is what stops the first
    disagreement from being resolved by deleting the row.
  - A `accepts_a_good_chain` control row was added. A rejection table with no
    accepting row is passed by a driver that fails everything, including one
    broken before it reaches the certificate.
  - Expiry is now expressed in the certificate rather than by moving the
    client's clock, because only one of the two engines has an injectable
    clock. It is the only form of the case that means the same thing to both.
  - Rejections still assert `ClientError::Path(_)` on the hand-rolled side, so
    consolidating did not weaken "refused" into "refused for any reason".

### Fixed
- Documentation that assumed `v0.2.0` would be tagged. It was not — the first
  published tag is `v0.2.1`. `RELEASE_NOTES.md` now says so under both entries,
  and `docs/versioning.md`'s "pin by tag" example pointed at `tag = "v0.2.0"`,
  which a consumer could have copied into a `Cargo.toml` that would not
  resolve. Docs only; no source change, so no version bump under §2.

## [0.2.1] - 2026-08-03

Built and tested against the same sibling revs as 0.2.0: `rusty_tokio` rev
`6d3bb05a45a393e4cf902013b05189dd168f6106` and `rustils` rev
`93b00ce964284d93ea6cec2581b3543f08df8f2d`. Neither moved.

`z` rather than `y` under `docs/versioning.md` §2: no public item changed
shape. The only source edits are doc comments.

### Fixed
- Two doc comments in the `handrolled` module linked to private items
  (`Expect`, `SignatureScheme::tls13_algorithm`). rustdoc does not render those
  as links, so both read as a cross-reference that silently is not one. They
  now point at what a reader can actually reach — `TLS13_SUPPORTED` for the
  second — or name the item as private rather than appearing to link it.

### Added
- CI builds the documentation with `-D warnings`, in both the default job and
  the `handrolled-engine` one. `cargo test --doc` runs doctests but never
  builds the docs, so rustdoc's own lints had no job that could fail on them —
  which is how the two links above survived thirteen pull requests. The
  handrolled job documents with `--all-features`, because the crate's prose
  links to items behind `rusty-tokio` and a partial-feature doc build reports
  unresolved links that say nothing about the module under test.

## [0.2.0] - 2026-08-03

Built and tested against `rusty_tokio` rev `6d3bb05a45a393e4cf902013b05189dd168f6106`
and `rustils` (`platform`, `platform-linux`, `platform-windows`, `platform-bsd`) rev
`93b00ce964284d93ea6cec2581b3543f08df8f2d`. See `docs/versioning.md` for why those
revs are part of the release rather than an implementation detail.

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
- **Hand-rolled engine, stage 5: the TLS 1.3 server** (rusty_tls#25).
  `handrolled::sign` produces handshake signatures — the first thing in the
  module that holds a private key rather than only checking one — and
  `handrolled::server` is the sans-IO server handshake, the mirror of
  `handrolled::client`. A real `rustls` client completes handshakes against it
  with ECDSA P-256, P-384, and Ed25519 server keys.

  Not supported, and refused rather than half-built: client certificates,
  session resumption, tickets, 0-RTT, TLS 1.2, and HelloRetryRequest
  generation.
- **Fixed:** the client did not check that a ServerHello echoed the
  `legacy_session_id` it sent, which RFC 8446 §4.1.3 requires. Found by
  mutating the *server* and watching this crate's own client accept the result.
- **Fixed:** the client's HelloRetryRequest path built a second ClientHello
  with a fresh `random` and session id. RFC 8446 §4.1.2 enumerates what a
  retried hello may change and neither is on the list.
- **Hand-rolled engine, stage 4a: the version boundary** (rusty_tls#25). The
  client now parses TLS alerts wherever they can arrive — in the clear before
  the ServerHello, inside the protected flight, and after the handshake — and
  reports them as `ClientError::PeerAlert` with the peer's own level and
  description.

  This exists because a TLS 1.2-only server answers a TLS 1.3-only ClientHello
  with a fatal `protocol_version` alert, and the client used to report that as
  `UnexpectedContentType(Alert)`: a correct refusal that discarded the only
  information distinguishing "this server is too old" from "something broke".

  Also adds the RFC 8446 §4.1.3 downgrade sentinel check, which separates a
  genuinely old server from an active downgrade. Both are refused; the sentinel
  decides only which error is returned.
- **Fixed:** an orderly `close_notify` was reported as a failure, because
  alerts were not parsed. It is now `Incoming::Closed`. The interop suite had a
  comment calling the old behaviour "the correct place to stop" — a missing
  feature described as correct behaviour.
- **Interop over a real socket** (`tests/handrolled_interop.rs`). The client
  completes TLS 1.3 handshakes against servers nobody here configured, using
  the machine's own trust store, and fetches real HTTP responses. `#[ignore]`d
  rather than gated on an environment variable, because a gated test that
  passes when the variable is unset reports `ok` for a run that did nothing.

  Two limits stated in the suite itself: where egress is intercepted the peer
  is a gateway rather than the host named, so the issuer is printed on every
  run; and because a gateway mints a certificate for whatever SNI it is
  handed, "connect under the wrong name and watch it be refused" cannot be
  written to pass in both environments. The checkable direction — every
  accepted certificate carries the name that was asked for — is asserted
  instead.
- **A hermetic test that a handshake flight split across records is
  reassembled**, down to one octet per record. This closes coverage that was
  assumed and absent: `rustls` sends its whole flight in one record, and so
  does every server the interop suite reaches, so `complete_prefix` and the
  client's reassembly buffer had never done any work.
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
