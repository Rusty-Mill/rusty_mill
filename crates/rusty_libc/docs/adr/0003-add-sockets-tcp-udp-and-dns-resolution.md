# ADR-0003: Add a sockets primitive — TCP, UDP, and DNS resolution

Status: Accepted
Date: 2026-07-23

## Context

REVIEW.md Round 4 raised a design question rather than a scored gap (see
[issue #72](https://github.com/baileyrd/rusty_libc/issues/72), same spirit as
#33's `signalfd` question): bash and several bash-compatible shells support
`exec 3<>/dev/tcp/host/port`-style redirection, which needs a real sockets
API (`socket`, `connect`, and — since `/dev/tcp/host/port` takes a hostname,
not just a numeric address — some form of DNS resolution) at minimum. This
crate has zero networking surface today, and Round 4's own review found no
evidence `rush` currently implements `/dev/tcp` redirection.

An earlier draft of this ADR (and of the PR that shipped it) proposed
deferring: no confirmed consumer need, so no speculative surface, mirroring
how Round 4's "Considered and not proposed" list treats `flock`, `chroot`/
namespaces, and similar categories. That default was overridden: the
decision is to build the primitive now rather than wait for `rush` to ask
for it first.

## Decision

**Add a sockets primitive covering TCP, UDP, and DNS-name resolution**,
scoped to what a job-control shell's `/dev/tcp`/`/dev/udp` redirection (and
similar future networking needs) actually requires:

1. **`socket` module** — the raw primitives on top of the kernel's own
   `sockaddr_in`/`sockaddr_in6` layouts (matching this crate's
   kernel-layout-not-glibc-layout convention throughout): `socket`, `bind`,
   `connect`, `listen`, `accept`/`accept4`, `send`/`recv`, `sendto`/
   `recvfrom`, `shutdown`, plus `AF_INET`/`AF_INET6`/`SOCK_STREAM`/
   `SOCK_DGRAM` and the handful of `SOCK_*`/`SHUT_*` constants needed to
   drive them. IPv4 and IPv6 both, from the start — treating IPv6 as an
   afterthought here would just mean redoing the address-family plumbing
   later.
2. **`dns` module** — a minimal stub resolver: read the nameserver list from
   `/etc/resolv.conf`, build and send an A (and, for IPv6 destinations,
   AAAA) query over UDP port 53, parse the response into a list of
   addresses. This crate has no libc to hand `getaddrinfo` off to and no
   dependencies to pull one in from, so resolution has to be implemented
   directly against the DNS wire format (RFC 1035) — a bounded, well-
   specified format, not an open-ended one.
3. **Error mapping**: DNS failures (`NXDOMAIN`, timeout, malformed
   response, no nameserver configured) do not fit the kernel's `-errno`
   convention `Errno` wraps — there's no syscall on the failing side, just a
   UDP round trip that didn't come back with a valid answer. These map onto
   the *existing* `Errno` space using the closest fit
   (`ENOENT`/`ETIMEDOUT`/`EINVAL` respectively) rather than inventing a
   second, parallel error type — one error currency for the whole crate,
   consistent with how every other module already reports failure.

Blocking semantics by default (`connect`/`recv`/etc. behave as ordinary
blocking syscalls unless the caller opens the socket with `SOCK_NONBLOCK`),
matching this crate's existing fd/process primitives, none of which impose
an async runtime.

## Alternatives considered

- **Defer until a confirmed consumer need exists.** This was the original
  position (see above) — overridden. Recorded here rather than silently
  dropped, since the reasoning for the reversal ("build ahead of the
  confirmed need this time") is itself worth having on record, the same way
  every other decision in this crate's ADR log is.
- **TCP only, defer UDP and DNS.** Rejected: `/dev/udp/host/port` is the
  direct sibling of `/dev/tcp/host/port` in bash, and a resolver that only
  ever sees numeric addresses would leave `/dev/tcp/host/port` half-working
  (numeric IPs only) — not a meaningfully smaller or safer first slice, just
  an incomplete one that would need revisiting immediately.
- **Depend on an external DNS crate instead of hand-rolling the resolver.**
  Rejected on the same zero-dependency constraint that governs the rest of
  this crate — `Cargo.toml` has never taken a dependency, and a resolver
  crate would be the first.
- **A separate error type for resolver failures instead of reusing
  `Errno`.** Rejected: every other module in this crate reports failure as
  `Result<T, Errno>`; introducing a second error type for exactly one module
  would break the "one error currency" property callers currently rely on
  everywhere else.

## Consequences

- New modules: `socket` (raw TCP/UDP primitives) and `dns` (stub resolver),
  tracked as separate implementation issues and shipped incrementally, the
  same pattern Round 3/4 used for every other multi-piece addition.
- New per-arch syscall numbers: `SOCKET`/`BIND`/`CONNECT`/`LISTEN`/
  `ACCEPT`/`ACCEPT4`/`SENDTO`/`RECVFROM`/`SHUTDOWN` (41–50, 288 on x86_64;
  198–212, 242 on aarch64 — verified against kernel headers).
- This is a meaningfully larger surface than anything else in the crate to
  date (two new modules, a wire-format parser, two new address-family
  layouts) — reviewed and tested to the same bar as everything else (kernel-
  layout size/offset assertions, per-arch clippy, no_std builds, real
  network round-trips in tests where feasible), not a special-cased
  exception to the crate's usual rigor.
- `/dev/tcp`/`/dev/udp` redirection itself remains `rush`'s responsibility to
  implement against these primitives — this crate provides the sockets and
  resolver, not shell redirection syntax.
