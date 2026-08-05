# ADR-0003: Session resumption without 0-RTT

Status: Accepted
Date: 2026-08-03

## Context

`rusty_tls#43` asked for session resumption in the hand-rolled engine: tickets,
PSK, and 0-RTT. Those three arrive together in RFC 8446 and are usually spoken
of as one feature, but they do not carry the same risk, and treating them as one
decision is how the dangerous half gets built by momentum.

Two things forced the issue now rather than later.

**A gap that made a test lie.** The client does not offer
`psk_key_exchange_modes`, so by §4.2.9 no conforming server ever sends a
NewSessionTicket. A test asserting that tickets were handled correctly was
green and vacuous — the branch it covered never ran, and a mutation returning a
ticket as *application data* survived it. That was patched by having a test
server send one unconditionally, but the underlying absence is real: the client
cannot ask, so nothing in normal operation produces one.

**0-RTT is a different kind of feature.** Early data is replay-safe only if the
application above it is. A request that reads is fine to replay; one that
charges a card is not. TLS cannot tell them apart, and nothing in the protocol
lets it.

## Decision

**Resumption is in scope. 0-RTT is out, and stays out until a named consumer
asks for it and an anti-replay design lands in a follow-up ADR.**

Concretely, in scope:

- `psk_key_exchange_modes` offering `psk_dhe_ke`, so a server may send tickets.
- NewSessionTicket handled and surfaced to the caller as a resumable session.
- `pre_shared_key` offered on a later connection, with binders computed over
  the truncated ClientHello.
- The server side of both: issuing tickets and accepting a PSK.

Out of scope, deliberately:

- **`early_data` in any form.** Not offered by the client, not accepted by the
  server, and an `early_data` extension in a ClientHello is refused rather than
  ignored.
- **`psk_ke` — resumption without a fresh key exchange.** Only `psk_dhe_ke` is
  offered. Resuming without new key material means the session's forward
  secrecy is only as good as the ticket's lifetime, and the saving is one
  key exchange.

## Alternatives considered

**Build all three, and document 0-RTT as "use at your own risk."** Rejected.
The risk is not the user's to assess per-connection: it depends on whether
*every* request the application might send early is idempotent, which is not a
property most applications know about themselves. A warning in a doc comment
does not make a replayed payment idempotent.

**Build 0-RTT with single-use tickets only.** This is a real anti-replay
mechanism and it is not sufficient: it stops replay against *one* server, and a
deployment with more than one needs shared state between them — which is a
distributed-systems problem this crate has no business solving on an
application's behalf. If 0-RTT is ever built, the strike-register design has to
come first and be written down, which is what the follow-up ADR is for.

**Leave resumption out too, and close #43.** Tempting, since this engine is not
the default and never will be (ADR-0002), so the performance argument barely
applies. Rejected on the strength of the vacuous-test finding: the ticket path
exists in the code and cannot currently be reached, and unreachable code in a
security-critical state machine is worse than either building it or deleting
it. Building it is the option that also closes the gap the test was pretending
to cover.

**Offer `psk_ke` as well as `psk_dhe_ke`.** Rejected. It exists to save a key
exchange, and this engine is an experiment where a saved key exchange is worth
nothing and a weakened forward-secrecy story is worth something.

## Consequences

**Accepted:**

- A resumed handshake is a second code path through the key schedule — the
  early secret comes from the PSK rather than from zeroes — and it has to be
  tested as its own thing rather than assumed to follow from the full
  handshake working.
- Binders are computed over a *truncated* ClientHello, which means
  `pre_shared_key` must be the last extension and the hello has to be
  serialisable in two pieces. That is an awkward shape and it is the RFC's,
  not a choice available to be made differently.
- The server now holds state that outlives a connection: a key that seals
  tickets. A ticket is an encrypted copy of authentication state, so that key
  is as sensitive as the certificate's private key, and rotating it is a
  deployment concern this crate must document rather than hide.

**Foreclosed, for now:**

- 0-RTT, and with it the latency win that is most of the reason people ask for
  resumption at all. Anyone who needs it needs the follow-up ADR first.

  "For now" still means for now. What changed on 2026-08-05 is only that this
  is no longer *tracked* as pending work — see the `#58` entry below. The
  engine refuses `early_data` on both halves rather than ignoring it, which is
  what keeps the decision visible in the code rather than only here.

**Created:**

- [`rusty_tls#58`](https://github.com/baileyrd/rusty_tls/issues/58) — 0-RTT and
  its anti-replay design. **Filed, considered, and closed as `not planned` on
  2026-08-05.** It is linked here as the record of that decision, not as
  something tracking work.

  **The gate in the Decision section above is unchanged.** 0-RTT stays out
  until a named consumer asks for it *and* an anti-replay design lands in its
  own ADR. Closing the issue does not move that gate; it stops an open issue
  implying an intent nobody holds. Reopening `#58` is the way back in, and its
  closing comment records what a revival would already have to hand and the two
  things it should not rediscover the hard way — that stateless resumption and
  single-use tickets conflict, and that anti-replay cannot be tested against a
  correct peer, because a correct peer never replays.

  The reason for closing is the **absence of a named consumer**, not the
  difficulty. ADR-0002 makes this engine permanently non-default; 0-RTT's whole
  value is latency; a non-default experiment gains close to nothing from a
  saved round trip and would take on the one TLS 1.3 feature whose security
  property RFC 8446 itself only bounds rather than eliminates.

  **Corrected 2026-08-04.** This bullet originally claimed the follow-up issue
  had been created. It had not been, and nothing checked the claim until
  `#43`'s resumption work landed in `0.9.0` and the ADR was read back against
  the issue list. The issue exists now.

  Worth recording rather than quietly fixing, because the failure is one this
  repo keeps meeting in a different costume: a document asserting coverage it
  does not have, with nothing that would notice. It is the same shape as the
  vacuous green tests `#25` records and the shape-not-value binder tests `#43`
  had to unpick — and here it mattered, because this bullet is what made 0-RTT
  *deferred* rather than silently *dropped*. For eight days the deferral had no
  tracked home, while the engine's own module docs pointed at this ADR and this
  ADR pointed at nothing.
