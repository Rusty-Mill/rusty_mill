# Release Notes

<!--
Two variants, pick the one that fits this repo's actual unit of change:

1. No version tags yet (pre-1.0, nothing published) — track by PR instead, same way
   AISF does it: one entry per merged PR against main, reverse chronological, each
   linking to its PR and (where one exists) to the doc that covers the change in full
   detail. Use "## PR #N — <summary>" headers.

2. Actual version tags exist — use "## vX.Y.Z - YYYY-MM-DD" headers instead, each
   linking to the PRs it shipped and a compare link to the previous tag. Add an
   "### Upgrade notes" subsection under any entry with a breaking change.

Either way, keep the tone AISF's file uses: bolded category tags inline in the
bullet (**Added:** / **Changed:** / **Fixed:**), not separate subheaders per
category — and state known limitations or deliberate scope cuts plainly instead of
leaving them implied.
-->

Tracks notable changes to this repo, one entry per merged PR against `main`,
newest first (no version tags yet — this is pre-1.0).

---

## PR TBD — Sync + async transport adapters (mission handoff step 3)
**2026-07-22** · (not yet pushed — link once merged)

- **Added:** `sync::SyncTransport<T: std::io::Read + Write>` and
  `async_tokio::AsyncTransport<T: rusty_tokio::io::AsyncRead + AsyncWrite>`
  (behind the new `rusty-tokio` feature) — both drive
  `head::parse_request_head`/`parse_response_head` and `body`'s framing
  over a real (or in-memory-loopback, in tests) transport: read a head,
  read its body per `Framing`, write a head, write a body (verbatim or
  chunked). A new `transport::Error`/`Result` (re-exported as
  `TransportError`/`TransportResult`) wraps I/O failures alongside the
  core's own `Error`, since the sans-IO core deliberately has no `Io`
  variant of its own.
- **Added:** `rusty_tokio` as an optional git dependency, pinned to the
  same rev `rusty_tls`/`rusty_request` already pin
  (`ac598c930e85460ae3f79328a1d82f28390672f8`), gated behind the
  `rusty-tokio` feature so a sync-only consumer pulls in nothing extra.
- **Added:** 13 new tests (7 sync, 6 async) exercising both adapters
  end-to-end — request/response heads plus all three body framings
  (`Content-Length`, chunked, close-delimited), head+body writing, and a
  repeat of the exact-head-consumption guarantee through the buffering
  adapter itself, not just the bare parser. 70 unit tests total, `cargo
  fmt`/`clippy -D warnings` clean with and without `--all-features`.
- **Found while building this, not in the original handoff:** step 3's
  sequencing assigned `rusty_tail`'s four donor sites to the sync
  adapter. Source review shows this is wrong — all four are already
  async, built on real crates.io `tokio`, not `std::io` and not
  `rusty_tokio` either. **Neither adapter built here fits those call
  sites as they exist today.** Recorded in `ARCHITECTURE.md` in detail;
  migrating `rusty_tail` (step 4) needs a deliberate call on a third
  adapter, a `spawn_blocking` bridge, or a runtime migration — not
  something this crate's mission decides on its own.
- **Known limitation, stated plainly:** both adapters are only exercised
  against in-memory loopbacks in tests, not a real socket or a real peer.
  The `cookies` feature is still not built; out of this step's scope.

