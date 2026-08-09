# Architecture

## Overview

A configuration-driven agent gateway: one YAML file describes ports, listeners,
routes, policies and backends, and the process compiles that into a data plane at
startup. It proxies HTTP, federates MCP servers, translates between LLM provider
APIs, and terminates or forwards TLS.

**What it is not.** It is not a control-plane client — there is no xDS, no
Kubernetes watch, no dynamic reconfiguration. The file is the whole input, read
once. Where upstream agentgateway learns services and endpoints from a control
plane, this reads the same shapes written down by hand (`services:`,
`workloads:`). That boundary is deliberate and it is why `--check` exists: if
configuration cannot change at runtime, then everything wrong with it should be
knowable before the process serves a request.

## Boundaries

This repo **deviates from the ports-and-adapters default**, and the deviation is
worth stating rather than leaving a reader to infer it from the absence of
traits.

There are no `trait` ports here. Every extension point is a closed enum matched
exhaustively at the seam. That is a real trade: an open trait would let a backend
kind live outside the crate that dispatches it, and in exchange a new variant
would compile fine while silently falling through to a runtime default. With
enums, adding a variant breaks the build at every site that must handle it. For a
gateway whose failure mode is *routing traffic somewhere nobody asked for*, that
is the trade worth making — `gateway.rs` deliberately has no catch-all arm, so an
unhandled backend kind cannot reach a client as a 501.

Recorded in full, with the alternatives, as
[ADR-0003](./docs/adr/0003-closed-enums-at-extension-points.md).

| Seam | Variants / implementations | Notes |
| ---- | -------------------------- | ----- |
| `BackendTarget` (config) → `BackendState` (runtime) | `host`, `service`, `mcp`, `ai`, `dynamic` | The dispatch point for a route's destination. No catch-all arm: a new kind is a compile error, not a 501 |
| `Provider` | `OpenAi`, `Anthropic`, `Gemini` | LLM API translation. `vertex`/`bedrock` parse and are a *startup* error — they sign with cloud credentials, which is different work from translating |
| `Protocol` + `Listener::passes_through()` | `HTTP`, `HTTPS`, `TLS` (terminate), `TLS`+`tcpRoutes` (forward), `TCP`, `HBONE` | `tcpRoutes` decides passthrough, not the protocol: `TLS` covers both Gateway API modes |
| `Registry` | services + workloads + named backends, from the config file | The one resolver. Backends, guardrails processors and TCP routes all resolve names through it |
| `Endpoints` | weighted round-robin ring | Shared by host proxying, guardrails processors and TLS passthrough. Deterministic, not random — see below |
| `TlsTerminator` certificates | one per listener hostname | Selected by peeking the ClientHello's SNI, because `rusty_tls` exposes no `ResolvesServerCert` ([ADR-0002](./docs/adr/0002-sni-by-peeking-the-clienthello.md)) |

**Crate seams** carry the rest of the structure. `agentgateway-config` owns
parsing, typing and linting and depends on nothing else; `agentgateway-core` owns
the primitives several crates share (`Registry`, `Endpoints`, `HostnamePattern`,
retry, CORS, rewrite); the protocol crates (`-proxy`, `-mcp`, `-llm`, `-a2a`,
`-tls`, `-auth`) depend on those two and not on each other; `agentgateway` is the
only crate that knows about all of them, and is where configuration becomes a
running data plane.

## Structure

A modular monolith, and there is no forcing function to split it. One process,
one config file, one deployable. The crate boundaries exist for compile-time
isolation and to keep the dependency graph a DAG — not as a staging area for
future services.

Two rules hold the graph in shape:

- **Nothing depends on `agentgateway`.** It is the composition root.
- **Shared code moves down, never sideways.** When two protocol crates needed the
  same thing, it moved into `-core` (this is how `Retry` and `Endpoints` got
  there) rather than one crate importing the other.

## Data flow

```
TCP accept (serve.rs, one task per port)
  │
  ├─ passthrough port?  ── peek SNI ── match tcpRoutes ── splice bytes ── done
  │                                    (nothing is decrypted)
  ├─ TLS port?          ── peek SNI ── pick certificate ── handshake
  │
  └─ hyper service ── gateway.handle(port, peer, scheme, request)
                        │
                        ├─ router: listener by hostname, route by match
                        ├─ policies: jwtAuth → extAuthz → rate limit → rewrite
                        └─ BackendState:
                             mcp     → federated session, guardrails per method
                             host    → weighted ring, HTTP proxy
                             ai      → provider translation, prompt guards
                             a2a     → agent-to-agent framing
                             dynamic → destination from the request itself
```

Two things about this path are worth knowing because they are easy to assume
wrong:

- **Backend selection is deterministic round-robin over a weighted ring**, not
  random choice. Randomness only reaches the configured ratio in expectation, so
  a low-traffic route can sit lopsided for a long time.
- **A `service` backend's weight is split across its instances, not repeated.**
  Half the traffic to a three-instance service means half. Repeating the weight
  per instance would give it three quarters — a silent traffic-splitting bug in
  the one feature weights exist for.

## Key decisions

See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs — [ADR-0002](./docs/adr/0002-sni-by-peeking-the-clienthello.md) on
reading SNI off the wire, [ADR-0003](./docs/adr/0003-closed-enums-at-extension-points.md)
on enums rather than traits.

Three more shape everything else, recorded here because they predate the ADR log
and are cross-cutting rather than single decisions:

1. **Parse everything upstream accepts; enforce what this build can.** A key that
   parses and does nothing is reported by `--check` rather than silently ignored
   — a policy that looks like security and isn't is worse than one that fails to
   load.
2. **Refuse at startup rather than at request time** wherever the configuration
   can be known bad. An unresolvable service, an uncompilable CEL expression, a
   provider that will never answer.
3. **Import `rusty_tls`, never `rustls`.** One documented exception (installing
   the crypto provider, which `rusty_tls` does not do). SNI selection was built
   by reading the ClientHello off the wire specifically to avoid a second.

## Development

The toolchain is pinned in `rust-toolchain.toml` so a local check and CI check
the same thing; CONTRIBUTING has the commands. CI runs fmt, clippy under
`-D warnings`, and the full suite on every PR — it reports but does not yet gate,
since it is not a required status check.

## Non-goals

- **Control-plane integration.** No xDS client, no Kubernetes informers. See
  Overview.
- **`vertex` and `bedrock` providers.** Cloud-credential signing is a different
  kind of work from API translation. They parse and are a startup error, and an
  OpenAI-compatible endpoint in front of them works today.
- **The deprecated MCP HTTP+SSE transport (`sse:`).** `rmcp` 3.1 has no client
  for it.
- **The mesh-only halves of the service inventory** — `vips`, `waypoint`,
  `locality`, `subjectAltNames`, `loadBalancer`. They describe a mesh this
  gateway is not part of. They parse so an upstream file loads, and `--check`
  names them.
