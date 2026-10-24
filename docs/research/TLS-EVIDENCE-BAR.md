# TLS native engine: evidence bar proposal (owner to approve)

Date: 2026-10-10. Status: **Tier 1 approved by the owner on 2026-10-10 (with the fuzz-length change below); Tier 2 not approved.** Review scope approved and reviewer nominated (David Bailey) by the owner on 2026-10-16, recorded from the owner's session message; on 2026-10-17 the owner confirmed the nominee as the independent reviewer; time box set by the owner on 2026-10-17 (three weeks, about 40 hours of review effort); still open: the review itself, which has not started (item 4); first consumer named and Amendment 1 accepted 2026-10-14 (wiring still gated on the unmet Tier 1 items); `anyExtendedKeyUsage` aligned 2026-10-12 and the two `nc-dos` cases bounded 2026-10-13; the two other native-accept/webpki-reject limbo cases are accepted exceptions (see "Accepted exceptions"; owner acceptance recorded 2026-10-13). Follows
`TLS-ENGINE-ASSESSMENT.md` section 6, which this narrows and updates. The engine's default,
both gates and every consumer are unchanged. The bar, replacing `ring`, and superseding
ADR-0002 are the owner's decisions.

## Why two tiers

The assessment set one bar, for making the engine the default. That bar is far away
(45 to 70 days plus an external review). Meanwhile one consumer could use the engine opt-in
and still fall back to rustls with one switch. The risk differs, so the bar should too.

- **Tier 1, opt-in consumer:** one named app enables the gated `NativeTlsStream` behind its
  own off-by-default feature. rustls stays its default. Nothing else changes.
- **Tier 2, default engine:** `TlsStream` and the seam types route through the engine.

## Where the engine stands (what I have run or read, not a claim of completeness)

| Item | Status |
| --- | --- |
| CI runs the engine (clippy, tests, docs, OpenSSL interop, zero-tests guard) whenever the planner selects `tls_engine`: a change to `rusty_tls` or to a crate it depends on (the planner selects by reverse dependency of what changed), a change to the engine's own CI files, and full runs (manual dispatch, the weekly schedule, and pushes with no usable base). Pushes to main are scoped like PRs. Not on unrelated changes | Met |
| TLS 1.2 client and server | Met (stages 4b onward) |
| BoGo through a shim | 861 pass, 0 fail, 455 disabled with reasons; CI floor 830 |
| OpenSSL interop in both directions, hermetic, in CI | Met |
| rustls differential, protocol side (suites, groups, HRR, resumption, ALPN, SNI, both roles) | Met by per-stage suites; not re-audited as a whole |
| Certificate differential | **Partial, much improved 2026-10-11.** x509-limbo (9,802 cases, pinned by sha256) now runs against both engines: 9,777 compared. It found three real looser-than-webpki defects (fixed, see below). Remaining: the BetterTLS and limbo `pedantic` classes and CRLs are not enforced by either engine; the two name-constraint DoS cases are now bounded by a comparison budget (see below), but two other limbo cases where we accept and webpki refuses still need a named-owner decision. No generated-chain corpus at scale yet |
| Fuzz targets | **Exist** (certificate, DER, TLS 1.2 record, schedule, client and server, version negotiation). **Smoke job added 2026-10-10** (30 s per target, on the same planner selection as the engine job; passed once on a runner, run 38094570843). No long run recorded. No target for TLS 1.3 handshake messages or the 1.3 machines beyond negotiation |
| Live internet | **One runner run, three Google hosts, TLS 1.3, one network.** The step is non-blocking and runs wherever the engine job runs (same planner selection) |
| Independent review | **Scope approved and reviewer named (David Bailey) by the owner, 2026-10-16; review not started, nothing reviewed yet.** Item 4 stays unmet until findings are closed |
| Soak in a real consumer | **None** |
| Mutation testing as a CI number | **None** (mutants are checked by hand per change) |
| RSA PKCS#1 v1.5 signing | **Not supported** (touches `sign.rs`) |
| Resumed-connection peer scheme | Not reportable (ticket format) |