## PR #2 — Url + sans-IO message core (mission handoff step 2)
**2026-07-21** · [#2](https://github.com/baileyrd/rusty_http/pull/2)

- **Added:** `Url` (ported near-verbatim from donor 2, `rusty_request`'s
  `url.rs`); `HeaderMap`, `Method`, `StatusCode`, `Version`; the sans-IO
  message core (`head::parse_request_head`/`parse_response_head` +
  `RequestHead::write`/`ResponseHead::write`, inverted from donor 1's
  async `http1.rs`); body framing (`body::request_framing`/
  `response_framing`) and the incremental `body::ChunkedDecoder`. 57 unit
  tests plus a doc test, all passing; `cargo fmt`/`clippy -D warnings`
  clean.
- **Added, pinning the mission's core requirement:** tests
  (`head::tests::request_head_over_reads_nothing_past_the_blank_line` and
  its response counterpart) that parse a head with trailing non-HTTP bytes
  appended — modeled on donor 4's actual Noise-upgrade scenario
  (`ts-control/controlhttp.rs`) — and assert the trailing bytes are
  returned untouched via `Outcome::Complete::consumed`.
- **Added, beyond the handoff's explicit step-2 scope:** an explicit
  `max_head_len`/`max_line_len` bound on head parsing and chunked-framing
  lines (`HeadTooLarge`/`ChunkFramingTooLarge` errors), defaulting to 8
  KiB. Not asked for in the sequencing, but flagged in this crate's own
  earlier review as a real gap: this core parses untrusted, server-bound
  requests (donor 6's LocalAPI server), and an unbounded head/line is a
  memory-exhaustion vector against exactly that consumer.
- **Changed:** `Method` gained `Connect`/`Trace` variants and an
  `Extension(String)` catch-all, and a `parse` constructor -- donor 1's
  client-only `Method` never needed to parse an arbitrary incoming
  token; a bidirectional core does.
- **Known limitation, stated plainly:** no sync/async transport adapter
  exists yet, so nothing here has been exercised against a real socket —
  only against byte buffers in tests. The `cookies` feature (RFC 6265,
  donor 3) is also not yet built; it wasn't part of this step's scope.

## PR #1 — Skeleton crate + seam rule (mission handoff step 1)
**2026-07-21** · [#1](https://github.com/baileyrd/rusty_http/pull/1)

- **Added:** `Cargo.toml` (zero runtime deps, `[lints.rust]` forbidding
  `unsafe_code`), `src/lib.rs` crate-level docs stating the mission/scope/
  seam rule, `.gitignore`, and a Rust CI workflow (`cargo fmt`/`clippy -D
  warnings`/`cargo test`) now that a manifest exists to run it against.
- **Changed:** README and ARCHITECTURE rewritten from the greenfield
  placeholder to the real mission (sans-IO HTTP/1.1 core + `Url`,
  replacing donor code in `rusty_request`/`rusty_tail`), with a boundary
  table naming the planned (not yet built) sans-IO core and sync/async
  transport adapters.
- **Known limitation, stated plainly:** this is the skeleton only — no
  parsing/serialization code has landed. `Url` and the sans-IO message
  core are next (step 2 of the handoff plan), followed by adapters and
  the migration PRs.
- **Correction to the source handoff, worth recording:** the handoff
  described all four `rusty_tail` donor sites as hand-rolled HTTP. Source
  review found `ts-cli/src/localapi.rs` and `ts-localapi/src/lib.rs`
  actually build on `hyper`'s HTTP/1 client/server connection machinery —
  only their request routing and query-param parsing are hand-rolled, not
  the HTTP framing itself. `ts-control/src/controlhttp.rs` and
  `ts-derp/src/client.rs` are genuinely hand-rolled as described. This
  doesn't change the mission, but it changes what migrating the LocalAPI
  sites means: trading a mature, widely-used dependency (`hyper`) for this
  new crate, not de-duplicating hand-rolled logic — worth weighing
  deliberately when step 4's migration order gets there, not assumed.

## PR #1 — Bootstrap repo governance scaffolding
**2026-07-21** · [#1](https://github.com/baileyrd/rusty_http/pull/1)

- **Added:** PR templates (feature/bug_fix/docs/chore), issue templates
  (bug_report/feature_request), CONTRIBUTING, CODE_OF_CONDUCT, SECURITY,
  CHANGELOG, RELEASE_NOTES (this file), ARCHITECTURE, and an ADR seed via the
  repo-config skill.
- **Changed:** expanded README with a one-line project description and a
  Status section noting this repo has no code yet.
- **Known limitation, stated plainly:** this repo is greenfield — no
  `Cargo.toml` exists yet, so CI workflows were intentionally skipped (an
  always-red workflow is worse than none), and the ARCHITECTURE boundary
  table is left empty since there's nothing real to document yet.
