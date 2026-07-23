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

## PR #17 — `HeaderMap::get_all()`
**2026-07-23** · [#17](https://github.com/baileyrd/rusty_http/pull/17)

- **Added:** `HeaderMap::get_all(&self, name: &str) -> impl Iterator<Item =
  &str>` — every value for a repeated header (case-insensitive, insertion
  order). `HeaderMap` was already a multi-map internally (`append()` keeps
  repeats), but `get()` only ever returned the first match; there was no way
  to read every `Set-Cookie` value off a `HeaderMap` directly before this.
- **Context:** part of a parity-loop run assessing this crate against the
  [`http`](https://docs.rs/http/1.4.2) crate (pinned v1.4.2) — see issue
  #13. Purely additive; no existing signature changed.

## PR #16 — `Method::is_safe()` / `Method::is_idempotent()`
**2026-07-23** · [#16](https://github.com/baileyrd/rusty_http/pull/16)

- **Added:** `Method::is_safe()` and `Method::is_idempotent()`, per RFC 7231
  §4.2. `Method::Extension(_)` (an unrecognized method token) is `false` for
  both.
- **Context:** part of a parity-loop run assessing this crate against the
  [`http`](https://docs.rs/http/1.4.2) crate (pinned v1.4.2) — see issue
  #12. Purely additive; no existing signature changed.

## PR #15 — Named `StatusCode` constants and `canonical_reason()`
**2026-07-23** · [#15](https://github.com/baileyrd/rusty_http/pull/15)

- **Added:** 61 named `StatusCode` constants (`StatusCode::OK`,
  `StatusCode::NOT_FOUND`, ...), the codes registered in the IANA HTTP
  Status Code Registry, plus `StatusCode::canonical_reason()` returning the
  standard reason phrase for a known code.
- **Context:** first PR from a parity-loop run assessing this crate against
  the [`http`](https://docs.rs/http/1.4.2) crate (pinned v1.4.2) — see
  issue #11. Purely additive; no existing signature changed.

## PR #8 — Cross-repo doc accuracy: `rusty_tail`'s migration is complete
**2026-07-22** · [#8](https://github.com/baileyrd/rusty_http/pull/8)

- **Fixed:** `README.md` and `ARCHITECTURE.md` still described
  `rusty_tail`'s migration as pending after both of its PRs
  (`ts-control`/`ts-derp`, then `ts-cli`/`ts-localapi`) had already
  merged in its own repo. Updated the Status section, the Overview, and
  the two "gap found" sections' closing lines to say so.
- **Added:** explicit cross-references between this crate's two
  consumers — `rusty_request`'s README now links `rusty_tail` and vice
  versa — plus a note that `ts-cli`/`ts-localapi` were `hyper`-based
  rather than hand-rolled, which the original "six hand-rolled
  implementations" framing didn't capture.
- **Known limitation:** docs-only change; no code touched.

## PR #7 — Byte-exact protocol handoff: `into_parts()` + `Replay<T>`
**2026-07-22** · [#7](https://github.com/baileyrd/rusty_http/pull/7)

- **Context:** preparing `rusty_tail`'s migration (per the user's
  instruction to start with its two genuinely-hand-rolled sites,
  `ts-control/controlhttp.rs` and `ts-derp/client.rs`) surfaced a real
  correctness gap in all three transports before any `rusty_tail` code
  was touched.
- **The gap:** `SyncTransport`/`AsyncTransport::into_inner()` hands back
  only the raw transport, silently discarding any bytes already read
  into the transport's internal buffer but not yet consumed by a
  `read_*`/`into_body_reader` call. That's fine when nothing can arrive
  before the caller starts reading (`rusty_request`'s CONNECT-tunnel
  handoff, already merged, is provably sequential — the proxy responds
  before our own client sends anything for the tunneled protocol). It is
  not fine for `ts-derp`'s DERP upgrade: the server can push its
  ServerKey greeting frame in the same TCP read as the HTTP upgrade
  response, and that frame would land in the discarded buffer,
  corrupting the DERP stream from its very first frame.
- **Added:** `into_parts(self) -> (T, Vec<u8>)` on all three transports
  (`sync::SyncTransport`, `async_tokio::AsyncTransport`,
  `tokio_native::AsyncTransport`), returning the transport plus any
  unconsumed buffered bytes, so a caller can reclaim exactly what the
  peer already sent before handing the connection to a different
  protocol. `into_inner()` is unchanged (still transport-only) but its
  docs now point callers doing a protocol handoff at `into_parts()`
  instead.
- **Added:** `tokio_native::Replay<T>`, a small `AsyncRead`(+`AsyncWrite`
  when `T` supports it) wrapper that replays a reclaimed prefix before
  falling through to the wrapped transport — needed because
  `tokio::net::TcpStream::into_split()` (what `ts-derp/client.rs` does
  right after its upgrade handshake) produces two owned halves that
  can't otherwise be primed with leftover bytes. 5 new unit tests plus a
  doc test walking through the full `into_parts` → `Replay` handoff; 112
  unit tests total (up from 107), 2 doc tests, `cargo fmt`/`clippy -D
  warnings` clean with `--all-features`.
- **Known limitation, stated plainly:** `Replay<T>` lives only in
  `tokio_native` for now, sized to what `ts-derp`'s migration actually
  needs — `sync`/`async_tokio` gained `into_parts()` for symmetry and
  because the same discard bug applied there too, but neither has a
  `Replay` type yet since no known caller needs one today.

## PR #6 — A third adapter, `tokio_native`, for real crates.io tokio
**2026-07-22** · [#6](https://github.com/baileyrd/rusty_http/pull/6)

- **Context:** `rusty_request` migrated onto this crate in its own repo
  (deleting its `http1`/`url`/`cookie`/`header`/`method`/`status`) --
  no change here, noted for the record since this repo's own
  `RELEASE_NOTES.md` only tracks PRs against this repo.
- **Added:** `tokio_native::AsyncTransport` (behind a new `tokio`
  feature) -- the same shape as `async_tokio::AsyncTransport`, but
  driving the sans-IO core over real crates.io `tokio`'s
  `AsyncRead`/`AsyncWrite` instead of `rusty_tokio`'s. Includes the same
  eager/incremental (`BodyReader`) body reading both other adapters
  have. 12 new tests; 107 total, `cargo fmt`/`clippy -D warnings` clean
  with `--all-features` and with just `--features tokio`.
- **Why:** resolves the gap `ARCHITECTURE.md` recorded while building
  the first two adapters -- `rusty_tail`'s four donor sites are async
  over real tokio, which fit neither `sync::SyncTransport` nor
  `async_tokio::AsyncTransport`. Considered and declined: a
  `spawn_blocking` bridge onto the sync adapter (would need a full
  actor-task-plus-channel design to own a blocking transport for a
  connection's whole lifetime, plus real per-call overhead) and a
  `rusty_tail` runtime migration onto `rusty_tokio` (a far larger,
  unrelated undertaking). See `ARCHITECTURE.md` for the full writeup.
- **Dependency added:** `tokio` (crates.io, `default-features = false`,
  `io-util` only for the shipped adapter; `rt`/`macros` added as a
  dev-dependency for tests only) -- optional, behind the `tokio`
  feature, so a consumer on a different runtime pulls in nothing extra.
- **Known limitation, stated plainly:** like the other two adapters,
  only exercised against in-memory duplexes in tests so far --
  `rusty_tail`'s own migration (not done in this repo) is what proves
  it against a real socket.

## PR #5 — Incremental body reading (`BodyReader`), ahead of the `rusty_request` migration
**2026-07-22** · [#5](https://github.com/baileyrd/rusty_http/pull/5)

- **Added:** `SyncTransport::into_body_reader`/`AsyncTransport::into_body_reader`,
  returning a `BodyReader<T>` that pulls a response body one chunk at a
  time via `next_chunk()` instead of buffering it all upfront
  (`read_body`'s existing eager behavior is unchanged and still
  available). 10 new tests (5 sync, 5 async); 96 total, `cargo
  fmt`/`clippy -D warnings` clean with and without `--all-features`.
- **Also:** `cookie::parse_http_date` is now `pub` (was private to the
  module) -- RFC 7231 IMF-fixdate parsing is a general HTTP concern
  (`Retry-After`, `Last-Modified`, not just `Set-Cookie`'s `Expires`),
  and `rusty_request`'s `retry.rs` already reuses this exact parser for
  `Retry-After` today, so the migration needs it reachable.
- **Why this wasn't in step 3's original scope:** donor 1
  (`rusty_request`'s `http1.rs`) had both an eager and a streaming
  response-reading path, and `rusty_request`'s own
  `send_streaming`/`StreamingResponse` depends on the streaming half.
  Discovered while planning the migration PR (step 4): without this,
  that migration could only replace the eager path, leaving a second
  parser behind for the streaming case -- the opposite of the mission.
- **Known limitation, stated plainly:** like the rest of this crate,
  `BodyReader` is only exercised against in-memory loopbacks in tests,
  not a real socket yet -- that's still `rusty_request`'s migration PR to
  prove out.

## PR #4 — `cookies` feature (RFC 6265 jar, completing the crate's own scope)
**2026-07-22** · [#4](https://github.com/baileyrd/rusty_http/pull/4)

- **Added:** `cookie::CookieJar` behind the new `cookies` feature — ported
  near-verbatim from `rusty_request`'s `cookie.rs` (donor 3), `pub` here
  rather than `pub(crate)` since this crate has external consumers. 16
  new tests; 86 total, `cargo fmt`/`clippy -D warnings` clean with and
  without `--all-features`.
- **Why now, ahead of a migration PR:** the handoff's step 4 groups
  `rusty_request`'s `http1`/`url`/`cookie` deletion into one migration
  unit. Without this feature, that migration could only delete two of
  the three donor files, leaving `cookie.rs` duplicated — the opposite of
  the mission. This closes that gap before any cross-repo change starts.
- **Known limitation, carried over from the donor, not introduced by this
  port:** no public-suffix-list support — the one safety check is RFC
  6265 §5.3's own narrower domain-suffix rule, not full "supercookie"
  defense. Documented in `cookie.rs`'s module docs, same as the donor.

## PR #3 — Sync + async transport adapters (mission handoff step 3)
**2026-07-22** · [#3](https://github.com/baileyrd/rusty_http/pull/3)

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
