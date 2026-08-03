# ADR-0002: A hand-rolled TLS engine behind a permanently non-default seam

Status: Accepted
Date: 2026-08-02

## Context

`rustls` is the one dependency this crate exists to wrap. Everything else
here — `TlsStream`, `AsyncTlsStream`, `TrustPolicy`, `TlsAcceptor` — is
adapter code around it. For an ecosystem whose stated purpose is hand-rolling
its own stack, the protocol implementation is the largest remaining borrowed
piece, and `README.md` already promises the seam exists precisely so that
"what sits behind it can be replaced piece by piece later without any consumer
changing a line."

rusty_tls#25 proposes taking that sentence literally. This ADR is the record
the issue asks for, and it exists because of an asymmetry worth stating
plainly:

> A wrong regex crate returns wrong matches, and you notice. A wrong TLS
> implementation silently accepts a forged certificate or leaks a session key,
> and you do not notice — an attacker does.

Every other hand-rolled component in this ecosystem fails loudly. This one
fails silently, in someone else's favor. That asymmetry — not the difficulty
of the code — is what this decision is organized around.

`ARCHITECTURE.md`'s non-goals already say rustls stays the engine and that an
alternative backend "happens explicitly, never silently promoted to default."
That was a statement of intent. This ADR converts it into a mechanism, because
intent does not survive contact with `cargo` feature unification.

## Decision

### 1. The engine ships behind two independent gates, not one

The hand-rolled engine compiles only when **both** of these hold:

1. the cargo feature `handrolled-engine` is enabled, **and**
2. the cfg flag `rusty_tls_handrolled` is set (via `RUSTFLAGS`).

Neither is on by default, and neither alone does anything.

The two gates are not redundant, because they fail differently:

- **A cargo feature alone is not sufficient.** Cargo features are *unified*
  across the dependency graph. If any crate anywhere in a consumer's tree —
  a transitive dependency five levels down, a dev-dependency leaking through
  a shared build — enables `rusty_tls/handrolled-engine`, every other user of
  `rusty_tls` in that build gets it too. Nobody has to opt in for everybody to
  be opted in. That is the exact "feature unification surprise" rusty_tls#25
  names, and a feature flag cannot defend against it because unification is
  what features are *for*.
- **A cfg flag is not unified.** `--cfg` comes from `RUSTFLAGS`, which is set
  by the person running the build, not by any crate in the graph. A dependency
  cannot set it on your behalf. There is no transitive path to it.

So the cfg is the gate that actually carries the guarantee, and the feature is
what keeps `ring` out of the dependency graph for everyone who is not using
this. Together they mean `rusty_request` and `rusty_rdp` cannot land on the
hand-rolled path by accident — only by a deliberate, local, visible act by
whoever invokes `cargo`.

This is the same construction `tokio` uses for `tokio_unstable` and
`getrandom` uses for its custom backend. It is not novel; it is the known tool
for "must not be reachable by feature unification," which is why it is the one
chosen here.

### 2. Enabling the feature without the cfg is a documented no-op, not a silent one