## Proposed Tier 1 bar (gates wiring one opt-in consumer)

All hard gates unless marked. "Candidate" is `rleval-app`'s OIDC transport (Google sign-in).

1. **Repeated live evidence.** The live-internet step passes on 10 consecutive scheduled CI
   runs over at least 7 days, against the candidate's real hosts, with the differential
   against the rustls-backed stream agreeing. A failure resets the count and is investigated
   (an interception flag is a runner issue, anything else is an engine finding).
2. **Certificate differential, bounded.** A published external corpus (x509-limbo or
   equivalent) is run against both engines for the chain shapes the candidate meets (RSA and
   ECDSA leaves, two-intermediate chains, SAN names). Every divergence is listed. None is
   "looser than webpki" unless you accept it by name.
3. **Fuzz smoke in CI.** Each existing target runs for a fixed budget on every run the planner selects for the engine (see the status table), and one
   long run (for example 24 CPU hours) is clean once before wiring.
4. **Review.** An independent review of the code in the approved scope, findings closed.
   **`TLS-REVIEW-SCOPE.md` is the authoritative scope** (approved 2026-10-16): the client
   path only, with `sign.rs`, the servers, the TLS 1.2 client (kept out by
   `NativeTlsStream::tls13_only()`, which the consumer must set), tickets and exporters
   excluded. The file list first proposed here (`x509`, `verify`, `name`, `sign` and the 1.3
   client state machine) is superseded by it; in particular `sign.rs` is not required. *This
   session cannot supply the review.* The reviewer is David Bailey, confirmed independent by
   the owner on 2026-10-17, with a time box of three weeks and about 40 hours of review
   effort (question 4). The review itself and its findings remain open, and naming a
   reviewer is not a review.
5. **Switch.** The consumer's native path is a normal off-by-default cargo feature in that app,
   with a one-line way back to rustls and a changelog note. No library default moves.
6. **Scope statement.** The consumer is client-only, TLS 1.3 against named hosts. RSA
   PKCS#1 v1.5 and ticket-format work are not needed for it (to be re-checked if its
   hosts change).
7. **ADR.** You decide whether an app enabling the double gate needs an ADR-0002 amendment (as
   the MCP crates got Amendment 1) or a superseding ADR. I read section 3 as requiring at least
   an amendment; that reading is yours to confirm.

Rough effort: items 2 and 3 about 8 to 10 days; item 1 is elapsed time (7 days) with little work;
item 4 is external.

## Tier 2 bar (default engine)

Unchanged from assessment section 6 items 1 to 9, plus: the seam adapters (G5), a soak of two
releases in two or more consumers, and a superseding ADR with the replacement guarantee. Not
proposed to start until Tier 1 has been in use.

## Questions for you (answers recorded 2026-10-10)

1. Tier 1: **approved**, fuzz length left to the reviewer.
2. Candidate: `rleval-app` OIDC transport (client only, TLS 1.3, named hosts) **chosen by the owner on 2026-10-14**, in the working session ("3 approved recommendation"). Naming it wires nothing. **Chosen by the owner on 2026-10-24 for the native backend** (ADR-0002 Amendment 2). Accepted there by the owner the same day: before wiring on that backend, item 1's runs are counted in the native CI job, and item 4 also covers `handrolled/crypto/native.rs` (a widening of `TLS-REVIEW-SCOPE.md`).
3. ADR: amendment before any wiring. **Amendment 1 accepted by the owner on 2026-10-14** (same session message).
4. Reviewer: **David Bailey**, nominated by the owner on 2026-10-16 and **confirmed by the owner as independent on 2026-10-17** (session message: "1 yes"). **Time box set by the owner on 2026-10-17: three weeks, about 40 hours of review effort** (session message: "2: three weeks, 40 hours"), in the scope's priority order; findings must be closed for item 4 to be met, so running out of time with findings open leaves it unmet. Open: the review itself. Tier 1 item 4 stays unmet.
5. `ring`: keep (my recommendation).
6. **Leave it, confirmed by the owner on 2026-10-17** (session message: "3 confirmed"). Tier 1 item 1 counts *scheduled* runs and the only
   scheduled CI run today is weekly (Mondays 03:17 UTC), so the criterion takes 10 weeks as
   written. No schedule is added and the criterion is unchanged.
