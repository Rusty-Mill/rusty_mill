# TLS engine assessment: can `rusty_tls`'s native engine replace rustls?

Date: 2026-10-08. Scope: `crates/libs/net/rusty_tls/src/handrolled/` (about 10.5k lines).
Status: assessment. The default, both gates and all behaviour were left alone; the only engine
changes are the F1 fix and the F11 lint fixes, made afterwards at the owner's request. Three decisions
are the owner's and are listed in section 7.

## 1. Verdict

The engine is a serious, well-tested **TLS 1.3 protocol implementation**. It is not yet a
replacement for rustls, for three separate reasons:

1. **Evidence.** It has one confirmed security bug (section 4, F1) found in this
   assessment by a single differential run, plus a thin differential against rustls
   (one four-case path test, one four-case rejection table). That a single targeted probe
   found a name-constraint bypass is itself the strongest signal about how much
   differential coverage is missing.
2. **Behaviour.** rustls speaks TLS 1.2; the engine does not (ADR-0002 stage 4b, `rusty_tls#41`
   closed not planned). Swapping engines changes which servers a consumer can reach.
   `rusty_rdp` is the likeliest to notice.
3. **Integration.** None of the seam types (`TlsStream`, `AsyncTlsStream`, `TlsAcceptor`,
   `TlsConnector`, `TrustPolicy`) route through the engine. The engine is a sans-IO core
   with its own config types; the adapters, trust mapping, ALPN, key-format detection and
   resumption cache are unwritten.

Also: **the engine is not tested by any CI job in this monorepo** (section 5, G2).

## 2. What was run

Clean runs, one at a time (an earlier attempt interleaved two logs and was discarded):

| Command | Passed | Failed | Ignored |
| --- | --- | --- | --- |
| `RUSTFLAGS='--cfg rusty_tls_handrolled' cargo test -p rusty_tls --features handrolled-engine --no-fail-fast` | 388 | 0 | 8 |
| `cargo test -p rusty_tls --no-fail-fast` | 27 | 0 | 0 |
| `cargo test -p rusty_tls --features rusty-tokio --no-fail-fast` | 39 | 0 | 0 |

Of the 388, 359 are engine tests and 29 are seam/rustls tests plus one doctest. The 8
ignored tests are the real-network interop tests (4 against public servers, 4 against
`openssl s_server`). **They were not run here** and cannot be assumed to pass.

Not run: libFuzzer targets (need nightly) and mutation testing. Clippy on the handrolled
cfg initially failed (F11, since fixed).

## 3. Evidence that exists

| Technique | What exists | Notes |
| --- | --- | --- |
| Known-answer | RFC 8448 traces drive record layer (6 tests), key schedule (19), handshake messages (24). | The only oracle independent of rustls. Covers TLS 1.3 only. RSA-PSS cannot use RFC 8448 (1024-bit key). |
| Differential vs rustls | Record layer byte-identical (5 tests; AES-128-GCM not covered). Whole handshakes between engine and rustls, both directions: client 46 tests, server 46 tests. Path outcomes vs webpki: **one test, four chains**. Shared rejection table: **four cases**. | Handshake interop is strong. Certificate-path and name differential is thin. |
| Rejection / negative | x509 rejection 34, path 31, name 24, record rejection 22, verify 16, TLS signature 13. Path tests cover expiry, not-yet-valid, wrong EKU, missing keyCertSign, non-CA intermediate, pathLen, unknown critical extension, name constraints, wrong anchor key, decoy and cycle handling. | Good breadth. Tests were mutation-checked by hand (ADR-0002), but there is no mutation tooling in the repo, so that is not reproducible. |
| Fuzz | Stable, deterministic harness (14 tests): DER, certificate, handshake-message invariants, seeded from real trust anchors. Two libFuzzer targets: `der_reader`, `certificate`. | No libFuzzer target for handshake parsing, record opening, or the client/server state machines. No continuous or long-run fuzzing. |
| Real-world corpus | Parses every OS trust anchor (4 tests). | Parsing only. Chains are never validated against real-world intermediates. |
| Interop with servers | 8 `#[ignore]`d tests (public servers, OpenSSL). | Not in CI. Public egress here is an intercepting gateway, so "real server" is partly a gateway. |
| Not present anywhere | Wycheproof vectors, x509-limbo or BetterTLS corpora, BoGo or tlsfuzzer, timing tests, an independent review. | |

