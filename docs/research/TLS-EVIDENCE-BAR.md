# TLS native engine: evidence bar proposal (owner to approve)

Date: 2026-10-10. Status: **Tier 1 approved by the owner on 2026-10-10 (with the fuzz-length change below); Tier 2 not approved.** Open: reviewer (item 4), ADR amendment text (item 7). Follows
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
| CI runs the engine on every PR (clippy, tests, docs, OpenSSL interop, zero-tests guard) | Met |
| TLS 1.2 client and server | Met (stages 4b onward) |
| BoGo through a shim | 861 pass, 0 fail, 455 disabled with reasons; CI floor 830 |
| OpenSSL interop in both directions, hermetic, in CI | Met |
| rustls differential, protocol side (suites, groups, HRR, resumption, ALPN, SNI, both roles) | Met by per-stage suites; not re-audited as a whole |
| Certificate differential | **Partial.** Name constraints and the OS trust store only. No x509-limbo or BetterTLS corpus |
| Fuzz targets | **Exist** (certificate, DER, TLS 1.2 record, schedule, client and server, version negotiation). **Smoke job added 2026-10-10** (30 s per target, per PR; not yet run on a runner). No long run recorded. No target for TLS 1.3 handshake messages or the 1.3 machines beyond negotiation |
| Live internet | **One runner run, three Google hosts, TLS 1.3, one network.** The step is non-blocking and now runs on every CI pass |
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
3. **Fuzz smoke in CI.** Each existing target runs for a fixed budget on every PR, and one
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
2. Candidate: `rleval-app` OIDC transport (my recommendation; not contradicted).
3. ADR: amendment before any wiring (my recommendation); **text not yet written or approved.**
4. Reviewer: **open.** Without one Tier 1 stalls at item 4.
5. `ring`: keep (my recommendation).

## Limits

Statuses come from this session's runs and the repo as of 2026-10-10. I did not re-run every
suite for this note. Effort figures are rough (plus or minus 50 percent).