7. **Smaller review scope:** `TLS-REVIEW-SCOPE.md` **approved by the owner on 2026-10-16** (session message); it is the authoritative scope for item 4.
8. **ADR amendment:** approach and text **accepted by the owner on 2026-10-14**; Amendment 1 in
   `crates/libs/net/rusty_tls/docs/adr/0002-handrolled-engine-behind-a-permanently-non-default-seam.md`
   (it adds no default and wires nothing; wiring is a separate change that must cite met Tier 1 evidence).

## Limits

Statuses come from this session's runs and the repo as of 2026-10-10. I did not re-run every
suite for this note. Effort figures are rough (plus or minus 50 percent).

## Findings from the x509-limbo differential (2026-10-11)

Corpus: `limbo.json` from C2SP/x509-limbo at the immutable commit `554528a9b0c0d95e071f55de018326f0b65a8364` (sha256 `611e337b...cd4de`), 9,802 cases; 9,777 compared
(10 client-validation, 6 custom key-usage, 6 max-depth, 1 no peer name, 2 unparsable time
skipped). Cases with no stated time use one second after the newest `notBefore`; both engines
get the same instant. Webpki is `verify_server_name` plus `verify_server_cert_signed_by_trust_anchor`.

**Before any fix, 6 cases were looser than webpki and wrong by limbo's verdict; the first pass
with all times found 5 more.** Three root causes, all fixed in this change, each with a unit test
that fails without the fix:

| Defect | Cases | Fix |
| --- | --- | --- |
| `extendedKeyUsage` checked on the leaf only, so a CA limited to e-mail protection could issue server certificates | 6 BetterTLS `pathbuilding` | check it on every CA in the path |
| End-entity certificate with `basicConstraints` `cA` true accepted | `webpki::ee-basicconstraints-ca` | `PathError::EndEntityIsCa` |
| Malformed `subjectAltName` entry skipped under a name constraint | `nc-permits-invalid-dns-san`, `nc-permits-invalid-ip-san` | `NameError::MalformedSubjectAltName` |

**After the fixes: 0 limbo cases where this validator accepts something webpki refuses and limbo says to
refuse, except two, which are now fixed too (2026-10-13; the user chose "budget" in the working session, not yet confirmed directly to the reviewer).** The two are
`pathological::nc-dos-1` and `-2`: a root with 2,048 name constraints and a leaf with 2,048 names,
over four million name-against-subtree comparisons. Limbo calls them technically valid and "typically
rejected by implementations due to quadratic blowup"; webpki refuses them; this validator used to
accept after doing all the work (4 s with the cap removed, in a debug build). Now
`check_name_constraints_body` computes names times subtrees up front and refuses with
`NameError::TooManyNameComparisons` above `MAX_NAME_COMPARISONS` (100,000), before any comparison.
The value is my choice, between real chains and the limbo cases; neither the owner nor webpki specified it.
The cap is per call, one call per constraining CA per candidate path. Worst case, then: at most the
signature-check budget of candidate paths (default 100) times the path depth plus one (default 9)
times the cap, about 9 x 10^7 logical name-against-subtree comparison attempts, and only for a peer that builds that many validly
signed candidate paths. That counts attempts, not CPU or bytes: parsing, allocation, scanning and variable-length string comparison sit outside it, so it is neither a measurement nor a claim about wall-clock time.
Three tests pin it: the exact boundary (100,000 accepted, 100,001 refused) and the limbo shape;
permitted subtrees counting towards the product; and names aggregated across a path, with a
self-issued intermediate exempt. Each was mutation-checked (`>=` for `>`, counting the self-issued
certificate, counting only the last certificate, ignoring permitted subtrees: each fails a test). The limbo golden lost the two `looser` lines and nothing else moved.

