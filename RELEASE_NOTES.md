# Release Notes

<!--
Two variants, pick the one that fits this repo's actual unit of change:

1. No version tags yet (pre-1.0, nothing published) — track by PR instead, same way
   AISF does it: one entry per merged PR against main, reverse chronological, each
   linking to its PR and (where one exists) to the doc that covers the change in full
   detail. Use "## PR #N — <summary>" headers.

2. Actual version tags exist — use "## vX.Y.Z - YYYY-MM-DD" headers instead, each
   linking to the PRs it shipped and a compare link to the previous tag. Add an
   "### Upgrade notes" subsection under any entry with a breaking change.

Either way, keep the tone AISF's file uses: bolded category tags inline in the
bullet (**Added:** / **Changed:** / **Fixed:**), not separate subheaders per
category — and state known limitations or deliberate scope cuts plainly instead of
leaving them implied.
-->

Tracked by PR against main, reverse chronological, one entry per merged PR,
grouped under the version that shipped them.

---

## v0.9.0
**2026-08-04**

**Built and tested against:** unchanged from v0.2.x.

- **Added:** the server half resumes (#43) — `ServerConfig::tickets` makes this
  server issue NewSessionTickets and accept them back as a `pre_shared_key`.
- **Added:** `handrolled::ticket` — `TicketKey`, `TicketKeys`, `TicketContents`.
  Resumption is stateless, so the server holds a sealing key instead of a
  session table. **That key is as sensitive as the certificate's private key**
  and needs rotating; `TicketKeys` carries retired keys so rotation is not an
  outage.
- **Changed:** `Incoming::Ticket(Box<Session>)` is now
  `Incoming::Tickets(Vec<Session>)`. A server routinely sends several tickets in
  one record and the old variant kept only the last.
- **Security:** a recognised identity with a bad binder aborts rather than
  falling back; `early_data` and a misplaced `pre_shared_key` are refused;
  tickets are ignored when they do not open, have expired, belong to another
  suite, or were issued under a different certificate chain.

**Measured, not assumed.** Twenty-five mutations across both halves, each
asserted to have applied before its result was believed, and each killed by a
named test. Four survived their first run — a reusable ticket nonce, a droppable
cipher-suite binding, removable sealing associated data, and a deletable
PSK-hash check — and are why three tests and one behaviour change exist.

### Upgrade notes

- **`ServerConfig` gained a required `tickets` field.** `tickets: None` is the
  previous behaviour exactly.
- **`Incoming::Ticket` is gone; match `Incoming::Tickets(sessions)`** and take
  what you want from the `Vec`, which is never empty.

**Known limitations:** resumption and client authentication do not combine — with
`client_auth` configured, a `pre_shared_key` offer gets a full handshake,
because these tickets carry no client identity. And `obfuscated_ticket_age` is
still unchecked by this server; it exists for 0-RTT anti-replay, which ADR-0003
puts out of scope.

## v0.8.0
**2026-08-04**

**Built and tested against:** unchanged from v0.2.x.

- **Added:** a handshake actually resumes (#43, client side) — two-phase
  ClientHello encoding (`handshake::BinderHello`), a `pre_shared_key` offer via
  `ClientConfig::resumption`, and the resumed route through the key schedule.
- **Added:** `Connection::resumed()`, and `Session::peer_certificates` so a
  resumed connection still reports the chain its peer was validated on.
- **Security:** `early_data` is refused rather than ignored; an unoffered
  `pre_shared_key`, a bad `selected_identity`, a suite/PSK hash mismatch, and a
  CertificateRequest in a resumed handshake are all refused.

**What this verifies that nothing before it did.** The PSK derivation from
v0.6.0 and the binder derivation from v0.7.0 were tested for shape, not value —
swapping `"res binder"` for `"ext binder"` passed every one of those tests. A
`rustls` server accepting a resumption checks the `"resumption"` expansion, the
`res master` transcript point, the `"res binder"` label, and the truncation
point, all independently. Each was mutated; each now fails.

### Upgrade notes

- **`ClientConfig` gained a required `resumption` field.** There is no
  `Default` for it, so every construction needs updating; `resumption: None`
  is exactly the old behaviour.

**Known limitations:** the **server half still does not resume** — no
`pre_shared_key` parsing, no binder verification, no NewSessionTicket issuance —
so **#43 remains open**. And `obfuscated_ticket_age` is pinned by arithmetic
rather than by a peer: `rustls` accepts a 1-RTT resumption whatever the age
says, measured by a mutation that zeroes `age_add` and survives every interop
test here.

## v0.7.0
**2026-08-03**

- **Added:** PSK binder derivation (#43, stage three, partial) —
  `schedule::binder_key` and `schedule::psk_binder`.

**Known limitation:** the tests check the shape of these functions, not their
values — a mutation swapping the `"res binder"` label for `"ext binder"` passes
all of them. Only RFC 8448's resumption vectors or a real resumption can check
the value, and neither exists here yet. Nothing calls these in the handshake
path, so **still nothing resumes.** #43 remains open.

## v0.6.0
**2026-08-03**

**Built and tested against:** unchanged from v0.2.x.

- **Added:** session tickets are kept rather than discarded (#43, stage two). A
  NewSessionTicket arrives as `Incoming::Ticket(Box<Session>)`, and the
  `Session` carries the PSK derived from it.

### Upgrade notes

- **`Incoming::Handled` no longer covers NewSessionTicket.** A caller matching
  on `Handled` to mean "a ticket arrived" now needs `Ticket(_)`. `Incoming` is
  `#[non_exhaustive]`, so a wildcard arm already handles it.

**Known limitation, stated rather than implied:** nothing offers the PSK back
yet, so **no handshake is resumed, and the derived key's value is verified by
nothing.** The tests show a key of the right length from the right inputs; only
a completed resumption proves the `res master` transcript point and the
`"resumption"` expansion are correct. #43 stays open for that.

## v0.5.0
**2026-08-03**

**Built and tested against:** unchanged from v0.2.x.

- **Added:** ADR-0003 — session resumption is in scope, 0-RTT is not, and the
  reasoning is written down rather than left implied.
- **Added:** the client offers `psk_key_exchange_modes` (#43, stage one), which
  makes the ticket path reachable for the first time. A `rustls` server now
  sends a ticket where it previously sent none.

**This does not close #43.** What lands here is the half that makes tickets
*arrive*; offering a PSK on a later connection — binders over the truncated
ClientHello — and the server side of both are still to come. Said plainly
because a version that claimed resumption and delivered half of it would be
worse than one that claims less.

## v0.4.0
**2026-08-03**

**Built and tested against:** unchanged from v0.2.x — `rusty_tokio` rev
`6d3bb05a45a393e4cf902013b05189dd168f6106`, `rustils` rev
`93b00ce964284d93ea6cec2581b3543f08df8f2d`.

- **Added:** client certificates, both halves (#42). This crate's client
  authenticates itself to a `rustls` server, and this crate's server verifies a
  `rustls` client — each direction checked by the implementation that did not
  produce the bytes.

### Upgrade notes

- **Breaking:** `ClientConfig` gained `identity` and `ServerConfig` gained
  `client_auth`. Both take `None` for the previous behaviour, but every
  construction needs the field. `ClientError::ClientCertificateRequested` is
  removed — it reported an unimplemented feature that now exists.
- Both types are behind `handrolled-engine` *and* `--cfg
  rusty_tls_handrolled`, so nothing a consumer of the shipped API touches has
  changed.

**Known limitations, stated rather than implied:** the client ignores a
CertificateRequest's `certificate_authorities` and `oid_filters` — it sends the
chain it was configured with, so a server wanting a different one refuses with
a worse error message than it could. Post-handshake authentication is not
supported. Still absent entirely: session resumption (#43) and TLS 1.2 (#41).

## v0.3.0
**2026-08-03**

**Built and tested against:** the same sibling revs as v0.2.x — `rusty_tokio`
rev `6d3bb05a45a393e4cf902013b05189dd168f6106`, `rustils` rev
`93b00ce964284d93ea6cec2581b3543f08df8f2d`. Neither moved.

- **Added:** the hand-rolled server generates HelloRetryRequest (#44). A client
  that supports a group this server does, but guessed wrong about which to send
  a `key_share` for, is now asked to try again instead of being refused. Both
  `rustls` and this crate's own client complete handshakes through it.
- **Added:** the hand-rolled server is driven over a real socket by
  `openssl s_client`, verifying the chain (#45) — the first evidence about this
  server from a stack with no shared provenance.
- **Changed:** the hermetic rejection cases are one table run against both
  engines rather than two suites that happened to agree (#46).
- **Fixed:** documentation that assumed a `v0.2.0` tag, which was never cut.

### Upgrade notes

- **`ServerError` gained two variants** and `NoSharedGroup` narrowed in
  meaning. Everything here is behind `handrolled-engine` *and*
  `--cfg rusty_tls_handrolled`, so no consumer of the shipped API is affected —
  but gated items are still public API, which is why this is `y` and not `z`.

**Known limitations, stated rather than implied:** §4.1.2 is checked in part,
not in full — the retried ClientHello is not diffed field by field against the
first, because that would mean retaining it across the round trip and §4.4.1's
`message_hash` substitution exists so a server need not. And still absent:
client certificates (#42), session resumption (#43), and TLS 1.2 (#41).

## v0.2.1
**2026-08-03**

**The first tagged release**, so there is no previous tag to compare against —
later entries carry a compare link. `0.1.0` was the decorative default the
`Cargo.toml` carried from the start, and `0.2.0` was a real version that was
never tagged (see its entry below), so this is the first ref anything can pin
to.

**Built and tested against:** the same sibling revs as `0.2.0` — `rusty_tokio`
rev `6d3bb05a45a393e4cf902013b05189dd168f6106`, `rustils` rev
`93b00ce964284d93ea6cec2581b3543f08df8f2d`. Neither moved.

- **Fixed:** two doc comments in the `handrolled` module linked to private
  items (`Expect`, `SignatureScheme::tls13_algorithm`). rustdoc renders those
  as plain text, so each read as a cross-reference that silently was not one.
- **Added:** CI now builds the documentation with `-D warnings`. `cargo test
  --doc` runs doctests but never builds the docs, which is why nothing could
  fail on a rustdoc lint — the two links above survived thirteen pull requests
  for that reason and no other.

**Known limitation, unchanged from v0.2.0:** a doc build with a partial feature
set still reports unresolved links, because the crate's prose links to items
behind `rusty-tokio` from text that is not itself feature-gated. CI documents
with `--all-features`, which is the configuration docs.rs uses and the one
these docs are written for. Making the prose resolve under every feature
combination would mean splitting paragraphs across `cfg_attr`, which costs more
in readability than it buys.

## v0.2.0
**2026-08-03**

**Never tagged.** This entry describes a version that exists in the history and
in `Cargo.toml`, but no `v0.2.0` ref was ever published — the tag was cut on
v0.2.1 instead. Nothing can pin to it; anything that would have is served by
v0.2.1, which is a superset. Everything below this heading, down to the next
version heading, shipped in the `0.2.0` version of `Cargo.toml`.

**Built and tested against:** `rusty_tokio` rev
`6d3bb05a45a393e4cf902013b05189dd168f6106`, `rustils` rev
`93b00ce964284d93ea6cec2581b3543f08df8f2d`.

That pairing is part of what this version means rather than an implementation
detail. `AsyncTlsStream` is generic over `rusty_tokio`'s own `AsyncRead`/
`AsyncWrite`, and Cargo treats two `rev`s of the same git dependency as
unrelated crates — so a consumer that also depends on `rusty_http` must use the
same `rusty_tokio` rev across all of them, or get a type-identity error that
reads like a mistake in their own code. `docs/versioning.md` has the detail.

### Upgrade notes

- **Pin by tag or by rev, never by branch.** Both of the first two carry the
  same guarantee; a branch name silently resolves to something different on
  every fresh `Cargo.lock`.
- **Nothing in the public API changed shape.** The version moves from `0.1.0`
  to `0.2.0` because the crate gained a large amount of new public surface —
  the `handrolled` module — not because anything existing moved. Under the
  ecosystem's `0.y.z` rule additive changes bump `y` too.
- **The hand-rolled engine is not reachable by accident and never will be.**
  It needs both the `handrolled-engine` feature and `--cfg
  rusty_tls_handrolled`. `rustls` remains the engine behind every exported
  type. See `docs/adr/0002-*`.

---

## Hand-rolled TLS engine, stage 5: the TLS 1.3 server
**2026-08-03**

- **Added:** `handrolled::sign` (rusty_tls#25) — producing handshake
  signatures, and the first thing in the engine that holds a private key rather
  than only checking one. The key is never exposed, `Debug` says only what
  algorithm it is, and it refuses to sign with a scheme it cannot produce
  rather than substituting one it can.
- **Added:** `handrolled::server` — the sans-IO TLS 1.3 server handshake, the
  mirror of `handrolled::client`. **A real `rustls` client completes handshakes
  against it** with ECDSA P-256, P-384, and Ed25519 server keys.
- **Fixed:** the client did not check that a ServerHello echoed the
  `legacy_session_id` it sent, which RFC 8446 §4.1.3 requires. This was found
  by mutating the *server* to stop echoing and watching this crate's own client
  accept it — each half is the other's adversary.
- **Fixed:** the client's HelloRetryRequest path built a second ClientHello
  with a fresh `random` and session id. RFC 8446 §4.1.2 lists what a retried
  hello may change and neither is on it. `rustls` accepts the old behaviour;
  no server is obliged to.
- **Known limitation, stated plainly:** no client certificates. The server
  never sends a CertificateRequest, so no client is ever authenticated and
  mutual TLS is not available.
- **Known limitation, stated plainly:** no session resumption, tickets, or
  0-RTT. Resumption is where a server's most interesting state lives, and a
  server that stores nothing cannot be confused about what it stored.
- **Known limitation, stated plainly:** no HelloRetryRequest generation. A
  client whose `key_share` names no group this server supports gets a
  `handshake_failure` alert rather than a retry. A retry is a legitimate answer
  and would be strictly better; it is not implemented.
- **Known limitation, stated plainly:** the server has not been exercised over
  a real socket by clients other than `rustls`, the way the client was in
  `handrolled_interop`. That would mean listening on a port, which is a
  different kind of test than this suite runs.

---

## Hand-rolled TLS engine, stage 4a: the version boundary
**2026-08-03**

- **Added:** TLS alert parsing (rusty_tls#25), wherever an alert can arrive —
  in the clear before the ServerHello, inside the protected flight, and after
  the handshake. A peer's refusal is now reported with the peer's own level and
  description instead of as a surprising content type.
- **Added:** the RFC 8446 §4.1.3 downgrade sentinel check, separating a
  genuinely old server from an active downgrade. Both are refused — the
  sentinel decides only which error is returned — and that is stated in the
  docs rather than dressed up as a gate.
- **Fixed:** an orderly `close_notify` was being reported as a failure, because
  alerts were not parsed at all. It is now `Incoming::Closed`. The interop
  suite had worked around it with a comment calling the old behaviour "the
  correct place to stop", which was a missing feature described as correct
  behaviour.
- **Added:** a TLS 1.2-only `rustls` server as a permanent test of the version
  boundary. It is also the answer to a question stage 4 has to ask.
- **Known limitation, stated plainly — and the reason stage 4b is not done:**
  the issue defines TLS 1.2 as work to do *only if a real peer forces it*, and
  the measurement says none does. Every host reachable from the development
  environment negotiates TLS 1.3 — including `tls-v1-2.badssl.com`, whose whole
  purpose is to be TLS 1.2-only — because they all terminate at an egress
  gateway that speaks 1.3 regardless of the origin. In that environment the
  condition cannot even be observed.
- **Known limitation, stated plainly:** speaking TLS 1.2 would give up a
  property the client has today — it cannot be downgraded, because it cannot
  speak anything to be downgraded to. That is the strongest form of the
  defence and it is free. Replacing it with a negotiated one is a real
  expansion of a security-sensitive surface, and the condition for accepting
  that expansion has not been met. The boundary that would report such a peer
  is now in place.

---

## Hand-rolled TLS engine: interop against real servers
**2026-08-03**

- **Added:** `tests/handrolled_interop.rs` (rusty_tls#25) — the hand-rolled
  client completing TLS 1.3 handshakes over a real socket, against servers
  nobody here configured, using the machine's own trust store, and fetching
  real HTTP responses. This closes the second item on ADR-0002's shipping bar
  for the client.
- **Added:** a blocking transport in about thirty lines, showing what "the
  caller splits the stream into records" actually looks like. It stays in the
  test rather than the library: `handrolled` is not the engine this crate
  ships, so giving it an IO adapter would grow the surface of an experiment
  for no production benefit.
- **Changed:** the tests are `#[ignore]`d rather than gated on an environment
  variable. A gated test that quietly passes when the variable is unset
  reports `ok` for a run that did nothing — which is exactly how a vacuous
  session-ticket test survived a mutation in stage 3c-ii. An ignored test
  reports `ignored`.
- **Fixed:** coverage that was assumed and absent. This work expected real
  servers to split a handshake flight across records, finally exercising the
  client's reassembly buffer — `rustls` sends its flight in one record, so
  `complete_prefix` and the buffer had never done any work. **The measurement
  disagreed**: every server tried also sends one record. The interop suite now
  reports the count rather than asserting a split, and the gap is closed
  hermetically by a test server that splits its flight on purpose, down to one
  octet per record.
- **Known limitation, stated plainly:** where outbound TLS is intercepted, the
  peer is an egress gateway rather than the host named. That is still an
  independent TLS 1.3 implementation and so still a third opinion, but a
  passing run is evidence about *whatever answered* — so the suite prints the
  certificate issuer on every run rather than leaving it to be inferred.
- **Known limitation, stated plainly:** one test somebody would expect to find
  here is absent and cannot be written. An intercepting gateway mints a
  certificate for whatever SNI it is handed, so a client *correctly* accepts
  it, and "connect to a real server under the wrong name and watch it be
  refused" cannot pass in both environments. The direction that is checkable
  everywhere — every accepted certificate carries the name that was asked for
  — is asserted instead, and the refusal direction stays covered hermetically
  where the peer's certificate can be chosen.

---

## Hand-rolled TLS engine, stage 3c-ii: the TLS 1.3 client
**2026-08-03**

- **Added:** `handrolled::client` (rusty_tls#25) — a sans-IO TLS 1.3 client
  handshake. `ClientHandshake::read_record` takes one whole record and returns
  the bytes to send back; `Connection` carries application data afterwards.
  Nothing here opens a socket, for the same reason `rustls::ClientConnection`
  does not: a handshake that owns its IO cannot be driven by a test.
- **Added:** **the engine interoperates.** The client completes handshakes
  against a real `rustls` server across every offered cipher suite and
  key-exchange group, including a HelloRetryRequest, and carries application
  data both ways. Nine stages of parsers and primitives now do something
  end to end.
- **Added:** HelloRetryRequest, including the transcript substitution RFC 8446
  §4.4.1 requires — once a retry happens the running hash starts from a
  synthetic `message_hash` message wrapping `Hash(ClientHello1)` rather than
  from ClientHello1 itself. Invisible until it is wrong, and then it fails
  with an error that looks like corruption rather than a transcript bug.
- **Added:** post-handshake handling. A NewSessionTicket is discarded and a
  KeyUpdate rekeys the receiving direction, answering when one is requested.
- **Added:** a rejection suite driven by a minimal test server, because the
  server's flight arrives inside one AEAD record and cannot be edited without
  its keys. Refused: a flight missing any message, a reordered flight, a
  CertificateVerify over the wrong transcript, a Finished that does not
  verify, an untrusted root, a wrong name, an expired certificate, an
  unoffered cipher suite, a server that does not select TLS 1.3, a second
  HelloRetryRequest, and a mislabelled `key_share`.
- **Added:** `handshake::complete_prefix`, for pulling whole messages out of a
  buffer that is still filling.
- **Known limitation, stated plainly:** client certificates are refused, not
  supported. A CertificateRequest ends the handshake with a specific error.
  Answering with an empty Certificate would be legal and would let it
  continue, but it is a feature this has not built.
- **Known limitation, stated plainly:** no session resumption, PSK, or 0-RTT.
  The ClientHello does not offer `psk_key_exchange_modes`, so by RFC 8446
  §4.2.9 a conforming server will never send a session ticket at all — which
  is why an earlier version of the ticket test passed while exercising
  nothing.
- **Known limitation, stated plainly:** TLS 1.2 and below are refused.
  `supported_versions` offers exactly `0x0304`.
- **Known limitation, stated plainly:** interop is against `rustls` in memory.
  Nothing has yet been tested against a server on the public internet, which
  is where the shipping bar's second item still points.
- Reachable only with the `handrolled-engine` feature *and*
  `--cfg rusty_tls_handrolled`. `rustls` remains the engine behind every
  exported type.

---

## Hand-rolled TLS engine, stage 3c-i: key exchange and handshake signatures
**2026-08-03**

- **Added:** `handrolled::kx` (rusty_tls#25) — ephemeral key exchange over
  X25519, P-256, and P-384. A key is used exactly once, enforced by `agree`
  taking `self` by value; the agreed secret is passed to a closure rather than
  returned, so there is no long-lived copy this crate would have to promise to
  erase and could not (zeroing on `Drop` needs a dependency or `unsafe`, and
  this crate forbids `unsafe`).
- **Added:** the TLS `SignatureScheme` namespace in `handrolled::verify`, and
  `verify_tls13_signature` for checking a CertificateVerify.
- **Fixed:** **RSASSA-PSS is now verifiable.** RFC 8446 §4.4.3 requires it for
  an RSA handshake signature and stage 2b-i refused it outright, so until now
  the engine could not have verified a single real RSA handshake. PSS stays
  refused for *certificates*, which is deliberate rather than inconsistent:
  X.509 carries the hash, mask generation function, salt length, and trailer as
  parameters that can each be misread into verifying the wrong thing, while a
  TLS scheme number fixes all four and leaves nothing to misparse.
- **Changed:** the certificate and handshake namespaces are separate types
  because they disagree, and the module docs now put the disagreements in one
  table. The sharpest: an X.509 `ecdsa-with-SHA256` names only a hash, so the
  curve is read off the key — the correction real trust-store certificates
  forced on 2b-i — while TLS's `ecdsa_secp256r1_sha256` names hash *and* curve,
  so a P-384 key under it is refused. One key, two opposite right answers.
- **Changed:** `rsa_pkcs1_*` is refused for handshake signatures with its own
  error rather than "unsupported". The algorithm works; it is being turned away
  on the RFC's instruction, and a reader who mistook that for a gap might
  helpfully close it.
- **Fixed:** the TLS path now checks a key's `AlgorithmIdentifier` parameters,
  which only the certificate path did before. A leaf's own key is what a
  CertificateVerify is checked against and path validation never inspects it,
  so it had been held to a lower standard than the CA above it. Found by a
  surviving mutation.
- **Added:** verifier fuzzing — all 65 536 `SignatureScheme` values walked
  against a real key with random signatures, none verifying; mutated
  certificates never verifying a signature; random key shares never panicking.
- **Known limitation, stated plainly:** RFC 8448 cannot be the independent
  oracle here, the way it was for stages 1, 3a, and 3b. Its example server key
  is 1024-bit RSA and `ring`'s PSS verifiers enforce 2048–8192 bits, so the
  RFC's own CertificateVerify is refused. That refusal is correct in 2026, and
  the suite asserts it and measures the modulus to prove that is the reason —
  but the positive PSS coverage comes from a generated key rather than from the
  RFC.
- **Known limitation, stated plainly:** no state machine yet. Nothing here
  decides what order messages may arrive in, handles a HelloRetryRequest, or
  drives a handshake. That is 3c-ii.
- **Known limitation, stated plainly:** X25519 has no invalid public keys —
  RFC 7748 §5 decodes every 32-octet string — so the small-order check is the
  only validation there is on that curve. `ring` performs it; this crate's
  contribution is passing the key share to it correctly.
- Reachable only with the `handrolled-engine` feature *and*
  `--cfg rusty_tls_handrolled`. `rustls` remains the engine behind every
  exported type.

---

## Hand-rolled TLS engine, stage 3b: handshake messages and the transcript hash
**2026-08-03**

- **Added:** `handrolled::wire` (rusty_tls#25) — TLS's presentation language
  (RFC 8446 §3). Fixed-width integers and length-prefixed vectors, read
  strictly, with sub-readers so nesting in the input becomes another loop
  rather than another stack frame. The writer's vectors take a closure and
  backfill the length, so nothing writes a prefix by hand and no prefix can
  disagree with what follows it.
- **Added:** `handrolled::handshake` — the handshake header, ClientHello,
  ServerHello (with HelloRetryRequest detected by its sentinel `random`, not
  by message type), EncryptedExtensions, Certificate, CertificateVerify,
  Finished, the extensions block with duplicates refused per RFC 8446 §4.2,
  the CertificateVerify signed content, and `Transcript`.
- **Added:** parse/encode round-tripping as a required property, byte for
  byte, on RFC 8448's own messages. The transcript hash covers *encoded*
  messages, so a parser and encoder that are not inverses compute a hash the
  peer does not share — and one that normalises while re-encoding hashes
  something nobody sent. `Transcript` therefore takes encoded bytes and never
  a parsed message: one path from a message to the hash, through what arrived.
- **Fixed:** the gap stage 3a shipped with and documented. The server's
  Finished `verify_data` now has a known-answer test. RFC 8448 publishes no
  labelled value for the transcript hash it covers, but with the messages
  parseable the hash is computable from the RFC's own bytes — and the MAC over
  it matches. The transcript (3b), the key schedule (3a), and the Finished MAC
  (3a) all have to be right at once for that to pass.
- **Added:** handshake fuzzing, seeded from RFC 8448's exchange, asserting
  that anything accepted re-encodes to the bytes accepted and that the message
  spans tile their input exactly with no gap or overlap. It caught six of the
  eleven mutants in this stage's mutation run on its own.
- **Added:** a wire-level test suite. Mutation testing found that deleting the
  length-overrun check left every handshake test passing — fairly, since the
  input is refused either way and only the error's *name* changes. The check
  is still worth keeping: a truncated stream is a network event, while a
  length prefix claiming more than its container holds is a peer sending a
  contradiction, and reporting the second as the first reports a hostile peer
  as a flaky link. The new suite pins that distinction, and the doc comment
  now says plainly that the check is not what keeps a read in bounds.
- **Known limitation, stated plainly:** no state machine, no key exchange, no
  X25519, and nothing that decides whether a handshake should proceed. This
  module reports what a message says, in the same sense the X.509 parser
  reports what a certificate says. Driving a handshake is stage 3c.
- **Known limitation, stated plainly:** `NewSessionTicket`, `KeyUpdate`, and
  `CertificateRequest` are framed and typed but have no body parser yet — a
  client that needs them will add one in 3c.
- Reachable only with the `handrolled-engine` feature *and*
  `--cfg rusty_tls_handrolled`. `rustls` remains the engine behind every
  exported type; nothing in the public API routes through any of this.

---

## Hand-rolled TLS engine, stage 3a: the TLS 1.3 key schedule
**2026-08-02**

- **Added:** `handrolled::schedule` (rusty_tls#25) — RFC 8446 §7.1.
  `HKDF-Expand-Label`, `Derive-Secret`, the early/handshake/master secret
  schedule, traffic key and IV derivation, `finished_key`, the Finished MAC
  with constant-time verification, and the key-update step, over SHA-256 and
  SHA-384.
- **Added:** a known-answer suite against RFC 8448's published intermediate
  values — every extracted secret, every `"derived"` step, every traffic
  secret. Checked derivation by derivation rather than by round trip, because
  a key schedule that is self-consistent but wrong interoperates perfectly
  with itself and a round-trip test cannot see the difference.
- **Added:** the two hand-rolled suites now meet in the middle. The traffic
  keys `handrolled_record_kat.rs` takes as *inputs* are derived here from the
  schedule, and keys derived here decrypt the RFC's own 679-octet wire record
  through the hand-rolled record layer. Neither module was written against the
  other; both were written against the same trace.
- **Known limitation, stated plainly:** this is arithmetic over byte strings
  and nothing else. It does not encode or parse handshake messages, does not
  accumulate the transcript (a caller supplies a finished hash), does not do
  key exchange, and does not talk to a peer. Those are stages 3b and 3c, and
  until they exist there is no handshake here — only the keys one would use.
- **Known limitation, stated plainly:** the server's Finished `verify_data` is
  not covered by a known-answer test. Its transcript runs through the server's
  CertificateVerify, and RFC 8448 does not publish that hash as a labelled
  value; computing it needs the handshake messages themselves, which is 3b.
  The client's `verify_data` *is* covered, since its transcript is published.

---

## Hand-rolled TLS engine, stage 2b-iii: name matching and name constraints
**2026-08-02**

- **Added:** `handrolled::name` (rusty_tls#25) — server name matching against
  `subjectAltName` (DNS with single-label wildcards, IP by octet) and RFC 5280
  §4.2.1.10 name-constraint enforcement across a path.
- **Added:** `path::verify_peer_certificate`, which does path validation and
  name matching in one call. The two checks are easy to separate and
  disastrous to separate, so they are offered together.
- **Changed:** name constraints are now *enforced* rather than fail-closed by
  accident. Previously `nameConstraints` landed in the parser's unhandled
  critical extensions and every constrained chain was refused wholesale;
  recognising the extension removed that blanket refusal, so the enforcement
  is load-bearing where it was not before. A constraint type this
  implementation cannot evaluate (`directoryName`, `rfc822Name`, URI) is an
  error rather than a skipped entry — recognising an extension without
  enforcing it is worse than not recognising it.
- **Changed:** `TrustAnchor` now carries the anchor's own name constraints
  alongside its name and key. Dropping them silently unconstrained a
  constrained root, which is exactly how an enterprise limits a private CA to
  its own namespace. Found by a test written to check something else.
- **Security:** no Common Name fallback, ever — a certificate with no
  `subjectAltName` matches nothing. Partial wildcards, wildcards outside the
  leftmost label, whole-TLD wildcards (`*.com`), and names containing a NUL
  (CVE-2009-2408) are refused, each with its own test.
- **Known limitation, stated plainly:** the whole-TLD wildcard rule is a crude
  stand-in for a public suffix list this crate does not carry. It stops
  `*.com` and does not pretend to stop `*.co.uk`.
- **Known limitation, stated plainly:** `directoryName`, `rfc822Name`, and URI
  name constraints are not evaluated, and a chain carrying one is refused
  rather than validated. Some real CAs use `directoryName` constraints, so
  this is a genuine capability gap on the safe side of one.

With this, path validation is complete.

---

## Hand-rolled TLS engine, stage 2b-ii: path building and validation
**2026-08-02**

- **Added:** `handrolled::path` (rusty_tls#25) — certification path building
  and RFC 5280 §6.1 validation. Finds a chain from a peer's certificate to a
  trust anchor and checks name chaining, every signature in the path, validity
  periods, `basicConstraints` `cA`, `keyUsage` `keyCertSign`,
  `pathLenConstraint`, unhandled critical extensions, and the end-entity
  certificate's extended key usage.
- **Added:** hard bounds on the path search — depth and a total
  signature-verification budget — because the intermediates are supplied by
  the peer and an unbounded search over them is a denial of service that has
  hit real libraries.
- **Known limitation, stated plainly:** this is not yet a complete trust
  decision. It does not check that a certificate is valid for any particular
  name, so on its own it would accept any certificate from any public CA for
  any site. Hostname and IP matching is stage 2b-iii.
- **Known limitation, stated plainly:** name constraints are not implemented.
  This is fail-closed by construction rather than by luck — `nameConstraints`
  MUST be critical, unknown critical extensions are refused, so a
  name-constrained intermediate is rejected rather than having its constraint
  ignored. The cost is that chains through such intermediates are refused. The
  fix is to implement name constraints, not to relax the critical-extension
  rule, and a test is named for that dependency.
- **Measured, not assumed:** webpki does not enforce RFC 5280 §6.1.4(n) — an
  intermediate marked `cA` whose `keyUsage` omits `keyCertSign` is accepted by
  rustls and refused here. One of three places this crate is deliberately
  stricter than the differential oracle.

---

## Hand-rolled TLS engine, stage 2b-i: certificate signature verification
**2026-08-02**

- **Added:** `handrolled::verify` (rusty_tls#25) — the first hand-rolled code
  that decides anything rather than reporting it. Answers whether a
  certificate was signed by a given key, over RSA PKCS#1 v1.5 with
  SHA-256/384/512, ECDSA with SHA-256/384 on P-256 and P-384, and Ed25519.
- **Added:** deliberate refusals, each an error and never a qualified success
  — SHA-1 and MD5 as weak, RSASSA-PSS as unsupported, unrecognised curves
  (P-521, secp256k1) rather than approximating them with the nearest
  supported one. No configuration relaxes any of it. Refusing SHA-1 loses no
  trust anchors: a root's self-signature is never checked during RFC 5280
  §6.1 path validation, which starts from the anchor's key.
- **Fixed before landing:** the first implementation read the elliptic curve
  off the signature algorithm, which is wrong — the algorithm names a hash,
  the key names the curve, and RFC 5758 does not pair them. Three P-384 roots
  signed with SHA-256 in the local trust store caught it; no generated
  certificate would have, since `rcgen`'s presets always pair curve and hash.
- **Known limitation, stated plainly:** this proves authorship, not authority.
  It builds no chain, reads no clock, and enforces no constraint, so it cannot
  make a trust decision on its own — an attacker can sign their own
  certificate. Path validation is stage 2b-ii and does not exist yet.

---

## Fuzz the hand-rolled parsers, and fix the hang it found
**2026-08-02**

- **Fixed:** an infinite loop in `handrolled::x509::ExtendedKeyUsage`'s
  iterator, found by the new fuzzer on its first run. `Reader::read` does not
  consume a value whose tag is wrong — that is what makes `OPTIONAL` fields
  work — so an `extendedKeyUsage` containing a non-OID yielded the same error
  forever and the iterator never returned `None`. It parsed, it did not panic,
  it never came back: a denial of service reachable from any certificate a
  peer sends. `GeneralNames` shared the shape and the latent bug. Reachable
  only behind both gates, so no released configuration was affected.
- **Added:** `tests/handrolled_fuzz.rs`, running on stable in CI on every
  change. Mutates a real seed corpus (the machine's own trust anchors) rather
  than generating noise, because random bytes are rejected at the first octet
  and never reach anything interesting. Asserts canonicality — anything the
  DER reader accepts must re-encode to exactly the accepted bytes — plus field
  provenance, determinism, and iterator termination. Measures its own reach
  (56% of mutants parse, 85% reach ten or more DER values) and fails if that
  drops, so it cannot silently degrade into testing nothing.
- **Added:** `fuzz/` — coverage-guided libFuzzer targets for deliberate longer
  runs on nightly. 7.1M executions clean at the time of landing.
- **Changed:** ADR-0002 now records the reasoning that failed here. "The code
  is simple enough that fuzzing would be a formality" was the argument, and it
  lost.

---

## Hand-rolled TLS engine, stage 2a: DER decoding and X.509 parsing
**2026-08-02**

- **Added:** `handrolled::der` (rusty_tls#25) — a strict DER reader that
  refuses every non-canonical encoding DER forbids: indefinite and non-minimal
  lengths, the high-tag-number form, non-minimal and negative `INTEGER`s,
  non-canonical `BOOLEAN`s, malformed OIDs and `BIT STRING`s, trailing data.
  Nothing recurses on input structure, so nesting cannot become stack depth.
- **Added:** `handrolled::x509` — certificate parsing that keeps
  `tbsCertificate`, `issuer`, and `subject` as borrows of the original input
  rather than re-encoding them. Understands `basicConstraints`, `keyUsage`,
  `extendedKeyUsage`, and `subjectAltName`; collects and reports critical
  extensions it does *not* understand, so a validator can comply with RFC 5280
  §6.1.3(f).
- **Added:** 50 tests across four oracles, including a corpus of 152 real root
  CAs — the only coverage that can catch a strict parser being strict about
  the wrong thing.
- **Known limitation, stated plainly:** this validates nothing. Parsing
  returning `Ok` says a certificate is well-formed, not that any claim in it is
  true. An embedded NUL in a `dNSName` is deliberately preserved rather than
  dropped or rejected (CVE-2009-2408), because hiding it from the name matcher
  that must defend against it would be worse.

---

## Hand-rolled TLS engine, stage 1: the TLS 1.3 record layer
**2026-08-02**

- **Added:** `handrolled::record` (rusty_tls#25) — RFC 8446 §5 AEAD protection
  and framing over an established connection, for AES-128-GCM, AES-256-GCM,
  and ChaCha20-Poly1305, with a sequence counter that refuses to wrap rather
  than reuse a nonce.
- **Added:** the `handrolled-engine` cargo feature, which does nothing without
  `--cfg rusty_tls_handrolled` as well. Two gates because cargo features are
  unified across a dependency graph — a feature alone would let any crate in a
  consumer's tree enable hand-rolled TLS for everyone else in that build — and
  a `--cfg` flag comes from RUSTFLAGS, where no dependency can reach it.
- **Added:** ADR-0002, recording "never the default" as a binding decision
  rather than an intention, plus the staging order and the bar each stage must
  clear.
- **Added:** 33 tests over three oracles — byte-identical differential output
  against rustls' own `MessageEncrypter`, RFC 8448 known-answer vectors (the
  only oracle independent of rustls), and a rejection suite.
- **Known limitation, stated plainly:** `rustls` remains the engine behind
  every type this crate exports, permanently. Nothing in the public API routes
  through the hand-rolled module, and no released configuration reaches it.

---

## Load OS trust anchors via `platform`, not `rustls-native-certs`
**2026-08-02**

- **Changed:** `TrustPolicy::System` now reads OS trust anchors through
  [`platform::security::TrustAnchors`](https://github.com/baileyrd/rustils)
  instead of `rustls-native-certs` (rusty_tls#24). No API change, no
  contract change — same best-effort anchor set, same three documented
  fidelity limits, same fail-closed-on-zero behavior. Reading a trust
  store is the one part of the TLS problem that is pure OS personality
  rather than cryptography, so it moved to the crate this ecosystem
  keeps OS personality in. Background: rustils' `design-discussion-tls.md`
  ("B1 reopened") and rustils#88, which built the slice for this.
- **Removed:** the `rustls-native-certs` dependency. Added `platform`
  plus exactly one backend per target (`platform-linux` /
  `platform-windows` / `platform-bsd`), all rev-pinned.
- **Added:** `tests/system_trust.rs` — the first test `TrustPolicy::System`
  has ever had. It stages a self-generated CA as the OS trust store via
  `SSL_CERT_FILE` and asserts three things end-to-end: a leaf from that CA
  verifies, a leaf from *any other* CA is rejected (proving the staged set
  is the only one in play, so the happy path isn't passing by falling back
  to the machine's real store), and an `SSL_CERT_FILE` naming no usable
  certificate fails closed rather than silently falling back. Verified to
  fail when the implementation is deliberately broken, so it isn't passing
  vacuously. Linux/BSD only — Windows and macOS don't honor
  `SSL_CERT_FILE`, so no hermetic anchor set can be staged for them here;
  their backends are covered by rustils' own parity suite on real runners.
- **Fixed:** `rusty_tokio` was a `path = "../rusty_tokio"` dependency, so
  `cargo metadata` failed in any fresh clone and CI had been red since
  `22c30a3`. Now the rev-pinned git dependency its own comment already
  claimed it was.
- **Known limitation:** with `--all-features`, two versions of `platform`
  build side by side — `rusty_tokio`'s pin resolves to an older rustils
  commit than this crate's. Harmless today (no `platform` type crosses
  between them) but wasteful, and it would become a real type mismatch if
  one ever did. Fix belongs in `rusty_tokio`: bump its rustils pin to
  match.

---

## Add CRL-based revocation checking to `TrustPolicy`
**2026-07-23**

- **Added:** `TrustPolicy::PinnedAnchorsWithRevocation { roots, crls }` —
  like `PinnedAnchors`, but additionally rejects any server certificate
  appearing on one of the caller-supplied, DER-encoded CRLs. Wired via
  `rustls::client::WebPkiServerVerifier::builder(..).with_crls(..)` in
  place of the plain `with_root_certificates` path the other
  verifying variants use.
- **Added:** `Error::InvalidRevocationConfig`, for when the roots/CRLs
  can't be turned into a verifier.
- **Scope:** CRL only, deliberately — OCSP remains out of scope (now
  explicit in `ARCHITECTURE.md`'s Non-goals) since it would mean this
  crate either making network calls for the first time or accepting
  caller-supplied staples, a separate decision from "check a
  caller-supplied CRL."
- **Context:** closes the gap tracked against `ARCHITECTURE.md`'s
  Non-goals list (parity-loop run), landing on top of the previous entry's
  `#[non_exhaustive]` migration so this variant didn't need a second
  breaking change.
- **Tests:** 3 new hermetic tests against a plain rustls server — a
  revoked certificate rejected, a live certificate from the same CA (and
  checked against the same CRL) accepted, and empty roots failing at
  config-build time the same way `PinnedAnchors` with zero certs does. All
  tests passing; `cargo clippy --all-targets --all-features -- -D
  warnings` and `cargo fmt --check` both clean.

## Make `TrustPolicy` `#[non_exhaustive]`
**2026-07-23**

- **Changed:** `TrustPolicy` is now `#[non_exhaustive]`.
- **Context:** a prerequisite for adding CRL/OCSP revocation support
  (tracked in #13) without a second breaking change — this crate's own
  usual pure-addition discipline doesn't stretch to "add an enum variant,"
  which is itself breaking for any downstream exhaustive `match`. Taking
  that cost once, deliberately, up front, per explicit decision on how to
  sequence #13's work.

### Upgrade notes

**Breaking change.** Any code that exhaustively `match`es on `TrustPolicy`
without a wildcard arm (`_ => ...`) will no longer compile against this
version. Known affected consumers: `rusty_request` and `rusty_rdp`, both
already migrated onto this crate — each needs a wildcard arm added to any
`TrustPolicy` match before picking up this version. No behavior changes
otherwise; existing variants (`System`, `PinnedAnchors`,
`DangerNoVerification`) are unchanged.

- **Tests:** no test changes needed — this repo's own use of `TrustPolicy`
  only ever constructs variants, never exhaustively matches on it. All 32
  tests still passing; `cargo clippy --all-targets --all-features -- -D
  warnings` and `cargo fmt --check` both clean.

## Enable TLS session resumption across connections
**2026-07-23**

- **Added:** `TlsConnector`, the client-side mirror of `TlsAcceptor` —
  builds a `ClientConfig` once via a new `pub(crate)` `TlsStream`/
  `AsyncTlsStream::from_config`, then reuses it (`Arc`-backed) across every
  `connect()`/`connect_async()` call. `TlsStream::new`/`AsyncTlsStream::new`
  build a fresh config (and thus a fresh, empty resumption cache) per call,
  so rustls' own default resumption support (a real 256-entry session
  cache, on by default) never had reused state to resume from through them
  alone — `TlsConnector` is the opt-in path for a caller that reconnects to
  the same host repeatedly.
- **Added:** `resumed_session()` on `TlsStream`/`AsyncTlsStream`, reporting
  whether a given connection resumed a previous session — the only way to
  actually confirm this gap is closed, not just that a connection succeeds.
- **Fixed:** `TlsAcceptor` silently disabled TLS 1.3 session resumption
  specifically — `rustls::ServerConfig` defaults its `ticketer` to
  `NeverProducesTickets`, so a server built via this crate never issued
  session tickets even though `TlsAcceptor` already builds its config once
  and reuses it across every accepted connection (exactly what resumption
  needs). Fixed via a shared `finish_config` helper that installs a real
  `rustls::crypto::ring::Ticketer` in all three `TlsAcceptor` constructors.
  Stateful (session-ID) resumption already worked, since `ServerConfig`'s
  default `session_storage` is a real in-memory cache — this was
  specifically the TLS 1.3 ticket-issuance half.
- **Context:** closes the gap tracked against `ARCHITECTURE.md`'s Non-goals
  list (parity-loop run); session resumption dropped from Non-goals.
- **Tests:** 3 new hermetic tests (2 sync + 1 async) that make two
  *sequential* connections through a shared `TlsConnector`/`TlsAcceptor`
  and confirm the second one actually resumes — plus a negative case
  confirming two independent `TlsConnector`s (mirroring what two
  independent `TlsStream::new` calls would do) never share a cache and so
  never resume. All 32 tests passing; `cargo clippy --all-targets
  --all-features -- -D warnings` and `cargo fmt --check` both clean.

## Add ALPN protocol negotiation to client and server adapters
**2026-07-23**

- **Added:** `new_with_alpn` constructors on `TlsStream`, `AsyncTlsStream`,
  and `TlsAcceptor` (each accepting `alpn_protocols: Vec<Vec<u8>>`,
  wire-format protocol IDs e.g. `b"h2"`), alongside the existing `new`
  constructors (untouched, no ALPN offered). `negotiated_alpn_protocol()`
  reads back what was actually negotiated, on all four stream types
  (`TlsStream`, `AsyncTlsStream`, `TlsServerStream`,
  `AsyncTlsServerStream`) — `None` until the handshake completes, and
  `None` after it if either side offered nothing or nothing overlapped.
  `AsyncTlsServerStream::accept_async` needed no changes: it already
  reuses whatever `ServerConfig` the acceptor was built with.
- **Added:** `trust::build_client_config_with_alpn`, following the same
  shared-`client_config_builder` pattern the mTLS constructors use.
- **Context:** closes a gap tracked against `ARCHITECTURE.md`'s Non-goals
  list (parity-loop run); ALPN dropped from Non-goals entirely.
- **Tests:** 4 new hermetic tests (3 sync + 1 async) using this crate's own
  client and server types together — a shared protocol negotiated, no
  protocol offered on either side (correctly negotiates none), and a
  confirmed hard failure per RFC 7301 when the offered protocol sets don't
  overlap. All 29 tests passing; `cargo clippy --all-targets --all-features
  -- -D warnings` and `cargo fmt --check` both clean.

## Add client-certificate (mTLS) verification to `TlsAcceptor`
**2026-07-23**

- **Added:** `TlsAcceptor::new_with_client_auth(cert_chain_der, private_key_der, client_ca_roots_der)`,
  requiring and verifying a client certificate against caller-supplied
  client-CA roots (via `rustls::server::WebPkiClientVerifier`), alongside
  the existing `new` (which stays on the no-client-auth path). Pairs with
  the client side's `new_with_client_identity` (previous entry) for full
  mTLS — this closes the loop, so mTLS is now round-trippable through this
  crate alone rather than needing a raw rustls server as one half.
- **Added:** `Error::InvalidClientCaRoots`, for when the supplied roots
  can't be turned into a client-certificate verifier (most commonly none
  supplied, or none valid).
- **Context:** closes the server-side half of the gap tracked against
  `ARCHITECTURE.md`'s Non-goals list (parity-loop run); `ARCHITECTURE.md`
  and `lib.rs` updated to drop mTLS from Non-goals entirely now that both
  halves exist.
- **Tests:** 4 new hermetic tests using only this crate's own client and
  server types together — a trusted client cert accepted, an untrusted-CA
  client cert rejected, no-client-cert rejected, and empty client-CA roots
  rejected outright at `new_with_client_auth` time. All 25 tests passing;
  `cargo clippy --all-targets --all-features -- -D warnings` and `cargo
  fmt --check` both clean.

## Add client-certificate (mTLS) presentation to `TlsStream`/`AsyncTlsStream`
**2026-07-23**

- **Added:** `TlsStream::new_with_client_identity`/`AsyncTlsStream::new_with_client_identity`,
  presenting a client certificate + private key to a server that requests
  and verifies one (mTLS), alongside the existing `new` constructors (which
  stay on the plain no-client-auth path).
- **Refactored:** `trust::build_client_config` split into a shared
  `client_config_builder` (the server-verification decision, per
  `TrustPolicy`) plus two thin callers — one ending in
  `with_no_client_auth()`, the new one ending in `with_client_auth_cert(..)`
  — so the trust-decision logic isn't duplicated between the two paths.
- **Context:** closes a gap tracked against `ARCHITECTURE.md`'s Non-goals
  list (parity-loop run); covers the client-presents-a-certificate half
  only — server-side verification is tracked separately (issue #10).
- **Tests:** 3 new hermetic tests (sync + async) against a plain rustls
  server configured to require client auth (this crate's own
  `TlsAcceptor` doesn't verify client certs yet), covering both successful
  presentation and rejection when none is presented. All 21 tests passing;
  `cargo clippy --all-targets --all-features -- -D warnings` and `cargo
  fmt --check` both clean.

## Add `AsyncTlsServerStream`: async server-side TLS adapter (feature `rusty-tokio`)
**2026-07-23**

- **Added:** `AsyncTlsServerStream<S: AsyncRead + AsyncWrite>`, the async
  counterpart to `TlsServerStream` — produced by the new
  `TlsAcceptor::accept_async(io)`, driving the same sans-IO
  `rustls::ServerConnection` over `rusty_tokio`'s poll-based I/O the way
  `AsyncTlsStream` already does on the client side. Same lazy-handshake
  shape, plus an async `complete_handshake()`.
- **Context:** closes a gap tracked against `ARCHITECTURE.md`'s Non-goals
  list (parity-loop run). Built without a confirmed live consumer today —
  an explicit, requested exception to this project's usual consumer-gating
  discipline, same as the sync server adapter's own history.
- **Bug caught during implementation:** the first `complete_handshake`
  draft reused the client adapter's `poll_complete_io` loop directly, which
  exits only once `wants_read()` goes false — but that stays true
  indefinitely on an idle, already-established connection (a live
  connection always "wants" more incoming bytes). Awaited directly, that
  loop would block forever past the point the handshake actually finished.
  Fixed with a dedicated `poll_handshake` that checks `is_handshaking()`
  before every I/O round instead, the same condition
  `rustls::Connection::complete_io` uses internally for the sync adapters.
- **Tests:** 2 new hermetic tests, including a full async-client-against-
  async-server round trip. All 18 tests (7 sync client + 4 async client +
  4 sync server + 2 async server + 1 doctest) passing; `cargo clippy
  --all-targets --all-features -- -D warnings` and `cargo fmt --check`
  both clean.

## Add server-side TLS: `TlsAcceptor`/`TlsServerStream`
**2026-07-21**

- **Added:** `TlsAcceptor::new(cert_chain_der, private_key_der)` (builds
  the server config once — no client-certificate authentication, matching
  the client side's own MVP scope; the private key format is
  auto-detected among PKCS#8/PKCS#1/SEC1) and `TlsAcceptor::accept(sock)`,
  which produces a `TlsServerStream<S>` — the server counterpart to
  `TlsStream`, same lazy-handshake shape, same `complete_handshake()`.
  `Error::InvalidPrivateKey` covers the one new failure mode (key isn't
  valid DER in any recognized format).
- **Known limitation, stated plainly:** sync only — no async server
  adapter yet (nothing drives `ServerConnection` over `rusty_tokio`). No
  client-certificate authentication (mTLS) on either side. Built without a
  confirmed live consumer today (`rusty_rdp`'s existing server-side code
  works fine on raw `rustls` and wasn't required to migrate; `rusty_llama`'s
  server TLS shape, flagged in this project's design record, has never
  been verified against its actual source) — an explicit, requested
  exception to this project's usual consumer-gating discipline, not a
  quiet one.
- **Tests:** 4 new hermetic tests, including a full client↔server
  round trip using this crate's own `TlsStream` against its own
  `TlsAcceptor`/`TlsServerStream` — proving the two halves interoperate,
  not just that each compiles in isolation. All 16 tests (7 sync client +
  4 async client + 4 server + 1 doctest) passing; `cargo clippy
  --all-targets --all-features -- -D warnings` and `cargo fmt --check`
  both clean.

## Add `TlsStream::complete_handshake`/`peer_certificate_der`
**2026-07-21**

- **Added:** `TlsStream::complete_handshake()` (blocks until the
  handshake finishes, without requiring the caller to send/expect
  application data first) and `TlsStream::peer_certificate_der()` (the
  peer's end-entity certificate, as raw DER bytes — never a parsed
  rustls type, keeping the seam intact). Driven by a real, named
  consumer: `rusty_rdp`'s CredSSP exchange needs the server's public key
  for channel binding *before* the CredSSP bytes go over the wire, which
  the sync adapter had no way to give it without exposing rustls
  internals directly.
- Sync adapter only (`TlsStream`, not `AsyncTlsStream`) — no async
  consumer has needed this yet; add it there if and when one does,
  rather than speculatively now.
- **Tests:** 1 new hermetic test (handshake starts pending, completes on
  `complete_handshake()`, exposes the expected DER, and the connection
  remains fully usable for application data afterward). All 12 tests
  (7 sync + 4 async + 1 doctest) passing; `cargo clippy --all-targets
  --all-features -- -D warnings` and `cargo fmt --check` both clean.

## Add the async adapter (`AsyncTlsStream`, feature `rusty-tokio`)
**2026-07-21**

- **Added:** `AsyncTlsStream<S: AsyncRead + AsyncWrite>`, driving the same
  sans-IO `rustls::ClientConnection` the sync `TlsStream` uses, but over
  `rusty_tokio`'s poll-based `AsyncRead`/`AsyncWrite` and reactor instead
  of blocking I/O. Same `TrustPolicy`, same constructor shape, same "never
  dials" rule. Gated behind a new `rusty-tokio` feature (off by default) so
  a sync-only consumer never pulls in `rusty_tokio`.
- **Added:** a hermetic async handshake test suite mirroring the sync
  one — success + round-trip, `DangerNoVerification`, wrong-hostname
  rejection, untrusted-root rejection — run via `#[rusty_tokio::test]`
  against a plain sync rustls server on a background thread (only the
  client side is what this adapter is responsible for).
- **Implementation note:** rustls has no built-in poll-based adapter (it
  only ships the sans-IO connection plus the blocking `rustls::Stream`),
  so this crate's own `poll_complete_io` drive loop plus a small
  `PollAdapter` (translates `Poll::Pending` to `io::ErrorKind::WouldBlock`
  for rustls' synchronous `read_tls`/`write_tls` to see, the same
  translation tokio-rustls uses) had to be written — not reused from
  anywhere.
- **Tests:** 4 new tests, all passing; `cargo clippy --all-targets
  --all-features -- -D warnings` and `cargo fmt --check` both clean.
- This completes sequencing step 3 from the project handoff. Remaining:
  the `rusty_request` and `rusty_rdp` consumer PRs (steps 4–5), and the
  follow-up rows in step 6.

## Bootstrap the library: sync TLS client, TrustPolicy, hermetic tests
**2026-07-21**

- **Added:** the crate's first real code. `TlsStream<S: Read + Write>` (a
  sync TLS client adapter wrapping `rustls::Stream` internally) and
  `TrustPolicy` (`System` via `rustls-native-certs`, `PinnedAnchors` for
  hermetic tests/private CAs, `DangerNoVerification` for out-of-band-trust
  deployments like RDP) — the two pieces `rusty_rdp`'s eventual migration
  and `rusty_request`'s `https://` support both need. No rustls type is
  part of the public API.
- **Added:** a hermetic handshake test suite (no network, no real CA) —
  one success path plus four rejection tests (wrong hostname, expired
  cert, untrusted root, zero pinned anchors), deliberately outnumbering
  the happy path per the design record's point that TLS failures are
  silent by default.
- **Known limitation, stated plainly:** client-only — no server-side TLS,
  no async adapter yet (`rusty_tokio` integration is the next real
  consumer-forcing step), no ALPN/session resumption/client-cert auth.
  `Csprng` integration (mirroring `rusty_rdp`'s pattern) was considered and
  dropped: rustls brings its own RNG, and this crate has no real call site
  that needs one — see `ARCHITECTURE.md`'s Non-goals rather than carrying
  a speculative feature.
- **Tests:** 6 new tests (1 unit-style config-validation test + 5
  integration tests against a local rustls test server); all passing,
  0 ignored. `cargo clippy --all-targets --all-features -- -D warnings`
  and `cargo fmt --check` both clean.

## Add basic CI workflow
**2026-07-21**

- **Added:** `.github/workflows/ci.yml` running `cargo fmt --check`, `clippy
  -D warnings`, `build`, and `test` on push to `main` and on PRs.
- **Known limitation:** no `Cargo.toml`/source exists yet, so the Rust steps
  are gated behind a `Cargo.toml` existence check and no-op for now — they'll
  start running for real once source lands, with nothing further to wire up.

## Repo governance setup
**2026-07-21**

- **Added:** standard governance file set (PR/issue templates, CONTRIBUTING,
  CODE_OF_CONDUCT, SECURITY, CHANGELOG, RELEASE_NOTES, ARCHITECTURE, ADR seed)
  via repo-config, and filled in README with a real description and dev
  commands.
- **Known limitation:** repo has no Cargo.toml or source yet — README's
  "Getting started" and ARCHITECTURE's boundary table are placeholders until
  actual code lands. Security contact is a personal email, not a team alias.
