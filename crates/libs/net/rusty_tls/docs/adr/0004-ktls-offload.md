# ADR-0004: kTLS offload — scope, seam, and the questions that block it

Status: **Accepted** — 2026-08-05, as recommended. D1, D2 and D3 were the
decisions reserved for a person; all three are accepted in the form proposed
below. D4 was settled on evidence and has since been re-measured.
Date: 2026-08-03
Accepted: 2026-08-05

`rusty_tls#14` is closed as `not planned` under D3. That closes the *tracking*,
not the option: this ADR is now the record, and it forecloses nothing. If a
consumer with a measured bottleneck ever appears, reopening #14 and following
D1 → probe → TX → RX is the path, and every decision it needs is already made
below.

## Context

`rusty_tls#14` tracks Linux kernel TLS offload. It is labelled `needs-human`
and its own body says why: kTLS is Linux-only, involves raw socket options and
capability detection, and is "closer to a small subsystem than a tight group of
related functions".

Investigation (recorded in full on the issue) turned up four findings. Two of
them were not what the issue anticipated, and they are what this ADR exists to
settle.

**1. The blocker is the type signature, not the syscall.**

```rust
pub struct TlsStream<S> { … }
impl<S: Read + Write> TlsStream<S> { … }
```

The crate is generic over anything readable and writable. **There is no file
descriptor anywhere in it** — by design, and it is what makes the test suite
hermetic. kTLS needs `setsockopt` on a real socket.

**2. `rustls` supports the handoff, and it is a one-way door.**
`ClientConfig::enable_secret_extraction` plus
`Connection::dangerous_extract_secrets()` yield `ExtractedSecrets`, and rustls'
own docs name kTLS as the reason. But the method takes `self`: once the secrets
are out, **there is no rustls left** to process post-handshake messages.

**3. It cannot live in rustils' `platform`.** That crate's consumer gate
(`docs/rfc-v2.md` §3) lists `net` as parked with no named consumer, and the
layer is a portable trait surface every backend implements with parity tests
asserting they agree. kTLS has no Windows or BSD counterpart.

**4. A modern kernel is not sufficient.** Measured here: kernel 6.18.5, and
`setsockopt(SOL_TCP, TCP_ULP, "tls")` returns **ENOENT** because the `tls`
module is absent and a container generally cannot load it.

Re-measured 2026-08-05 on a second machine before accepting: kernel
`6.18.5-fc-v18`, no `/sys/module/tls`, no `tls` in `/proc/modules`, and the
probe against a live socket returns `ENOENT` again. Two independent runs, same
answer — which is what D4 rests on, and the reason it is the one decision here
that was never anyone's to prefer.

## Decisions proposed

### D1 — The post-handshake policy (mine to recommend, yours to accept)

**KeyUpdate must be handled before any offload ships.** RFC 8446 §4.6.3
requires responding to one. The kernel surfaces it as a control message rather
than acting on it, and with the rustls connection consumed there is nothing
left that knows how to rekey. A peer that sends KeyUpdate to a naive
implementation gets silence.

Proposed: **an offloaded connection that receives a KeyUpdate tears down rather
than ignoring it**, until rekeying is implemented. A connection that fails
loudly is worse than one that works and better than one that silently stops
being confidential.

`NewSessionTicket` and `close_notify` arrive on the same control path and need
the same explicit answer.

### D2 — Where the fd comes from (yours; it is an API decision)

Three options, none free:

| Option | Cost |
|---|---|
| Add `AsRawFd` to the existing bound | A Linux-only bound on a portable type; every in-memory test stops compiling |
| A separate `KtlsStream` requiring a real socket | Duplicates the drive loop `ARCHITECTURE.md` deliberately declined to unify |
| Runtime downcast to recover the socket | Defeats the generic, and fails silently when it does not match |

**Recommended: the separate type.** It keeps `TlsStream` portable and honest,
and confines a Linux-only concern to a Linux-only type. It is also the option
`ARCHITECTURE.md`'s existing decision not to share the blocking and async drive
loops already points at.

### D3 — Whether to build it at all (yours, and it is a consumer-gate question)

**Recommended: not yet.** Both repos run a consumer gate and kTLS has no named
consumer; `ARCHITECTURE.md` lists it under Non-goals with exactly that
condition attached.

The performance case is also weaker here than it looks. kTLS's real payoff is
zero-copy — `sendfile`/`splice` straight from page cache to an encrypted
socket. **This crate has no API that could express that:** `TlsStream` is
`Read + Write` over a generic, so bytes are already in userspace by the time
they arrive. Without a zero-copy path, kTLS buys a context-switch reduction and
costs a Linux-only branch through the most security-sensitive code here.

### D4 — Detection is a probe, never a version check

Finding 4 settles this one on evidence rather than preference. Detection must
**attempt `TCP_ULP` and handle `ENOENT`**, and the fallback is the common case
in containers — which is where this crate's consumers actually run. The
fallback is not an edge case to tidy up last; it is the default path.

## Alternatives considered

**Put it behind `platform` anyway, with non-Linux backends returning
`Unsupported`.** Rejected: that is precisely the shape rustils' parity suite
exists to make suspicious, and it would need an amendment to that repo's §3
table. If kTLS is ever built, it belongs in a Linux-only seam here, not in the
portable layer there.

**Ship TX-only offload and defer the control-message problem.** Tempting,
because TX alone is coherent and is where the payoff is. Rejected as a
*starting* point: TX still shares the connection with an RX path that must
handle KeyUpdate, so D1 has to be answered first either way.

## Consequences

**Accepted:**

- D1, D2 and D3 are decided in the form proposed. D3 is the operative one:
  **not yet**, on the consumer gate `ARCHITECTURE.md` already applies to this
  exact item.
- The capability probe this ADR described as "the cheapest work" is **not**
  built. It would be public surface for a feature nobody is building, in a
  crate whose Non-goals section runs a consumer gate — and `rusty_tls#25`'s
  most expensive lesson was about surface that exists without being reachable.
  Cheap is not the same as warranted.
- `rusty_tls#14` is closed as `not planned`. An open issue implies intent, and
  after `#58` (0-RTT) and `#41` (TLS 1.2) closed under the same gate, leaving
  this one open would have implied a distinction that is not being drawn.

**Foreclosed:** nothing. This ADR records decisions; it removes no option that
is currently available, and D1–D4 are exactly what a revival would otherwise
have to work out from scratch.

**Where the record now lives.** `ARCHITECTURE.md`'s Non-goals entry, and this
ADR. Between them they carry more than the issue did: the Non-goals line states
the gate, and this states the four decisions that gate was hiding — including
the one that matters most, which is that **kTLS's payoff is zero-copy and this
crate has no API that could express it.** `TlsStream` is `Read + Write` over a
generic, so the bytes are already in userspace. That is not a scheduling
problem and no consumer changes it.

**Left for whoever revives this.** The rustils governance question in finding 3
is still open and still theirs: kTLS does not fit `platform`'s portable trait
surface, and saying so is an amendment to that repo's `docs/rfc-v2.md` §3
rather than a decision this ADR can make.
