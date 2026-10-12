# TLS native engine: evidence bar proposal (owner to approve)

Date: 2026-10-10. Status: **Tier 1 approved by the owner on 2026-10-10 (with the fuzz-length change below); Tier 2 not approved.** Open: reviewer (item 4); review scope draft awaiting approval; `anyExtendedKeyUsage` and `nc-dos` are unresolved pre-wiring blockers. Follows
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
| Certificate differential | **Partial, much improved 2026-10-11.** x509-limbo (9,802 cases, pinned by sha256) now runs against both engines: 9,777 compared. It found three real looser-than-webpki defects (fixed, see below). Remaining: the BetterTLS and limbo `pedantic` classes and CRLs are not enforced by either engine; two name-constraint DoS cases are accepted by us and refused by webpki (owner decision below). No generated-chain corpus at scale yet |
| Fuzz targets | **Exist** (certificate, DER, TLS 1.2 record, schedule, client and server, version negotiation). **Smoke job added 2026-10-10** (30 s per target, on the same planner selection as the engine job; passed once on a runner, run 38094570843). No long run recorded. No target for TLS 1.3 handshake messages or the 1.3 machines beyond negotiation |
| Live internet | **One runner run, three Google hosts, TLS 1.3, one network.** The step is non-blocking and runs wherever the engine job runs (same planner selection) |
| Independent review | **None** |
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
4. **Review.** An independent review of `x509`, `verify`, `name`, `sign` and the 1.3 client
   state machine, findings closed. *This session cannot supply it.* Alternative for your
   call: a time-boxed review by a second engineer or a paid reviewer.
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
2. Candidate: `rleval-app` OIDC transport is **my recommendation, not a chosen consumer**; the owner has not confirmed it.
3. ADR: amendment before any wiring (my recommendation); **text not yet written or approved.**
4. Reviewer: **open.** Without one Tier 1 stalls at item 4.
5. `ring`: keep (my recommendation).
6. **Proposed, pending confirmation:** leave it. Tier 1 item 1 counts *scheduled* runs and the only
   scheduled CI run today is weekly (Mondays 03:17 UTC), so the criterion takes 10 weeks as
   written. No schedule is added and the criterion is unchanged.
7. **Smaller review scope:** drafted in `TLS-REVIEW-SCOPE.md` for the owner's approval.
8. **ADR amendment:** approach and text **pending confirmation**; proposed as Amendment 1 in
   `crates/libs/net/rusty_tls/docs/adr/0002-handrolled-engine-behind-a-permanently-non-default-seam.md`
   (marked Proposed; it adds no default, wires nothing and is not operative).

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
refuse, except two.** The two are `pathological::nc-dos-1` and `-2`: a root with 2,048 name
constraints and a leaf with 2,048 names. Limbo calls them technically valid and "typically
rejected by implementations due to quadratic blowup". This validator accepts them; webpki
refuses. **Owner decision needed.** Fixture measurement only: the two cases together took 0.65 s
in a debug test build, including parsing and webpki's refusal; their certificates are about 77 KB
of DER. That is two data points, not a bound. This validator has no name-comparison budget, and
path search can repeat the comparison across candidate paths, so I cannot say what the worst
handshake costs. Accepting them needs an explicit resource bound (for example a comparison
budget in `check_name_constraints`, which I can add) or broader adversarial evidence; I do not
recommend accepting them on the timing above.

**A second looser-than-webpki behavior, not covered by limbo, owner decision needed:** a
certificate or CA whose `extendedKeyUsage` is `anyExtendedKeyUsage` alone is accepted for server
authentication here (RFC 5280 §4.2.1.12 reads it as any purpose) and refused by webpki 0.103
(it requires the specific `serverAuth` OID whenever the extension is present, on the leaf and on
every issuer). Pinned by `any_extended_key_usage_is_accepted_here_and_refused_by_webpki`.
Aligning is a one-line change in `check_eku` plus reversing an existing, tested leaf
expectation; I recommend aligning (parity with the engine this would replace, and it is the
tightening direction), but it reverses a documented choice, so it waits for the owner.

Other divergences, none a defect in this validator:
- 2 cases where this validator is *looser* than webpki and agrees with limbo (webpki is the stricter one): `pathlen::self-issued-certs-pathlen`, `rfc5280::nc::permitted-self-issued`.
- 4 cases where this validator is *stricter* and limbo expects refusal (empty `permitted`/`excluded` sequences, a leading-period DNS constraint).
- 1 false refusal: `rfc5280::nc::nc-forbids-othername-noop`. An `otherName` name constraint is refused rather than ignored by design (ADR-0002 section 2b-iii).
- 57 cases both engines accept that limbo says to refuse: 10 CRL cases (neither engine implements revocation, finding F8), and mostly `pedantic` issuance-profile lints (AKI/SKI presence, serial number size, CN versus SAN, weak or unusual subscriber keys, public-suffix wildcards) and trust-anchor properties (anchors are taken as name and key only, by design). Listed in `tests/data/limbo_divergences.txt`; the ratchet fails if this list changes.

The golden also pins `compared`, every `skipped` reason and count, the count per outcome, and a
digest of every case's id and outcome, so lost coverage or a flip among cases that agree is a diff
(unit-tested in `handrolled_limbo.rs` without the corpus). The CI step fails, rather than skips, if
the pinned commit cannot be fetched or its hash differs. How to run, bump the pin and bless a
change: `.github/scripts/limbo_differential.sh` and the header of `tests/data/limbo_divergences.txt`.
