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

No version tags yet (pre-1.0, nothing published). Tracked by unit of change,
reverse chronological, each linking to its PR once one exists.

---

## Retention module: segment rolling, size/time-based deletion, `Clock` seam
**2026-08-02**

- **Added:** `src/retention.rs`'s `Log` — owns a sequence of `Segment`s in
  one directory, rolls to a new one once the active segment would cross
  `RetentionPolicy::max_segment_bytes`, and deletes closed segments via
  `Log::enforce_retention` by size (`max_total_bytes`) or age
  (`max_segment_age_millis`) — oldest first, active segment never touched.
  A retiring segment is synced as part of the roll itself, not left to the
  caller, so a crash right after a roll can't lose records this process
  already considered safely closed.
- **Added:** `src/clock.rs`'s `Clock` trait, `SystemClock` (real),
  `SimClock` (deterministic, manually advanced) — the same
  real/simulated pairing `rusty_tokio::io::OpDriver`/`SimDriver` already
  established, applied to time so retention age checks are provable without
  a test actually sleeping. Unlike `OpDriver`, this is `rusty_stream`'s own
  trait — `rusty_tokio` has no clock abstraction to build on.
- **Added:** `Segment::byte_len()` — what `Log` uses to decide when to roll.
- 7 new tests (21 total), including: crossing the size threshold actually
  rolls; size-based retention deletes the right segment and only that one;
  time-based retention leaves a segment alone until the simulated clock
  actually crosses the age window, then deletes it; and a full
  create-append-roll-crash-reopen cycle recovers both the closed and active
  segment correctly.
- **Known limitation, stated plainly:** `Log::open` recovers from an
  explicit list of segment base offsets, not directory scanning —
  `rusty_tokio::io::OpDriver` has no directory-listing operation at all
  (`SimDriver` only knows paths it's been told about), so this is a real
  constraint, not a shortcut. In practice this means a manifest of which
  segments exist needs to be persisted somewhere before a real
  restart-and-recover path works — not built in this pass. Recovered closed
  segments also don't have a real creation timestamp yet (not persisted
  anywhere on disk), so time-based retention is only accurate within a
  single process's uptime, not across a restart, until segment creation
  time gets added to the on-disk header alongside epoch/base_offset.

## Cargo project scaffolded: first real `Segment` storage code lands
**2026-08-02**

- **Added:** `Cargo.toml` depending on `rusty_tokio` (pinned `git` `rev`,
  `thread-per-core` + `io-uring-fs` features, per ADR-0002 D3) — the first
  real code in this repo.
- **Added:** `src/record.rs` — on-disk record framing (`[len][crc32][payload]`),
  hand-rolled CRC-32/ISO-HDLC (no new dependency — this project treats every
  dependency as audit surface, per ADR-0002's whole D3 thread).
- **Added:** `src/offset.rs` — `Offset`/`DurableOffset`/`CommittedOffset`/
  `Epoch`, the ADR-0002 D2 primitives a future consensus layer needs without
  a storage-format migration.
- **Added:** `src/segment.rs` — a real, working append-only `Segment`: create,
  append, read, sync, and crash recovery (truncates a torn tail rather than
  serving it). Built directly on `rusty_tokio`'s `OpDriver`/`UringFile`, not a
  hand-rolled parallel trait, per ADR-0002 D4. 14 tests pass, including
  working versions of all three of D4's minimal DST scenarios (crash/recovery
  cycles, torn write, lying fsync) against `SimDriver`.
- **Added:** `src/retention.rs`, `src/consumer.rs` — module stubs (docs only,
  no implementation) for segment rolling/deletion and per-consumer offset
  tracking, scoped but deliberately not designed yet.
- **Fixed (blocking, upstream):** `rusty_tokio` could not be consumed as a
  Cargo `git` dependency by any external project — its `Cargo.toml` used
  `path = "../rusty_std"`-style sibling-repo dependencies, which only
  resolve inside its own multi-repo dev checkout. Filed and verified fixed
  upstream (`baileyrd/rusty_tokio#254`, closed via `rusty_std`/`rusty_libc`/
  `rusty_win32` converting to pinned `git` dependencies, matching the
  existing `rustils` pattern) before this scaffold could build at all.
  Re-verified the actual fix — not just that the issue closed — with a
  build against a completely fresh `CARGO_HOME`.