**A second looser-than-webpki behavior, not covered by limbo, fixed 2026-10-12 (owner decision):**
a certificate or CA whose `extendedKeyUsage` is `anyExtendedKeyUsage` alone was accepted for server
authentication (RFC 5280 §4.2.1.12 reads it as any purpose) while webpki 0.103 refuses it (it
requires the specific `serverAuth` OID whenever the extension is present, on the leaf and on every
issuer). The owner chose to align with webpki. `check_eku` no longer treats the OID as a wildcard;
`any_extended_key_usage_alone_does_not_satisfy_the_requirement` and
`any_extended_key_usage_is_refused_like_webpki` (leaf and intermediate, ours and webpki) pin it. The
limbo golden is unchanged (the corpus has no such case).

**Accepted exceptions (Tier 1 item 2).** Accepted by the owner in the working session on 2026-10-13: "Confirmed, I accept both exceptions", given after the two cases were named. The reviewer separately reported a matching owner confirmation on PR #670; that report is corroboration only, and this record rests on the owner's message in the session. Two cases
where this validator is *looser* than webpki and agrees with limbo:

| Case | Why this validator follows the standard |
| --- | --- |
| `pathlen::self-issued-certs-pathlen` | RFC 5280 §6.1.4(l): a self-issued certificate does not consume path length. webpki counts it. |
| `rfc5280::nc::permitted-self-issued` | RFC 5280 §6.1.3(b) (permitted subtrees) and §6.1.3(c) (excluded subtrees): name constraints are not checked against a self-issued certificate unless it is the final certificate. webpki applies them. |

Basis: the user's direction in the working session on 2026-10-13, "Always err on the side of the
standards", read as the RFCs plus the Web PKI profile for server authentication, then the acceptance above. Both behaviors are in the code with the RFC section
cited. This is not inferred from limbo expecting success. The acceptance covers exactly these two
cases as named Tier 1 exceptions and nothing broader: no consumer wiring, no broader
standards-preference policy, no reversal of the #628 `anyExtendedKeyUsage` restriction, no
unrelated ADR. Any other native-accept/webpki-reject case found later needs its own decision.

On `anyExtendedKeyUsage`: RFC 5280 §4.2.1.12 permits a validator to restrict it, so refusing it
(as merged in #628) is within the standard. Favoring the standards does not require reversing #628.

Other divergences, none a defect in this validator:
- 4 cases where this validator is *stricter* and limbo expects refusal (empty `permitted`/`excluded` sequences, a leading-period DNS constraint).
- 1 false refusal: `rfc5280::nc::nc-forbids-othername-noop`. An `otherName` name constraint is refused rather than ignored by design (ADR-0002 section 2b-iii).
- 57 cases both engines accept that limbo says to refuse: 10 CRL cases (neither engine implements revocation, finding F8), and mostly `pedantic` issuance-profile lints (AKI/SKI presence, serial number size, CN versus SAN, weak or unusual subscriber keys, public-suffix wildcards) and trust-anchor properties (anchors are taken as name and key only, by design). Listed in `tests/data/limbo_divergences.txt`; the ratchet fails if this list changes.

The golden also pins `compared`, every `skipped` reason and count, the count per outcome, and a
digest of every case's id and outcome, so lost coverage or a flip among cases that agree is a diff
(unit-tested in `handrolled_limbo.rs` without the corpus). The CI step fails, rather than skips, if
the pinned commit cannot be fetched or its hash differs. How to run, bump the pin and bless a
change: `.github/scripts/limbo_differential.sh` and the header of `tests/data/limbo_divergences.txt`.