## 4. Code review findings

Reviewed in full: `der.rs`, the parsing/time/extension code in `x509.rs`, `path.rs`,
`name.rs` (matching and constraints), `verify.rs` (algorithm policy and entry points),
`sign.rs`. **Not reviewed:** `record.rs`, `schedule.rs`, `handshake.rs`, `wire.rs`, `kx.rs`,
`ticket.rs`, and the `client.rs`/`server.rs` state machines (about 6.5k lines, including
the largest attack surface, the server). Only targeted greps were run on those.

### F1. Wildcard SAN bypasses an excluded name constraint (confirmed; **fixed** after this assessment)

`name.rs` `check_one` tests each SAN against excluded subtrees with `dns_within`, a literal
string comparison. A SAN of `*.example.com` is not "within" `bad.example.com`, so it is not
excluded, yet the same certificate authenticates `bad.example.com`.

Reproduced with rcgen: a root with `excludedSubtrees: bad.example.com`, and a leaf with SAN
`*.example.com`.

| Engine | `verify_peer_certificate` for `bad.example.com` |
| --- | --- |
| native | **Ok** (accepted) |
| rustls / webpki | Err `NameConstraintViolation` |

**Status: fixed.** `name.rs` now has `dns_excluded`, which treats a wildcard as excluded when
any host it can match falls inside the subtree. `a_wildcard_certificate_is_refused_when_it_covers_an_excluded_host`
(in `tests/handrolled_path.rs`) runs seven cases against both engines and they agree on
every row; it fails without the fix. Two unit tests cover the helper. Only wildcards under
*excluded* subtrees were affected; the *permitted* side already failed closed. Other
constraint shapes (IP ranges, intermediates carrying wildcards, permitted plus excluded
together) have not been differentially tested; that remains gap G4.

Impact before the fix: a name-constrained CA (the standard way to limit a private or enterprise CA) can
issue a certificate for a name it was told to exclude. The mirror case (wildcard under a
*permitted* subtree) fails closed, so only `excludedSubtrees` is affected. Fix was small:
when a presented name is a wildcard, treat it as covering its whole subtree for the
excluded test.

### Smaller findings (by reading; not all tested)

| ID | Where | Finding | Severity |
| --- | --- | --- | --- |
| F2 | `path.rs` `check_ca` | `extendedKeyUsage` is checked on the end-entity certificate only. A CA whose EKU excludes serverAuth is still accepted. Whether webpki enforces this on intermediates was **not measured**; a differential test will say. | Medium, unconfirmed |
| F3 | `path.rs` `TrustAnchor` | Carries name, key and name constraints, but not the anchor's `pathLenConstraint`. | Low |
| F4 | `x509.rs` | Leniencies: explicit `DEFAULT FALSE` BOOLEAN on an extension is accepted (documented); `KeyUsage` padding bits are not required to be zero, so a bit in the padding reads as set; `GeneralizedTime` is accepted for years before 2050; `notBefore > notAfter` and serial zero are accepted. None is exploitable without the issuing CA's cooperation, because the TBS is signed. | Low |
| F5 | `der.rs` | `read_bool` reports a wrong-length BOOLEAN as `MalformedBitString`. Cosmetic. Otherwise strict and correct: no recursion, minimal lengths, canonical integers, bounded reads. | Cosmetic |
| F6 | `sign.rs` | `SigningKey` accepts PKCS#8 only, with the algorithm named by the caller. The seam's `TlsAcceptor` auto-detects PKCS#8, PKCS#1 and SEC1. | Seam gap |
| F7 | `verify.rs` | Signature maths is `ring` (RSA 2048-8192 only, ECDSA P-256/P-384, Ed25519). P-521 and PSS in certificates are refused. Interop impact not measured. | Scope |
| F8 | whole engine | No revocation (CRL) support, while the seam has `TrustPolicy::PinnedAnchorsWithRevocation`. No consumer in the workspace uses that variant today. | Seam gap |
| F9 | `client.rs` | No ALPN handling found by grep in client or server (constant only). The seam offers ALPN, and `agentgateway-tls` uses `new_with_alpn`. Not confirmed by test. | Seam gap |
| F10 | docs | `ARCHITECTURE.md` and ADR-0002 say client certificates and resumption are refused, and that HelloRetryRequest is not generated. The code and tests show client auth and TLS 1.3 resumption on both sides, and the server module documents HRR. The docs are stale. | Docs |
| F11 | `client.rs` | `cargo clippy -p rusty_tls --all-targets --features handrolled-engine -- -D warnings` failed on rustc 1.97 with two `needless_question_mark` errors (`client.rs:1362`, `server.rs:654`). Trivial, but it shows the engine is not linted by any CI job either. **Fixed** alongside F1 (behaviour-preserving). | Low |

