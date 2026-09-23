# ADR-0123: TLS in the Refresh CLI, the Loop as a Function, and the Review's Test Debts

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "All of them" — every open item after the
  growth-line review's fix rounds). No wire change.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `ADR-0118` (the refresh CLI; "TLS not built, on
  evidence"), `ADR-0122` (`RGL-FR-004`: the three tests not added and
  the `free_port()` race named), `ADR-0119` (the `dm-log-writes`
  replay still owed), `ADR-0117` (`Null` on the wire; no nullable
  column yet), `docs/reports/2026-09-23-growth-line-review.md` item 26.
- Supersedes/Superseded by: none. Additive: `Target::with_tls`,
  `refresh_loop`, `REPLICA_REFRESH_TLS_SERVER_NAME`; the listening
  banner's address is the bound one.

## Context

The owner's "All of them" covered six things: TLS in `replica_refresh`
(`ADR-0118` left it out), the four test debts `ADR-0122` named (a
`waiting_writers > 0` assertion, a `.failed-` rename test, an
`--every` test, the `free_port()` race), a nullable field capability
with SQL `NULL`, and the `dm-log-writes` replay. The first five are
buildable here; the last two are not the same kind of item, so they
are answered below rather than built.

## Decision

- `RGT-FR-001` — TLS in the CLI. `Target::with_tls(addr, token,
  server_name)` sets `ConnectOptions::tls` to a `ClientTlsConfig` with
  `TrustPolicy::System`, so the server's certificate is verified
  against the operating system's anchors; a private CA is named by
  `SSL_CERT_FILE`, the same knob the client library already honours.
  The CLI reads `REPLICA_REFRESH_TLS_SERVER_NAME`; unset, the transport
  is plaintext as before, said in its usage. No certificate path enters
  the command line; no trust-anything mode is exposed by the CLI.
- `RGT-FR-002` — the two assertions. `journal.rs`'s
  `stats_count_the_followers_parked_for_a_held_leader` holds a leader
  in its hook, parks two followers and reads `waiting_writers == 2`
  through the public stats, then 0 after release.
  `tests/replica_refresh.rs`'s
  `a_failed_verification_is_kept_aside_and_never_listed` refreshes the
  wrong domain against a real server: the directory lands under
  `.failed-`, `snapshots` never lists it, `prune` never removes it.
- `RGT-FR-003` — the loop as a function. `refresh_loop(target, root,
  domain, keep, every, report)` is the CLI's `--every` body moved into
  the library: refresh, prune, report, sleep, until `report` returns
  `false`. `main` is one call to it. The test runs three rounds with
  `keep = 1` and stops from the callback.
- `RGT-FR-004` — no port race. `memory_server` prints
  `listener.local_addr()` in its listening banner (and the metrics
  listener's in its own), so `SERVER_ADDR=127.0.0.1:0` is usable. The
  four binary tests share one `spawn_listening(Command) -> Server`
  helper that reads stderr until the banner and parses the address the
  kernel chose; `free_port()` is gone. The `Server` guard kills the
  child on drop and keeps the pre-banner text for the tests that assert
  on it.
- Not built, on evidence — a nullable field capability and SQL `NULL`.
  `ADR-0117` put `Null` on the wire and set the trigger: "when the first
  nullable column arrives". No shipped column is nullable; the two
  domains with absent values keep their documented sentinels
  (`ADR-0056`). A capability with no producer is speculative
  generality; the wire, both clients and the `Malformed` rule are
  ready the day a column needs it.
- Not run here, still owed — the `dm-log-writes` replay. The guest
  kernel (`6.18.44-fc`) has no device-mapper and loads no modules; the
  runbook (`ADR-0119`, fixed by `ADR-0120`) is what a machine with it
  runs. The crash-prefix trial in `RESULTS.md` stands as the evidence
  this environment can give.

## Consequences

- Positive: a standby can be refreshed across an untrusted network;
  the two review assertions that were only ever `0` and "exists" now
  pin the behaviour; the loop is testable without a process; the
  crate's server tests cannot collide on a port again.
- Negative / tradeoffs: `TrustPolicy::System` only — a pinned CA
  bundle is `SSL_CERT_FILE`'s job, not a flag; the CLI never
  disables verification. `spawn_listening` reads the banner line by
  line, so a server that fails before binding surfaces as the test's
  panic with its stderr, not a timeout.
- Named, not hidden: the two items above that this round does not
  build.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.101.0 / `FR-114`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 998 tests across 46 targets, 0 failed; Python 7 tests OK. Builder: Claude.