- **Known limitation:** the on-disk index is dense and in-memory only, not
  the sparse on-disk index `docs/phase1-scope.md` §2 describes — real,
  useful, but a narrower slice than the full Phase 1 scope. No fsync policy
  configuration yet (`Segment::sync` exists; when to call it is left to the
  caller). No wire protocol integration (ADR-0002 D1) yet.

## ADR-0002 D3 reopened and reversed: `rusty_tokio` replaces compio as the Phase 1 runtime
**2026-08-01**

- **Changed:** the Phase 1 runtime decision flips from compio (pinned 0.18.0)
  to `rusty_tokio` (RustyMill's own runtime), with its `thread-per-core` and
  `io-uring-fs` features. `rusty_tokio` was previously out of the D3
  comparison entirely — it lacked thread-per-core scheduling and had no
  io_uring file I/O (`fs::File` was 100% `spawn_blocking`). Those two gaps
  were written up as a concrete handoff (`baileyrd/rusty_tokio#252`) and
  filed as an issue; `baileyrd/rusty_tokio#253` closed it with a real
  implementation.
- **Verified, not taken on trust:** builds clean on stable Rust; straced the
  new example and confirmed real `io_uring_enter` calls for file ops (not
  `spawn_blocking`); confirmed genuine per-core reactor instantiation via
  `sched_setaffinity` and source inspection; ran the documented ASAN
  cancellation-safety command for real (clean, no UAF/double-free); ran all
  21 new tests (cancellation, segment-roll, thread-per-core, `SimDriver`
  fault injection, segment-log crash recovery) — all pass; re-measured
  dependency footprint at 28 crates (still roughly a tenth of compio's 231).
- **Bonus:** `rusty_tokio`'s `io-uring-fs` ships an `OpDriver` trait with a
  real `SimDriver` (torn writes, lying fsyncs, disk-full, crash/reopen) —
  exactly what ADR-0002 D4 asked for as a plan, delivered as a working
  implementation. D4 now builds directly on `OpDriver`/`SimDriver` instead of
  hand-rolling a parallel `Storage`/`Clock` trait.
- **Known limitation, stated plainly, not a shortfall:** `rusty_tokio`'s
  `io-uring-fs` uses one process-wide io_uring ring, not one per core —
  a deliberate, documented choice (correctness over per-core I/O throughput),
  consistent with this project's own stance on not chasing throughput
  records. Only the *scheduling* side (tasks, sockets, timers) is fully
  per-core; disk I/O submission still synchronizes through one driver thread.

## ADR-0002 D3: runtime decision closed — compio 0.18.0, after checking all three
**2026-08-01**

- **Changed:** ran the same source-level spike (stable-Rust build + real
  program + driver/reactor source inspection) against glommio 0.9.0 and monoio
  0.2.4 that had just been run against compio, rather than pin-and-move-on
  after finding compio's stated rationale was wrong.
- **Found:** none of the three runtimes expose a public seam for injecting
  simulated per-operation disk I/O — glommio's `Reactor` is even more
  monolithic than compio's (`pub(crate)`, single non-generic `Rc`, no
  alternate-backend cfg selection at all); monoio's `Driver` trait is
  genuinely public and `Runtime<D>` is generic over it, but that trait only
  covers executor scheduling, not the actual read/write/fsync dispatch (which
  is `pub(crate)` and hardcoded to a closed enum). This neutralizes
  DST-pluggability as a differentiator entirely — D4's own `Storage`/`Clock`
  trait abstraction is what actually delivers DST, regardless of runtime.
- **Also reconfirmed, not stale:** DataDog/glommio#707 ("call for
  maintainers") is still open and unresolved; monoio's io_uring feature-parity
  gap and slow maintenance pace (flagged by Iggy) still hold as of a fresh
  2026-08-01 check.
- **Decided:** compio, pinned to exactly `0.18.0` (not a range), stays the
  Phase 1 runtime — now on the strength of dependency modularity and
  maintainer responsiveness rather than the falsified driver-swappability
  claim. D3 in ADR-0002 is closed, not provisional.

## ADR-0002 D3: compio validation spike — core rationale falsified, decision reopened
**2026-08-01**

- **Changed:** ran the D3 validation spike the ADR called for. Built and ran a
  minimal compio (0.19.1) file-I/O program in this environment and read the
  actual `compio-driver`/`compio-runtime` source rather than relying on docs.
- **Fixed (a wrong claim in the original ADR):** D3's stated reason for
  preferring compio over glommio/monoio — "the only one of the three engineered
  to let the I/O driver be swapped for a simulated one" — is false. `Proactor`
  and `Runtime` are non-generic, and the `Driver` type is chosen entirely at
  compile time from a closed set of built-in backends; there's no public trait
  a downstream crate can implement to inject a simulated driver. This does not
  block D4's testing strategy (which abstracts at the team's own `Storage`/
  `Clock` trait boundary, not the runtime's internals), but the specific reason
  given for picking compio was wrong and needed correcting, not quietly kept.
- **Found (new, unplanned):** current compio (0.19.0/0.19.1) does not compile
  on stable Rust — an unconditional use of the newly-and-not-yet-stably
  stabilized `cfg_select!` std macro. Confirmed via bisection: `compio` 0.18.0
  builds cleanly on stable, 0.19.0 does not. Likely temporary (stable Rust's
  ~6-week release cycle should catch up), but real today.
- **Known limitation:** D3 is intentionally left open rather than force-closed
  — the ADR now names two concrete paths to finalize it (pin to 0.18.0 and
  accept the non-pluggable-driver finding, or re-run this same source-level
  check against glommio/monoio) rather than picking one without the same rigor
  applied to the alternatives.

## ADR-0002: Phase 1 foundational decisions
**2026-08-01**

- **Added:** `docs/adr/0002-phase1-foundational-decisions.md` — resolves all four
  `docs/phase1-scope.md` §6 open questions with cited research: skip Kafka
  wire-protocol compatibility (build on `rusty_wire`); defer the VSR-vs-Raft
  choice to Phase 2 but lean VSR and require consensus-ready storage primitives
  now (durable/committed-offset split, truncatable log tail, epoch/fencing-token
  field); compio as a provisional runtime choice pending a validation spike;
  coexist with NATS JetStream behind an explicit, criteria-based re-evaluation
  gate rather than replacing it outright.
- **Added:** a concrete DST testing strategy (injectable `Storage`/`Clock`
  traits from the first storage-engine commit, three minimal fault-injection
  tests) and a set of "consumer gates" the storage engine must clear before
  Phase 1 is considered done.
- **Known limitation:** the runtime choice (D3) is explicitly provisional —
  the ADR names a validation spike that hasn't run yet. No implementation
  lands in this change; this is research/ADR only, per the scope doc's gate.

## Repo setup — minimal CI workflow + main branch
**2026-08-01**

- **Added:** `main` branch on `origin`, created from the governance-scaffolding
  commit and now the repo's default branch.
- **Added:** `.github/workflows/ci.yml` — a minimal `check` job (name matches the
  required-status-check convention in the repo-config reference) that no-ops green
  until a `Cargo.toml` exists, then automatically runs `cargo fmt`/`clippy`/`test`.
  Exists now so branch protection has a real check to gate on rather than one
  that's never reported.
- **Known limitation:** branch protection on `main` is still unset — no tool in
  this environment reaches GitHub's branch-protection or repo-settings API, so
  that (require PR, require the `check` status, require up-to-date branches, and
  disabling squash/rebase merge in repo settings) remains a manual step.

## Repo setup — Phase 1 scope doc + governance scaffolding
**2026-08-01**

- **Added:** trimmed copy of the Phase 1 pre-RFC research brief
  (`docs/phase1-scope.md`) — dropped the "governance-native data contracts"
  differentiator per direction to leave that out for now.
- **Added:** standard governance file set via `repo-config` — README, ARCHITECTURE,
  CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, PR/issue templates, ADR seed.
- **Known limitation:** no `Cargo.toml` yet, so no CI workflow was added — nothing
  to run. `ARCHITECTURE.md`'s boundary table is left as scaffold since no code has
  landed. This repo has no `main` branch yet (root commit lives on
  `claude/review-attached-document-3at8q9`), so the PR-per-change workflow doesn't
  apply until one exists.
