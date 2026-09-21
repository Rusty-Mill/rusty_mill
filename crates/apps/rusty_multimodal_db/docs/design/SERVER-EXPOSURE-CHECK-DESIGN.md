# Server Exposure Check: Refuse an Unprotected Non-Loopback Listener (Proposed and implemented)

- Status: **Proposed and implemented on one branch; the owner asked
  for it** (2026-09-21, `ADR-0094`). The third "do now" item of the
  release-readiness review.
- Date: 2026-09-21
- Related: `ADR-0012`/`docs/design/SERVER-AUTH-DESIGN.md` (the
  "nothing configured is `ReadWrite`" rule this leaves in the library),
  `ADR-0014`/`ADR-0023` (TLS), `ADR-0028` (certificate classes count
  as configured), `ADR-0032` (`ServeOptions`), `ADR-0092`/`ADR-0093`
  (the two rounds before), `README.md`'s security paragraph.
- Supersedes/Superseded by: none. Additive: `src/server/exposure.rs`,
  one check per binary, `SERVER_ALLOW_INSECURE`.

## Purpose and scope

Make the binaries enforce the rule the README states: a server
reachable from the network must have authentication and TLS. Leave the
library as it is — `ServeOptions::default()` reproducing the original
open server is what a thousand loopback tests rely on.

Scope, exactly: loopback classification (`EXP-FR-001`); the rule
(`EXP-FR-002`); the binaries and the override (`EXP-FR-003`); proven
against the binary (`EXP-FR-004`); everything else unchanged
(`EXP-FR-005`).

## Non-goals

- **Refusing inside `serve`/`serve_tables`.** Option (b); the library
  keeps its contract.
- **Resolving hostnames.** `localhost:7881` is treated as exposed;
  bind `127.0.0.1:7881`.
- **Checking the metrics HTTP listener.** `ADR-0069`'s accepted
  tradeoff (no auth, no TLS on `/metrics`) stands; a separate round if
  wanted.
- **Warning on a loopback bind.** Nothing to warn about.

## Context and terminology

Read from `main` after PR #293 this pass:

- **`bind_is_loopback(addr)`**: `addr.parse::<SocketAddr>()` and
  `ip().is_loopback()`; an unparsable string is `false`.
- **`check_exposure(addr, &options)`**: loopback → `Ok`;
  `!options.is_configured()` → `Err(NoAuth)`; `options.tls().is_none()`
  → `Err(NoTls)`; else `Ok`. `Exposure: Display` names the variables to
  set.
- **`allow_insecure_from_env()`**: `SERVER_ALLOW_INSECURE` trimmed
  equals `1`.
- **The binaries**: `dog_server`, `reminder_server`, `entity_server`,
  `memory_server` — after the `TlsConfig` fold, before any further
  option: refuse with `panic!("refusing to listen on {addr}: …")`, or
  with the override `eprintln!("WARNING: …")` and continue.

## Requirements

- `EXP-FR-001` **Loopback.** As "Context"; `127.x`, `[::1]` are;
  `0.0.0.0`, `[::]`, a LAN address, a hostname are not.
- `EXP-FR-002` **The rule.** As "Context"; auth before TLS, so the
  first missing half is the one named.
- `EXP-FR-003` **The binaries and the override.** Each of the four;
  the refusal names the address, the reason, and
  `SERVER_ALLOW_INSECURE=1`; the override warns with the same reason.
- `EXP-FR-004` **Proven against the binary.** `memory_server` on
  `0.0.0.0`: nothing configured refuses naming
  `SERVER_AUTH_READ_WRITE_TOKEN`; a token alone refuses naming
  `SERVER_TLS_CERT_CHAIN_PATH`; the override starts and serves. The
  loopback case is every other subprocess test.
- `EXP-FR-005` **Everything else unchanged.** `serve`, `serve_tables`,
  `ServeOptions`, the wire, the clients.

## Considered options

- **(a) A pure check applied by each binary — implemented.**
- **(b) Refuse inside `serve`.** Every caller and test that binds
  `0.0.0.0` or a LAN address without auth breaks; the library's
  "default reproduces the original" contract ends.
- **(c) Warn only.** The banner already did; the review's finding
  was that a warning is not a refusal.
- **(d) Decline.**

The owner's shorthand: **(a)** as implemented; **(b)** into the
library; **(d)** decline and revert.

## Proposed shape

`src/server/exposure.rs` (new); `src/server/mod.rs`; the four
`src/bin/*_server.rs`; `tests/memory_server_exposure.rs` (new);
`Cargo.toml` (the `[[test]]`).

## Data/state and invariants

- `check_exposure` is pure over `(addr, is_configured, tls.is_some())`.
- A binary that reaches `serve` on a non-loopback address either has
  auth and TLS or `SERVER_ALLOW_INSECURE=1` was set.

## Errors, failure, recovery, and observability

A refusal is a startup panic (the binaries' convention for
misconfiguration) with the reason and the fix on stderr; the override
leaves a `WARNING:` line in the same place.

## Security, privacy, and compatibility

A loopback deployment is unchanged. An exposed deployment that relied
on nothing being configured now fails to start until it is configured
or explicitly allowed — the intended incompatibility.

## Acceptance criteria

1. Unit: `EXP-FR-001`/`002`/the messages (three tests in
   `exposure.rs`).
2. Integration: `EXP-FR-004` against the binary.
3. Not measured: a startup-only check.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`.
Independent review owed.

## Traceability

- Roadmap: `SERVER-EXPOSURE-CHECK`.
- Decision: `ADR-0094`.
- Specification: `SERVER-001` v0.79.0 / `FR-091`.
- Requirements: `EXP-FR-001`–`005`.

## Open questions

- **Into the library** — option (b).
- **The metrics listener** — the same rule for
  `SERVER_METRICS_HTTP_ADDR`, if `ADR-0069`'s tradeoff is revisited.

## Change history

- 2026-09-21: proposed and implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` on the owner's word ("3"),
  the third "do now" item of the release-readiness review.
- 2026-09-21: implemented as `SERVER-001` v0.79.0 / `FR-091`. Acceptance
  criteria 1–2 are the tests: `exposure.rs` +3,
  `tests/memory_server_exposure.rs` +3. `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — lib 654 (up from 651), `memory_server_exposure` 3 (new), 954 tests across 42 targets, 0 failed. Still no independent
  review — owed.
