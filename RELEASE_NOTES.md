# Release Notes

One entry per merged PR against `main`, newest first. No version tags yet, so the
PR is the unit of change. Each entry says what changed and *why*, and states known
limitations plainly rather than leaving them implied.

---

## PR #44 — Apply the standard governance file set
**2026-08-08** · [#44](https://github.com/baileyrd/rusty_agent_gateway/pull/44)

- **Added:** PR and issue templates, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY,
  CHANGELOG, this file, ARCHITECTURE, an ADR seed, and a Rust CI workflow. The repo
  audit went from 1/10 to 10/10 plus CI.
- **Changed:** README was deliberately left alone. The existing one is the repo's
  real documentation; the template would have been a downgrade.
- ARCHITECTURE records that this repo *deviates from the ports-and-adapters
  default* — there are no `trait` ports, every extension point is a closed enum
  matched exhaustively. Said out loud because a reader would otherwise infer it
  from the absence of traits and assume it was an oversight.
- The CI workflow's three commands were run locally before it shipped, so it
  arrives green rather than red on its first PR.
- **Known limitation:** `SECURITY.md` still carries the placeholder contact —
  publishing an address into a repo is outward-facing and was not chosen
  unprompted. CI also gates nothing until it is set as a required status check in
  branch protection.

## PR #43 — Name the inventory's mesh-only fields in the scope list
**2026-08-08** · [#43](https://github.com/baileyrd/rusty_agent_gateway/pull/43)

- **Changed:** README's "parses but is not enforced" list now names `vips`,
  `waypoint`, `locality`, `subjectAltNames` and `loadBalancer` alongside the other
  entries, instead of only explaining them further down.
- Documentation only; no code changed.

## PR #42 — Forward a TLS connection without terminating it
**2026-08-08** · [#42](https://github.com/baileyrd/rusty_agent_gateway/pull/42)

- **Added:** `protocol: TLS` passthrough. A listener carrying `tcpRoutes` forwards
  connections without decrypting them, presenting no certificate.
- **Changed:** `tcpRoutes` — not the protocol — is what marks a listener as
  passthrough. `TLS` covers both of the Gateway API's modes, and a listener naming
  it with a certificate and HTTP routes has been terminating since before
  passthrough existed here; keying off the protocol alone would have broken those.
- The only thing such a route can match on is the SNI name, because a path, method
  or header is inside the encryption. Exact names beat wildcards beat the
  catch-all; a name nothing claims is **closed** rather than sent to whichever
  route sorted first.
- **Known limitation, stated rather than implied:** no route policy applies to
  passthrough traffic — not `jwtAuth`, `extAuthz`, header modifiers or guards.
  Every policy in this gateway works because the bytes are in the clear at some
  point, and none of them are here. `--check` says so out loud, since it is
  invisible in a file that simply has no `policies:` key.
- 832 tests passing (819 before).

## PR #41 — Serve a certificate per hostname
**2026-08-08** · [#41](https://github.com/baileyrd/rusty_agent_gateway/pull/41)

- **Added:** SNI-based certificate selection. Two listeners on one port may hold
  different certificates, chosen by the name the client asked for.
- **Changed:** the name is read by peeking the ClientHello rather than through a
  `rustls` certificate resolver. `rusty_tls` holds its `ServerConfig` privately and
  exposes no `ResolvesServerCert`; reaching around it into `rustls` would have
  given up the one thing importing it buys. The bytes stay in the socket and
  `rustls` still parses them a moment later.
- The parser only ever reads. A ClientHello it cannot parse yields no name, the
  default certificate is served, and the handshake succeeds or fails on its own
  merits — the same outcome as before selection existed. Every length in the
  message is attacker-controlled, so every read is bounds-checked against the
  slice; a test truncates a valid ClientHello at every byte offset and asserts none
  parse or panic.
- **Still a startup error,** because a name cannot choose between them: two
  listeners claiming the same hostname with different certificates, and two
  certificates where neither listener names a hostname.
- 819 tests passing (802 before).

## PR #40 — Resolve a guardrails processor by backend or service name
**2026-08-08** · [#40](https://github.com/baileyrd/rusty_agent_gateway/pull/40)

- **Added:** `mcpGuardrails` processors may name `backend:` or `service:` instead
  of a literal `host:`, resolved through the same registry a `service` backend
  uses. A top-level `backends:` list joins the inventory.
- **Changed:** a processor holds an endpoint *ring* rather than a single address,
  since a `service` resolves to however many instances back it. That moved
  `Endpoints` from the proxy crate into `agentgateway-core` — the same move
  `Retry` made earlier, for the same reason.
- A name that does not resolve is a **startup failure**. Dropping it would leave an
  operator believing a guardrail was running, which is the failure the old lint
  existed to avoid.
- 802 tests passing (793 before).

## PR #39 — Serve `service` and `dynamic` backends
**2026-08-08** · [#39](https://github.com/baileyrd/rusty_agent_gateway/pull/39)

- **Added:** the last two backend kinds. `service` resolves against a written-down
  inventory (`services:` + `workloads:`, joined the way a control plane sends
  them); `dynamic` takes its upstream from the request, making the route a forward
  proxy.
- **Fixed (before it could ship):** weights are split across a service's instances,
  not repeated per instance. Half the traffic to a three-instance service means
  half — repeating would have given it three quarters, a silent traffic-splitting
  bug in the one feature weights exist for.
- **Changed:** `gateway.rs` no longer has a catch-all backend arm at all. Every
  kind the configuration can name is now handled, so a new one fails to compile
  there rather than reaching a client as a 501.
- A `dynamic` route is logged at startup and reported by `--check`, both naming
  what to put in front of it: an open forward proxy lets anyone who can reach the
  listener open a connection anywhere the gateway can.
- **Known limitation:** a service the inventory does not hold, or one nothing
  healthy backs, is a startup failure rather than something that recovers — in a
  file-driven inventory nothing is going to come along and fill it in.
- 793 tests passing (747 before).

## PR #38 — Stream Gemini and carry its tool calls
**2026-08-08** · [#38](https://github.com/baileyrd/rusty_agent_gateway/pull/38)

- **Added:** streaming and tool calling for the Gemini provider, completing it as a
  third first-class provider alongside OpenAI and Anthropic.

## PR #37 — Serve Gemini for a non-streamed answer
**2026-08-08** · [#37](https://github.com/baileyrd/rusty_agent_gateway/pull/37)

- **Added:** the `gemini` provider, which previously parsed and was a startup
  error.
- **Changed:** the model name is validated before it reaches a URL the gateway
  signs — Gemini puts the model in the path rather than the body.

## PR #36 — Call the OpenAI moderation endpoint
**2026-08-08** · [#36](https://github.com/baileyrd/rusty_agent_gateway/pull/36)

- **Added:** `promptGuard`'s `openAIModeration` rule, the last unenforced `ai`
  sub-policy.
- **Changed:** a moderation key never travels to a host it was not issued for. An
  Anthropic route will not lend its key to OpenAI's moderation endpoint; the
  gateway refuses to start rather than doing so quietly.

## PR #35 — Fix the cross-binary test port collision
**2026-08-08** · [#35](https://github.com/baileyrd/rusty_agent_gateway/pull/35)

- **Fixed:** the flaky port allocator. Bands are claimed via `flock` rather than
  derived from a pid, so two test binaries cannot collide however their process ids
  fall.

---

<!--
Entries before PR #35 predate this file. History is in git; this log starts where
the file does rather than being backfilled from commit messages that were not
written as release notes.
-->
