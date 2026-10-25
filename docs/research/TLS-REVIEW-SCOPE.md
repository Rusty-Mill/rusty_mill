# TLS native engine: smaller independent review scope (approved 2026-10-16; review not started)

Date: 2026-10-11. Status: **scope approved by the owner 2026-10-16, widened by the owner 2026-10-24 to add `handrolled/crypto/native.rs` (ADR-0002 Amendment 2); reviewer: David Bailey, confirmed independent by the owner 2026-10-17; review not started.** Recorded from the owner's message in the working session (originally a draft). Time box set by the owner 2026-10-17: three weeks, about 40 hours of review effort, in the priority order below; open findings at the end leave the item unmet. Still open: the fuzz long-run length (the reviewer's) and the review itself. Naming a reviewer is not a review and authorizes no wiring. Supports Tier 1 item 4 of
`TLS-EVIDENCE-BAR.md`. The owner asked for a smaller scope than a review of the whole
engine. This is the code the Tier 1 consumer (`rleval-app` OIDC, named by the owner on 2026-10-14, a TLS 1.3 client to
named Google hosts) actually executes, and what is left out and why.

## In scope: about 9,000 lines (client path only)

| Area | File (`crates/libs/net/rusty_tls/src/`) | Lines | Why it matters for a client |
| --- | --- | --- | --- |
| DER reading | `handrolled/der.rs` | 494 | Parses hostile bytes from the peer's certificates |
| Certificate parse | `handrolled/x509.rs` | 926 | Same; decides what a certificate says |
| Signature verify | `handrolled/verify.rs` | 833 | Decides who signed what |
| Path validation | `handrolled/path.rs` | 635 | Decides whether to believe the chain |
| Names and constraints | `handrolled/name.rs` | 582 | Decides which host a certificate authenticates (the F1 bug lived here) |
| Wire and handshake messages | `handrolled/wire.rs`, `handrolled/handshake.rs` | 322 + 1287 | Parse hostile handshake bytes |
| TLS 1.3 client machine | `handrolled/client.rs` | 2796 | Flight ordering, HelloRetryRequest, CertificateVerify, Finished |
| TLS 1.3 record layer | `handrolled/record.rs` | 646 | Decrypt and reject; the only place plaintext is released |
| Key schedule and key exchange | `handrolled/schedule.rs`, `handrolled/kx.rs` | 441 + 210 | Derives every key |
| Limits | `handrolled/limits.rs` | 139 | Resource bounds on hostile input |
| Blocking stream | `handrolled/stream.rs` | 493 | The adapter the consumer would call; `close_notify`, truncation, permanent failure |
| Trust loading | `trust.rs` | 240 | Which anchors are trusted (OS store reader) |
| Native crypto seam (added 2026-10-24, ADR-0002 Amendment 2) | `handrolled/crypto/native.rs` | 628 | On the native backend, every hash, HMAC, AEAD, key exchange, signature check and random byte goes through it to the first-party crates. The primitives themselves rest on their own records (plan section 11b) |

Suggested priority if time is short: `name.rs`, `path.rs`, `verify.rs`, then the client
machine's CertificateVerify and Finished handling, then `stream.rs`.

## Out of scope for Tier 1, with the reason

| Area | Lines | Reason |
| --- | --- | --- |
| Server: `server.rs`, `server12.rs` | 2132 + 842 | The candidate is a client |
| TLS 1.2: `client12.rs`, `handshake12.rs`, `record12.rs`, `schedule12.rs` | 1289 + 453 + 431 + 320 | Kept out by `NativeTlsStream::tls13_only()`; see the resolved point below |
| Tickets and resumption: `ticket.rs` | 459 | The candidate makes one request per connection; resumption is not used |
| Signing: `sign.rs` | 245 | Only the server and client-auth sign; a client without client certificates does not |
| Exporters: `export.rs` | 122 | Not used |
| Version negotiation: `negotiate.rs` | 590 | Read only the part that rejects a downgrade sentinel if time allows |

## Resolved point (found while scoping; decided 2026-10-14 and 2026-10-16)

`NativeTlsStream` currently negotiates both TLS 1.3 and TLS 1.2 (it drives the dual-version
client). A peer that answers with 1.2 would run the 1.2 client code, which this scope leaves
out. Two ways to make the scope true:

1. **Add a TLS 1.3-only option to `NativeTlsStream`** (small change inside the gated
   module; the consumer sets it). Then the 1.2 client is genuinely not executed. My
   recommendation. **Built** as `NativeTlsStream::tls13_only()`, on the owner's request
   on 2026-10-14 (merged in #716). The owner approved this scope on 2026-10-16, which holds only while
   the consumer sets `tls13_only()` when it is wired.
2. **Widen the scope** to the four TLS 1.2 client files (about 2,500 lines).

## What the reviewer receives

- This table, the evidence bar, and `TLS-ENGINE-ASSESSMENT.md` (F1 and the smaller findings).
- The test inventory: BoGo 861 pass and 455 disabled with reasons, OpenSSL interop in CI, the
  per-stage differential suites against rustls, 8 fuzz targets with a 30 s smoke job.
- A list of what has **not** been checked: the x509-limbo differential is pinned and run, but the BetterTLS and limbo `pedantic` classes and CRLs are enforced by neither engine, and
  the `nc-dos` bound counts comparison attempts, not CPU or time; no long fuzz
  run, no constant-time evidence for `ring`'s primitives (not ours).

## Questions for the owner

1. Approve this scope as the Tier 1 review, or change it? **Approved 2026-10-16.**
2. TLS 1.3-only option (1) or widen the scope (2)? **Option 1**, built as `NativeTlsStream::tls13_only()` (merged in #716). The four TLS 1.2 client files stay out of scope, on condition that the consumer sets the option when it is wired.
3. Who reviews, and how long (the fuzz long-run length is theirs to set)? **Reviewer: David Bailey** (named by the owner 2026-10-16, confirmed independent 2026-10-17). **Time box: three weeks, about 40 hours** (owner, 2026-10-17). The fuzz long-run length is the reviewer's to set at the start.

## Limits

Line counts are from the tree at 2026-10-11 and include tests-in-file and comments. The
in-scope total is a sum of the rows above. The scope reflects my reading of which code the
candidate executes; the reviewer should confirm it.