Good practice seen: Finished and PSK binders use `ring`'s constant-time `hmac::verify`; no
CN fallback; NUL and trailing-dot handling in names; unhandled critical extensions fail
closed; algorithm and key type are cross-checked; ECDSA curve is read from the key for
X.509 and from the scheme for TLS; SHA-1 and MD5 refused; search depth and signature-check
budgets bound path building.

## 5. Gaps, ranked

Effort is focused engineering days, rough (plus or minus 50 percent), excluding review wait
time. "Blocks default" means no evidence bar I would accept can be met without it.

| Rank | Gap | Blocks default | Effort |
| --- | --- | --- | --- |
| G1 | ~~Fix F1 and add it as a test~~ (done). Remaining: the broader name-constraint differential (permitted, IP, wildcards on intermediates, mixed). | Yes | 1 |
| G2 | **No CI job runs the engine.** The only handrolled job is in the crate's own `.github/workflows/ci.yml`, which GitHub does not execute inside a monorepo. ADR-0002 says a stage not covered by that job "has not landed". Add a root job with `RUSTFLAGS` and `RUSTDOCFLAGS` cfg, plus the zero-tests guard. | Yes | 1 |
| G3 | TLS 1.2 decision (section 7, D1). Implementing it is large; declining means a documented behaviour change or a retained rustls fallback. | Yes | 0 to 35 |
| G4 | Certificate-path differential at scale: x509-limbo and BetterTLS style corpora, plus generated chains, run against both engines, with every divergence classified as stricter, looser or equal. Includes F2. | Yes | 6 to 8 |
| G5 | Seam integration: engine-backed `TlsStream`, `AsyncTlsStream`, `TlsConnector` (with a resumption cache), `TlsAcceptor` and both server streams; `TrustPolicy` to anchors; ALPN; key-format detection; `peer_certificate_der`; error mapping; clock source. The core is sans-IO, so adapters are mostly mechanical. | Yes | 15 to 25 |
| G6 | Fuzzing: libFuzzer targets for handshake messages, record open, client and server state machines; a time-boxed CI smoke run; a scheduled long run. The server parses only unsolicited input and has no fuzz target at all. | Yes | 4 to 6, plus compute |
| G7 | Interop in CI, not `#[ignore]`: OpenSSL both directions (hermetic, no network), plus BoGo or tlsfuzzer for state-machine and alert behaviour. BoGo needs a shim and triage. | Yes | 4 (OpenSSL) to 15 (BoGo) |
| G8 | Independent security review of x509, path, name, verify, sign, and the handshake state machines. Not something this session can supply. | Yes | external, 2 to 3 weeks |
| G9 | Wycheproof vectors through the signature wrappers (ECDSA, RSA-PSS, Ed25519, X25519). The maths is `ring`'s, so the value is in the wrapper's parameter and encoding decisions. | Should | 3 |
| G10 | Review the unreviewed 6.5k lines, and add resource-limit tests (message size caps, buffer growth, ticket floods). | Yes | 5 to 7 |
| G11 | Reproducible mutation testing (`cargo-mutants`) so "survivors" is a CI number. | Should | 2 to 3 |
| G12 | Update ARCHITECTURE.md, README and ADR status (F10); supersede ADR-0002 if D3 is approved. | With D3 | 1 to 2 |
| G13 | CRL revocation parity (F8). Defer until a consumer needs it. | No | 8 to 10 |
| G14 | PKCS#1 and SEC1 key support (F6), if not folded into G5. | Part of G5 | 1 to 2 |

