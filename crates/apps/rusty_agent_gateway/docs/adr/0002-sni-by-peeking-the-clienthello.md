# ADR-0002: Read SNI from the ClientHello instead of a certificate resolver

Status: Accepted
Date: 2026-08-08

## Context

Two listeners on one port may need different certificates, chosen by the name
the client asked for. `rustls` supports exactly this through
`ResolvesServerCert`, and that is how it is normally done.

This gateway does not import `rustls` directly. It imports `rusty_tls`, whose
`TlsAcceptor` is built from a single certificate chain and holds its
`ServerConfig` privately — there is no constructor taking a config, and no
resolver to install. So the normal route was closed.

Until this decision, the situation was handled by refusing to start: two
listeners on one port with different certificates was a startup error, because
quietly serving the first one's certificate to the second one's clients is a
misconfiguration nobody notices until a browser complains.

## Decision

Read the server name off the wire before the handshake begins, and use it to
pick among several `TlsAcceptor`s.

The first bytes a client sends are a ClientHello carrying the SNI extension in
plaintext — it has to be, since a server cannot decrypt anything before it knows
which certificate to present. The socket is **peeked** rather than read, so the
bytes stay in the kernel buffer and `rustls` parses the same ClientHello itself a
moment later.

The parser only ever reads. It never decides whether a handshake succeeds. A
message it cannot parse yields no name, the default certificate is served, and
the handshake succeeds or fails on its own merits — which is exactly the
behaviour from before selection existed.

## Alternatives considered

**Build the `rustls::ServerConfig` directly and install a `ResolvesServerCert`.**
The obvious answer, and the one rejected. It gives up the single thing importing
`rusty_tls` buys: that this gateway is not the consumer in the ecosystem that
rolls its own TLS. There is already one documented exception to "import
`rusty_tls`, never `rustls`" — installing the crypto provider, which `rusty_tls`
does not do — and a second one would establish that the rule bends whenever it
is inconvenient.

**Add a constructor to `rusty_tls`.** Correct in the long run, and still open.
It lives in another repository on its own release cadence, so it does not
unblock this.

**Keep refusing at startup.** Honest, but it makes a legitimate deployment
impossible rather than merely unimplemented.

## Consequences

- A hand-written ClientHello parser now exists in `agentgateway-tls`, on the
  path of every accepted connection to a multi-certificate port. Every length in
  that message is written by whoever connected, so every read is bounds-checked
  against the slice rather than against the preceding length. A test truncates a
  valid ClientHello at every byte offset and asserts none parse or panic.
- The peek is time-bounded. `peek` returns whatever is buffered *now*, so a
  ClientHello split across two segments arrives in two looks, and a peer that
  sends half and stops would otherwise be waited on forever.
- The same parser turned out to be what `protocol: TLS` passthrough needed
  (ADR-0003 territory, implemented in the same series): routing a connection
  nobody decrypts can only be done on the name, and the name was already being
  read.
- Two cases remain startup errors, because a name cannot choose between them:
  two listeners claiming the same hostname with different certificates, and two
  certificates where neither listener names a hostname.
- If `rusty_tls` ever exposes a resolver, this can be deleted in favour of it.
  The behaviour it must preserve is the fallback: no name, or an unrecognised
  name, serves the default certificate rather than refusing.
