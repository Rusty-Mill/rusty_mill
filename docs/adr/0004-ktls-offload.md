# ADR-0004: kTLS offload — scope, seam, and the questions that block it

Status: **Proposed** — not accepted. Two of the decisions below belong to a
person, and one belongs to another repository. Written so they can be made
rather than drifted into.
Date: 2026-08-03

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

If accepted as recommended, this ADR **closes nothing and unblocks the
cheapest work**: a capability probe that reports what a machine can do, with no
data path and no API change, is honest and self-contained. Everything past it
waits for a named consumer.

If overruled and the work proceeds, the order is forced: D1 (policy) → probe →
TX → RX, because each later stage depends on the earlier one's answer.

**Foreclosed either way:** nothing. This ADR records decisions; it removes no
option that is currently available.