Total to a defensible default, excluding TLS 1.2 and external review: roughly 45 to 70 days.
With TLS 1.2: add about 30 to 40.

## 6. Proposed evidence bar (owner to set)

The ADR-0002 bar (differential, interop, rejection, fuzz, KATs) was met stage by stage but
not as a whole. The proposal below is stricter where this assessment found weakness. Every
item is a hard gate unless marked.

1. **CI.** The engine runs on every PR in the root workflow, on Linux, with a zero-tests guard.
2. **Differential, certificate side.** At least one external corpus (x509-limbo or equivalent)
   plus generated chains. Every divergence from webpki is listed and classified. No
   "looser than webpki" divergence remains unless the owner accepts it by name.
3. **Differential, protocol side.** Handshake outcomes and alerts match rustls across suites,
   groups, HRR, resumption, client auth, ALPN and SNI, in both roles.
4. **Independent oracles.** RFC 8448 and Wycheproof pass; OpenSSL interop in both directions
   runs in CI; BoGo (or tlsfuzzer) pass rate is published, with every skip justified.
5. **Fuzz.** Coverage-guided targets for every parser and for both state machines; a CI smoke
   run on each PR; a long run (for example 72 hours of CPU) clean before each release that
   changes the engine.
6. **Review.** An independent review of the security-critical modules, with findings closed.
7. **Soak.** An opt-in period (for example two releases) in which at least `rusty_request`
   and `rusty_rdp` run on the engine in their own CI and in dogfooding, with rustls still one
   switch away.
8. **Behavioural parity.** TLS 1.2 either implemented and held to items 2 to 5, or consumers
   that need it keep rustls, recorded per consumer.
9. **Mechanism.** The cfg-only gate exists because a cargo feature can be enabled by any
   dependency. If the engine becomes default, ADR-0002 must be superseded with a replacement
   guarantee, for example a default-off `native-engine` feature first, then a default-on one,
   with a documented opt-out, so a transitive dependency cannot silently change a consumer's
   TLS stack in either direction without it being visible in the lockfile and the changelog.

Rollout I would propose: engine behind a normal cargo feature (default off) once G1, G2, G5
land; default on only after items 1 to 8 hold.

## 7. Decisions needed from the owner

- **D1. TLS 1.2.** Implement it (about 30 to 40 days, new record layer and key schedule, new
  downgrade surface), or accept a 1.3-only native engine and keep rustls for consumers that
  reach 1.2-only peers? This is the largest scope decision and ADR-0002 explicitly left it
  open pending a named consumer. I recommend measuring `rusty_rdp` and the homelab crates
  against real targets first.
- **D2. Replace `ring`.** My recommendation is no, and to say so in the new ADR. Removing
  rustls, webpki and `rustls-pki-types` is achievable. Hand-rolling AEAD, ECDSA, RSA and
  X25519 cannot be validated by any technique on the bar above, because the property that
  matters (constant time) is invisible to all of them. The workspace sovereignty ADR already
  says TLS and crypto replacement needs its own forcing function and evidence program.
  Consequence: the crate stays Tier A, and "rustls-free" is the honest goal, not "ring-free".
- **D3. Supersede the crate ADR-0002.** Only after D1 and D2, and with the mechanism in
  section 6 item 9 written down.

## 8. Limits of this assessment

- **rusty_tls#25 could not be read.** The issue lives in `baileyrd/rusty_tls`, which is archived
  and outside this session's repository scope; attaching it was denied. `rusty_mill#25` is an
  unrelated import PR. I used the shipping bar that ADR-0002 section 5 quotes from the issue
  ("unchanged"), and any additional acceptance criteria in the issue are unknown to me.
- Interop tests were not run; the unreviewed modules were not read in full; the F2 to F9
  findings are from reading, not from tests; effort figures are estimates.
- The F1 reproduction used rcgen-generated keys and one wildcard form. Other wildcard and
  constraint combinations were not exercised.