`feature = "handrolled-engine"` without `--cfg rusty_tls_handrolled` compiles a
stub `handrolled` module whose only content is documentation explaining what
is missing and how to enable it. A silent no-op would be a footgun ("I turned
the feature on, where is the module?"); a `compile_error!` would break
`cargo build --all-features`, which this repo's CI runs and which any consumer
is entitled to run. The stub is the option that is neither silent nor
explosive.

### 3. Never the default. This does not expire.

`rustls` is what every consumer gets. There is no version of this crate in
which the hand-rolled engine becomes the default, becomes a default feature,
or gets promoted on the grounds that its tests pass.

That last clause is the load-bearing one. "The tests pass" is the confidence
that precedes most TLS vulnerabilities — Heartbleed, goto fail, and BERserk
all shipped with passing test suites. A test suite proves the absence of the
bugs someone thought of. The relevant bugs here are the ones nobody thought of,
found by an adversary who is looking, later, on purpose.

Superseding this decision requires superseding this ADR explicitly, with the
argument written down. It is not something a future PR gets to do as a side
effect of a green CI run.

### 4. Staging order, and what "shipping" a stage means

Per rusty_tls#25, ordered by value ÷ risk, each stage independently useful and
independently abandonable:

| Stage | Scope | Status |
| --- | --- | --- |
| 1 | TLS 1.3 record layer — AEAD framing over an established connection | **landed** |
| 2a | DER decoding and X.509 certificate parsing | **landed** |
| 2b-i | Certificate signature verification | **landed** |
| 2b-ii | Path building and RFC 5280 §6.1 validation — chain to an anchor, expiry, `basicConstraints`/`keyUsage`, path length, EKU | **landed** |
| 2b-iii | Name matching (hostname, IP) and name constraints | **landed** |
| 3a | TLS 1.3 key schedule — HKDF-Expand-Label, the secret schedule, traffic keys, Finished | **landed** |
| 3b | Handshake message encoding/parsing and the transcript hash | **landed** |
| 3c-i | Ephemeral key exchange, and the TLS `SignatureScheme` namespace — including RSASSA-PSS, which 2b-i refused | **landed** |
| 3c-ii | The client state machine — flight ordering, HelloRetryRequest, driving it all | **landed** |
| 4 | TLS 1.2 — only if a real peer forces it | not started |
| 5 | Server side — last, or never | not started |

Stage 2 is split because the original single row was two units of work with
different risk profiles. Bundling them would have meant landing a validator
together with the parser it depends on, with no opportunity to be wrong about
only one of them. 2a takes hostile input and decides nothing; 2b decides
everything and takes only what 2a produced. They fail differently and are
worth reviewing separately.

2b splits for the same reason 2 did, and the boundary is the same kind: 2b-i
answers "who signed this", 2b-ii answers "should we believe them". The first
is a cryptographic fact with one right answer; the second is a policy
question with a dozen interacting rules. Landing them together would have
made it impossible to be confident about only one.

Two things learned in 2b-i are worth carrying forward rather than
rediscovering:

- **Real certificates corrected a design error.** The first implementation
  read the elliptic curve off the signature algorithm — `ecdsa-with-SHA256`
  implying P-256. That is wrong: the algorithm names a *hash*, the key names
  the curve, and RFC 5758 does not pair them. Three roots in the trust store
  are P-384 keys signed with SHA-256, and they are the only reason it was
  caught. No generated test certificate would have produced that combination,
  because `rcgen`'s presets pair curve and hash by convention.
- **SHA-1 is refused, and refusing costs nothing.** 28 of 152 roots carry
  SHA-1 self-signatures, which sounds like a reason to accept it and is not: a
  trust anchor's self-signature is never checked in RFC 5280 §6.1 path
  validation, which starts from the anchor's key. Refusing means SHA-1 can
  never authenticate a link *inside* a chain, which is where chosen-prefix
  collisions actually matter.

2b-ii split once more, leaving name matching and name constraints for 2b-iii,
which has now landed and closed that gap. The arrangement in between is worth
keeping on the record, because the shape recurs: **name constraints were
fail-closed by construction rather than by enforcement.** `nameConstraints`
MUST be critical (RFC 5280 §4.2.1.10), the parser reported critical extensions
it did not understand, and `validate_path` refused any certificate carrying
one — so a name-constrained intermediate was refused outright rather than
having its constraint ignored. Safe, and it cost real capability.

2b-iii recognised the extension, which removed that blanket refusal and made
the new enforcement load-bearing in a way it had not been. That transition is
the dangerous kind, and two rules came out of it that later stages should
inherit:

- **Recognising an extension without enforcing it is strictly worse than not
  recognising it.** A constraint type `handrolled::name` cannot evaluate is
  therefore an error, not a skipped entry.
- **A trust anchor's own constraints have to travel with it.** `TrustAnchor`
  carries a name, a key, *and* constraints. Dropping them would silently
  unconstrain a constrained root, which is exactly how enterprise deployments
  limit a private CA to their own namespace. This was found by a test, not by
  design — see below.

Also measured in 2b-ii, and worth knowing before writing 2b-iii: **webpki does
not enforce RFC 5280 §6.1.4(n)**. An intermediate marked `cA` whose `keyUsage`
omits `keyCertSign` is accepted by rustls and refused here. That is one of
three places this crate is deliberately stricter than the differential oracle,
so the oracle cannot be used as a specification — only as a cross-check on the
cases where both are answering the same question.

Mutation testing earned its place again in 2b-iii, and differently from
before. Two mutations survived the suite as originally written, and both were
real gaps rather than test-quality nits:

- Applying name constraints only to the end-entity certificate passed every
  test, because every test had the constraint one level above the leaf. RFC
  5280 §6.1.4 constrains *every* certificate below the constraining CA, and an
  intermediate can carry a `subjectAltName` too.
- Dropping a trust anchor's own constraints passed as well, because
  `TrustAnchor` was not carrying them at all.

The second is the more interesting: the test that exposed it was written to
check something else and failed for a reason its author had not considered.
That is what a test suite is for, and it only works if a failing test is read
rather than reshaped until it passes.

The split draws a line that then has to stay drawn: **2a validates nothing**.
A parsed certificate is an attacker-supplied document that has been given
structure, and `Certificate::parse` returning `Ok` is not evidence for
anything the certificate claims. Until 2b exists there is no trust decision
available from that module at all — which its own documentation says at the
top, because this is precisely the distinction that gets lost.

A stage "ships" only when it meets the bar in §5. A stage that cannot meet the
bar does not land behind the flag either — the flag is not a place to park
code that does not work.

Stage 3 splits three ways on the same principle as 2 and 2b: 3a is arithmetic
over byte strings with no state and no peer, 3b is parsing hostile input, 3c is
a state machine that talks to someone. Three different failure modes, three
different test strategies.

3a is where RFC 8448 pays off a third time. It publishes every PRK, every
`info` string, and every expanded secret at every step, so the tests check
individual derivations rather than that a handshake works — which matters,
because a key schedule that is self-consistent but wrong interoperates
perfectly with itself and with nothing else, and a round-trip test cannot see
the difference when both sides are the code under test.

One thing 3a got wrong first, worth recording because the reasoning was
reasonable: `HKDF-Expand-Label` outputs of different lengths do *not* share a
prefix, even though plain HKDF-Expand produces a stream and RFC 5869 alone
would suggest they do. The requested length is a field of `HkdfLabel` and so
part of the `info`. The test was written the other way round and failed
immediately — free domain separation by output length, discovered by asserting
the opposite.

3b closes a gap 3a landed with rather than hid. 3a could not assert the
server's Finished `verify_data`, because the transcript it covers runs through
the server's CertificateVerify and RFC 8448 publishes no labelled value for
that hash. Documenting the gap was the right call; with 3b's parser the hash is
computable from the RFC's own messages, and the MAC over it matches. Three
independently-written pieces — the transcript, the key schedule, and the
Finished MAC — have to be simultaneously correct for that assertion to pass,
which is worth more than any of them checked alone.

The property 3b is actually built around is that **parsing and encoding are
inverses on the wire bytes**. That is not tidiness. The transcript hash covers
encoded messages, so a client that parses one way and re-encodes another
computes a transcript the peer does not share, and the handshake fails looking
like a network fault. Worse, an implementation that *normalises* while
re-encoding hashes something nobody sent — the same defect as re-encoding a
`tbsCertificate` before checking its signature, and the reason the X.509
parser keeps it as a borrow. `Transcript` therefore takes encoded bytes and
never a parsed message: one path from a message to the hash, and it goes
through what arrived.

Two things came out of mutation-testing 3b that generalise:

- **A check that changes no accept/reject decision cannot be pinned by
  behavioural tests, and may still be worth keeping.** Deleting the
  length-overrun check from `wire::Reader` left every handshake test passing,
  because `take` is bounds-checked anyway and the deletion only changed what
  the error was *called*. The check earns its place regardless: a truncated
  stream is a network event, a length prefix claiming more than its container
  holds is a peer sending a contradiction, and reporting the second as the
  first reports a hostile peer as a flaky link. The answer was a wire-level
  test suite that asserts the distinction directly, plus a doc comment saying
  plainly what the check is and is not load-bearing for. Overstating what a
  check defends against is its own bug.
- **A parser and its encoder make each other testable.** The fuzz suite's
  handshake invariant — anything accepted must re-encode to the bytes accepted
  — independently caught six of eleven 3b mutants, including ones no
  hand-written case targeted. It is the same shape as 2a's DER canonicality
  invariant, and it works for the same reason: it is a property of *every*
  input rather than of the inputs someone thought of.

3c split for the usual reason, and the boundary is the usual kind: 3c-i is
facts about bytes with no state and no peer, 3c-ii is a state machine. What
made the split obvious was discovering, while scoping 3c, that the engine could
not verify a single real RSA handshake — RFC 8446 §4.4.3 requires RSASSA-PSS
for a CertificateVerify, and 2b-i had refused PSS outright. That is a
self-contained piece of work with its own failure modes, and bundling it into a
state machine would have meant reviewing both at once.

Three things came out of 3c-i worth carrying forward:

- **The same question can have opposite right answers in two namespaces.**
  2b-i was corrected by real certificates into reading an ECDSA key's curve off
  the *key*, because `ecdsa-with-SHA256` names only a hash. TLS 1.3 then
  inverts it: `ecdsa_secp256r1_sha256` names hash *and* curve, so a P-384 key
  under that scheme must be refused. A single shared "signature algorithm"
  type would have made one of those two rules unstatable. They are separate
  types, and the module doc sets them side by side rather than leaving a
  reader to notice.
- **A refusal that is really a downgrade must not read as a gap.** The
  `rsa_pkcs1_*` schemes are implemented and are refused for a handshake
  signature on the RFC's instruction. Reporting that as "unsupported" would
  invite a future reader to helpfully add support for it, which is exactly the
  change the RFC forbids. It gets its own error variant, and a test asserts the
  two do not render alike.
- **Refusing PSS in one namespace and requiring it in the other is not
  inconsistency, and the reason should be written down.** In X.509 the PSS
  parameters are a DER structure carrying hash, MGF, salt length, and trailer,
  any of which can be misread into verifying the wrong thing — so 2b-i failed
  closed. A TLS `SignatureScheme` is one number that fixes all four. Nothing is
  left to misparse. The safe answer differs because the encodings differ.

Two testing notes from 3c-i, both about being wrong in a useful way:

- A mutation survived — dropping the Ed25519 parameters check — because every
  key in the suite came from `rcgen` and was therefore conforming. The
  certificate namespace *did* cover the same rule, which is what made the gap
  invisible: both paths looked equally tested. Chasing it turned up a real
  asymmetry as well, in that the TLS path was not checking an RSA key's
  parameters at all, so a leaf's own key was held to a lower standard than its
  issuer's.
- A fuzz invariant was written too strongly and failed immediately: "a random
  key share never agrees a secret" is false for X25519, which by RFC 7748 §5
  has *no* invalid public keys — every 32-octet string decodes. The test now
  asserts what is actually true per group, and the finding sharpened something
  worth knowing: the small-order check is the only validation X25519 has.

One limitation of 3c-i is worth recording because it will look like an
omission later. RFC 8448 has been the independent oracle for stages 1, 3a, and
3b, and it cannot be one here: its example server key is 1024-bit RSA, and
`ring`'s PSS verifiers enforce 2048–8192 bits. Refusing it is correct, so the
suite asserts the refusal — and measures the modulus to prove that is the
reason — but the positive coverage for PSS has to come from generated keys
instead of the RFC.

3c-ii is the first stage with a peer, and it changes what a test can be worth.
Every earlier stage had an oracle independent of this code — RFC 8448's
published bytes, the machine's own trust store, rustls' record layer. A state
machine has a better one, a **real server**, because a handshake is a mutual
computation: a client that derives the wrong secret, orders the transcript
wrongly, or encodes an extension slightly wrong would still satisfy a suite
where both sides are this code. `rustls` has not read this implementation, so
a completed handshake against it is evidence about TLS rather than about
internal agreement. That test is the one the stage rests on.

Interop proves only the happy path, and a client that completes a handshake
with a good server *and* with an attacker is worse than useless. The refusals
needed a peer that could be made to misbehave, and rustls cannot: the server's
flight arrives inside one AEAD-protected record, so editing it needs keys the
test does not have. The answer was a minimal test server built from this
crate's own primitives — which would be circular if it were used to prove the
client works, and is not used for that. It is used only to make the client
**refuse**, and a refusal cannot be manufactured by shared wrongness: if both
sides agreed on a malformed flight, the client would accept it and the test
would fail. A control test asserts the test server's correct flight completes,
so the rejection suite cannot pass by producing garbage.

Four things from 3c-ii worth carrying forward:

- **A state machine's expected message should be data, not control flow.**
  `Expect` is a field, so "the server skipped CertificateVerify" is the default
  for everything that is not the message required, rather than a case someone
  has to remember to write. The mutation that advanced from Certificate
  straight to Finished was caught by fourteen tests; had the ordering been a
  chain of `if`s, it would have needed one written specifically.
- **A check that only improves an error message is still worth having, and
  still must not be oversold.** Deleting the `key_share` group check survived
  the suite, and chasing it showed the check does not stop a mismatched share
  from being usable — `agree` uses the client's own group whatever the label
  says, so a wrong label just fails at the Finished instead. It buys a
  diagnosable error at the point of the disagreement. That is the same shape as
  3b's length-overrun check, and it got the same treatment: keep it, test the
  distinction directly, and say in the doc comment what it is *not* doing.
- **A test can pass because the thing it tests never happens.** The session
  ticket test was green and vacuous: this client does not offer
  `psk_key_exchange_modes`, so by RFC 8446 §4.2.9 a conforming server never
  sends a ticket, and rustls duly sent none. A mutation that returned tickets
  to the caller as application data survived. The handling is still worth
  having — the cost of getting it wrong is a caller handed handshake bytes as
  data, a protocol surprise turned into silent corruption — so the test server
  now sends one, because a conforming peer will not.
- **State the fuzz invariant you can actually defend.** "No corrupted record
  stream completes a handshake" would look like a security property and be a
  tautology, since a recorded stream is bound to one ephemeral key. The
  defensible invariants are that nothing a peer sends makes the client panic —
  a remotely reachable denial of service — and that a stream which *does*
  complete authenticated the real server. Corrupting a change_cipher_spec
  changes nothing, because that record is discarded whatever it holds; the
  test asserts that harmlessness rather than treating the completions as a
  mystery.

Interop over a real socket corrected an assumption this work had been
carrying, which is the most useful thing it did. The expectation was that real
servers split a handshake flight across records, and that this would finally
exercise the client's reassembly buffer — `rustls` sends its flight in one
record, so `complete_prefix` and the buffer had been carried by tests that
never made them work. The measurement disagreed: every server tried also sends
one record. The interop test now *reports* the count rather than asserting a
split, because asserting one would be asserting a fact about other people's
servers that can change without notice, and the gap is closed hermetically by
a test server that splits its flight on purpose, down to one octet per record.

The general lesson is the one 3c-ii already learned twice and this makes three:
**a test can be green because the thing it tests never happens.** The vacuous
session ticket, the reassembly buffer nothing reassembled, and the
wrong-name interop test whose premise an intercepting gateway removes are the
same failure wearing different clothes. The defence is to measure what the
test actually exercised — record counts, mutation survival, reach percentages
— rather than to infer coverage from a passing run.

### 5. The shipping bar

From rusty_tls#25, unchanged. Items 1 and 3 are hard gates:

1. **Differential testing against rustls.** Same input, both engines,
   byte-identical output.
2. **Interop against real servers**, plus the existing hermetic rejection
   suite passing identically on both engines. Met for the client:
   `handrolled_client` completes handshakes against a real
   `rustls::ServerConnection` across every offered suite and group, including
   a HelloRetryRequest, and `handrolled_interop` does the same over a socket
   against servers nobody here configured, fetching real HTTP responses.

   Those tests are `#[ignore]`d rather than gated on an environment variable,
   because a gated test that quietly passes when the variable is unset reports
   `ok` for a run that did nothing — which is precisely how a vacuous
   session-ticket test survived a mutation in 3c-ii. An ignored test reports
   `ignored`, which is the truth.

   Two honest limits on what that interop proves. Where egress is intercepted,
   the peer is a gateway rather than the host named; the suite prints the
   issuer so a passing run cannot be misread as having reached a particular
   server, and a gateway is still an independent TLS 1.3 stack, so the run is
   still a third opinion. And interception makes one obvious test unwritable:
   a gateway mints a certificate for whatever SNI it is handed, so "connect
   under the wrong name and watch it be refused" cannot pass in both
   environments. The checkable direction — every accepted certificate carries
   the name that was asked for — is asserted instead, with the refusal
   direction covered hermetically where the peer's certificate can be chosen.
3. **Every rejection path tested to actually reject.** The dangerous failure
   is accepting something bad, and no happy-path test catches it.
4. **Fuzz the parsers.** DER, record, and handshake parsing take hostile input
   by definition. `tests/handrolled_fuzz.rs` runs on stable in CI, seeded from
   real trust anchors and RFC 8448's exchange; `fuzz/` holds the
   coverage-guided targets for a deliberate longer run.
5. **Known-answer tests from RFC vectors**, which are the only oracle in this
   list that is independent of rustls. Differential testing cannot catch a
   misreading of the spec that both implementations share — most realistically,
   one where this crate's author read rustls' source and reproduced its
   interpretation rather than the RFC's. Stage 1 uses RFC 8448 for exactly
   this reason, and every later stage should find its equivalent.

### 6. Cryptographic primitives stay on `ring`, for now

X25519, AES-GCM, ChaCha20-Poly1305, SHA-2, and P-256 are **not** in scope for
any stage above. Hand-rolling primitives is a distinct and much harder project
than hand-rolling the protocol over them, and its central correctness property
— constant-time execution — is not testable by differential testing, KATs, or
fuzzing. Every technique on the §5 list is blind to a timing side channel.

The protocol layers get hand-rolled; the primitives underneath stay `ring`.
Revisiting that is its own issue, with its own ADR, and nothing here should be
read as a step toward it.

## Alternatives considered

- **Keep `rustls` permanently, build nothing.** Status quo, and still the right
  default regardless of what this produces. Rejected as the *only* answer
  because it forecloses the seam's entire stated purpose. Not rejected as the
  default answer — it remains that, permanently, per §3.
- **Cargo feature alone, no cfg gate.** Rejected: it does not deliver the
  guarantee rusty_tls#25 asks for. Feature unification means a transitive
  dependency can enable it for a consumer who never asked, which is precisely
  the accident the issue says must be impossible.
- **cfg gate alone, no cargo feature.** Rejected on dependency hygiene: the
  engine needs `ring` as a direct dependency, and an unconditional dependency
  would land in every consumer's tree — including `Cargo.lock`, `cargo deny`
  output, and audit surface — for code they can never reach. (`ring` is already
  present transitively via rustls, so the feature costs nothing in *build*
  terms; this is about the dependency graph being an honest description of what
  is compiled.)
- **Hand-roll and promote to default once tests pass.** Rejected — contradicts
  `ARCHITECTURE.md`'s non-goal directly, and see §3 on why "the tests pass" is
  not the reassurance it sounds like.
- **Do this in `rustils` instead.** Already rejected there: rustils'
  `docs/design-discussion-tls.md` researched hand-rolled TLS as "Option D" and
  recorded it *researched-and-declined* on unverifiable-correctness grounds.
  TLS is protocol, not OS personality, so it stays out of the PAL. Behind this
  crate's own opt-in flag is the only place it fits.
- **Start at TLS 1.2.** Rejected — larger attack surface, more legacy
  construction (CBC, MAC-then-encrypt, renegotiation), and no learning benefit
  over 1.3.
- **Start at the handshake rather than the record layer.** Rejected — the
  record layer is the smallest piece with a real test oracle on both sides
  (RFC 8448 vectors *and* a byte-for-byte differential against rustls'
  `MessageEncrypter`), and the handshake needs a working record layer under it
  regardless.

## Consequences

**Accepted:**

- A second implementation of the most security-sensitive code in the ecosystem
  now exists in this repo. It is unreachable by default and unreachable by
  accident, but it exists, and it will be read by people who assume anything
  in a repo is meant to be used. §3 and the module docs both say otherwise, in
  the places someone would actually look.
- Testing the engine requires a CI job with `RUSTFLAGS`, because
  `--all-features` alone does not reach it. Without that job the code would be
  compiled by nobody and tested by nobody — worse than not having it. One CI
  job is added per this ADR, and a stage that is not covered by it has not
  landed.
- `cargo build --all-features` on a consumer's machine silently does nothing
  new, which is the intended outcome and also a mild surprise. §2's stub module
  is the mitigation.

**Created:**

- Stage 2 (X.509 chain validation) is the next unit of work, and pairs with the
  trust-anchor loading already landed in rusty_tls#24.
- Fuzz targets were owed after stages 1 and 2a and are now delivered, in two
  forms, because one form could not do both jobs. `fuzz/` holds coverage-guided
  libFuzzer targets for the DER reader and the certificate parser; they need
  nightly and sustained runtime, so they are something a person runs
  deliberately, not something that guards a branch.
  `tests/handrolled_fuzz.rs` is the stable, deterministic counterpart that
  runs on every pull request, seeded from the machine's real trust anchors and
  mutating them rather than generating noise.

  This was worth doing rather than assuming. The reasoning for expecting it to
  find nothing — a small, total parser with every length checked and no
  `unsafe` anywhere — was sound and also wrong: the first run found an
  infinite loop in `ExtendedKeyUsage`'s iterator, reachable from any
  certificate a peer chooses to send. `Reader::read` deliberately does not
  consume a value whose tag is wrong (that is what makes `OPTIONAL` fields
  work), so an `extendedKeyUsage` containing a non-OID yielded the same error
  forever. It parsed, it did not panic, it never returned. Fifty hand-written
  tests, including thirty in a rejection suite, did not find it, because
  nobody writes a test for a loop they did not know could spin.

  The lesson is recorded here rather than in a commit message: for this
  engine, "the code is simple enough that fuzzing would be a formality" is not
  a reason to skip fuzzing. It is the argument that was made, and it lost.
- A differential gap is knowingly left open: rustls' `AeadKey` is publicly
  constructible only at its maximum length of 32 bytes, so the byte-identity
  differential covers AES-256-GCM and ChaCha20-Poly1305 but *not*
  AES-128-GCM. AES-128-GCM is covered instead by RFC 8448 known-answer
  vectors, which is a stronger oracle in kind (independent of rustls) but a
  narrower one in coverage (fixed inputs, not a matrix). Closing it properly
  needs either an upstream rustls change or a real handshake with
  `enable_secret_extraction`.

**Foreclosed:**

- Promoting the hand-rolled engine to default, without superseding this ADR.
- Hand-rolling cryptographic primitives under any stage listed here (§6).
