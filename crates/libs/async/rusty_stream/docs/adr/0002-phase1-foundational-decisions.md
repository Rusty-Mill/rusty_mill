# ADR-0002: Phase 1 foundational decisions — wire protocol, consensus posture, runtime, testing strategy

Status: Proposed
Date: 2026-08-01

## Dual mandate

This ADR has two jobs, not one:

1. **Understanding** — answer the four open questions from `docs/phase1-scope.md` §6
   with real research (sources cited throughout), not house priors. Each decision
   below is backed by a dedicated research pass over the primary sources named in
   the scope doc's §5 reading list, plus targeted follow-up search.
2. **Foundation** — turn those answers into concrete constraints on the Phase 1
   storage-engine design *now*, so later phases (clustering, DST hardening) don't
   force an on-disk format migration or a runtime rewrite to retrofit what this
   ADR could have required from commit one.

Non-goal of this ADR: resolving every implementation detail. Where research
surfaced a decision that genuinely can't be made without empirical validation
(the runtime spike, most notably), this ADR says so plainly rather than
manufacturing false confidence.

## Context

`docs/phase1-scope.md` gates implementation on: (a) the required reading in its
§5, and (b) answers to its four §6 open questions. That reading and research pass
is now done — see the per-question sections below for sources. This ADR is the
"draft an ADR ... before any code lands" step called for in the scope doc's §7.

## Decisions

### D1 — Wire protocol: skip Kafka compatibility, build on `rusty_wire`

**Decision:** rusty_stream does not implement Kafka wire-protocol compatibility.
It uses `rusty_wire` as a first-class, purpose-built protocol, and treats this as
a final choice for planning purposes rather than a "compat shim later" hedge.

**Reasoning:** Kafka wire compatibility (Blink, Redpanda) buys ecosystem
gravity — point existing, uncontrolled clients at the broker with zero migration
friction. That payoff only matters when there's an external client population you
don't control. rusty_stream serves a single governed enclave where every
producer/consumer is internal and already Rust-first; there is no unknown-client
population to placate, so the payoff is mostly unrealized here. The cost is fully
realized regardless: an open-ended, externally-paced KIP surface to track
(Redpanda's own docs carry an explicit compatibility-gap matrix), which cuts
directly against the sovereignty goal of being auditable and self-contained.
The one-way-door risk actually favors this choice — a first-party protocol only
creates lock-in to ourselves, which is the correct direction for an internal
ecosystem project (this mirrors Iggy's stance).

**Tradeoffs accepted:** no zero-friction path for a future external tool that only
speaks Kafka protocol (Kafka Connect, MirrorMaker); every client, connector, and
observability integration must be built in-house against `rusty_wire`.

**What would flip this:** RustyMill's charter changing from "governed enclave" to
needing interop with external Kafka-speaking infrastructure — at which point a
narrow, explicitly-scoped read-only Kafka-shaped export gateway (not full wire
compat baked into the core protocol) is the better-scoped fallback, not a reason
to revisit this decision now.

Sources: Blink (https://github.com/cleafy/blink), Iggy's own-protocol rationale
(https://blog.iggy.rs/posts/building-message-streaming-in-rust/,
https://iggy.apache.org/docs/faq/faq/), RobustMQ's critique of Iggy's ecosystem
tradeoff (https://robustmq.com/en/Blogs/34), Redpanda's KIP-lag/compatibility
caveats (https://docs.redpanda.com/current/develop/kafka-clients/).

### D2 — Consensus: defer the protocol/crate choice to Phase 2, lean VSR, build consensus-ready primitives now

**Decision:** Do not adopt `openraft`, `raft-rs`, or a VSR implementation in
Phase 1. Defer the concrete choice to Phase 2, gated on an actual second-enclave
forcing function, per the scope doc's anti-speculation principle. Record a
**working-hypothesis lean toward VSR** for when that day comes. Phase 1's storage
engine must expose the primitives both VSR and Raft need, so the eventual choice
doesn't force a format migration (see "Storage engine implications" below).

**Reasoning:** VSR's deterministic round-robin primary rotation (no
randomized-election split votes, no stable-storage requirement for correctness)
is a genuine failure-model fit for a governed deployment where predictable
failover matters more than internet-scale elasticity — this is real signal, not
fashion, per Iggy's and TigerBeetle's rationale. But two facts argue against
committing now: rusty_stream's near-term shape is a small (2-3 node) fixed
cluster, not the large dynamically-reconfigured fleets `openraft`'s flagship
features exist to serve; and there is no mature, standalone Rust VSR crate — Iggy
hasn't shipped its own VSR clustering yet, and TigerBeetle's implementation is
Zig, not a reusable Rust dependency. Adopting VSR today means building
from-scratch against a paper with documented ambiguities (Vanlightly's TLA+
analysis calls it "at times too vague, at times contradictory"). That's real
implementation risk to take on before there's a concrete second-enclave need.

**What "consensus-agnostic Phase 1" concretely requires** (both protocols
converge on these — this is the foundation half of this ADR's mandate):
- Monotonic, append-only offsets per partition/segment (already natural for the
  segment-log design).
- A first-class **durable-offset vs. committed/visible-offset (high-watermark)**
  distinction, even in single-node mode.
- Ability to **truncate an uncommitted log tail** without corrupting segment
  files — needed by both VSR view-change and Raft term-change recovery.
- An **epoch/fencing-token field** in segment/index metadata (a VSR view-number or
  Raft term are both instances of this one primitive), so a future consensus
  layer attaches without an on-disk format migration.
- Do not hard-code Raft-specific invariants (e.g. "a new leader always has the
  full committed log") into the storage engine — VSR's recovery/state-transfer
  model violates that assumption, and baking it in would quietly foreclose VSR.

**Tradeoffs accepted:** Phase 2 starts with less certainty and a possible
reevaluation cycle when the real forcing function shows up; leaning VSR later
means accepting unproven-in-Rust implementation risk versus `openraft`'s
production track record — a knowingly deferred risk, not a dismissed one.

Sources: Liskov & Cowling VSR paper (content triangulated via
https://jack-vanlightly.com/analyses/2022/12/20/vr-revisited-an-analysis-with-tlaplus
and https://charap.co/reading-group-viewstamped-replication-revisited/ — the
primary PDF was unreachable from this environment), `openraft`
(https://github.com/databendlabs/openraft), `raft-rs`
(https://github.com/tikv/raft-rs), Iggy's io_uring migration post confirming VSR
but deferring detailed rationale to a forthcoming post
(https://iggy.apache.org/blogs/2026/02/27/thread-per-core-io_uring/), TigerBeetle
VSR internals (https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vsr.md).

### D3 — Runtime: `rusty_tokio` (`thread-per-core` + `io-uring-fs`) — decided, after three rounds of spikes

**Decision (final, revised 2026-08-01):** `rusty_tokio` — RustyMill's own
hand-rolled async runtime — with its `thread-per-core` and `io-uring-fs`
features, is the Phase 1 runtime. This **supersedes** the earlier compio
decision recorded below, kept intact as the record of how this comparison
actually evolved rather than quietly rewritten. Round 1 falsified compio's
stated rationale; round 2 checked glommio and monoio with the same rigor and
found the same DST-pluggability gap in all three, closing D3 on compio for
reasons unrelated to the original claim; round 3 (this one) checked
`rusty_tokio` — previously ruled out only because it lacked thread-per-core
scheduling and real io_uring file I/O — after those two gaps were closed by a
real implementation (`baileyrd/rusty_tokio#252` → `#253`), verified directly
rather than taken on trust. See "Spike results, round 3" below for exactly
what was checked and how.

**Round 2 decision (2026-08-01, superseded by round 3 below):** compio,
pinned to `=0.18.0`, is the Phase 1 runtime. Reached in two rounds: the first
spike falsified the original driver-swappability rationale for compio
specifically; rather than quietly keep a wrong reason or pick a replacement
without equal scrutiny, the same source-level check was then run against
glommio and monoio too (see "Spike results, path (b)" below). All three fail
the DST-pluggability test the same way, which neutralizes it as a
differentiator and makes D4's own `Storage`/`Clock` abstraction load-bearing
regardless of which runtime is chosen. On the axes that remain real —
stable-Rust buildability, maintenance health, audit surface — compio still
came out ahead of glommio/monoio at that point, for reasons that had nothing
to do with the original claim. This reasoning is preserved below because it's
still exactly right as a comparison *among those three* — it's `rusty_tokio`
entering the comparison newly-qualified that changes the outcome, not a flaw
found in this round-2 reasoning itself.

**Original decision (2026-08-01, pre-spike):** compio is the working choice for
the thread-per-core/io_uring runtime, but this is explicitly provisional — not a
locked-in decision — pending a validation spike (defined below). This is a "best
of three imperfect options" call, not a strong conviction, and the ADR says so
rather than manufacturing false certainty.

**Reasoning:** None of compio, glommio, or monoio have documented integration
with deterministic-simulation tooling (`madsim`, `turmoil`) — both of those are
built around a Tokio-shaped API surface, not thread-per-core io_uring runtimes.
So "DST support" isn't a pick-off-the-shelf property of any candidate; a custom
deterministic I/O layer will be built regardless (see D4). What differs is which
runtime's *architecture* makes that buildable: compio's explicitly decoupled
driver/executor design — the same reason Iggy cites for adopting it — is the only
one of the three engineered to let the I/O driver be swapped for a simulated one.
Glommio is a tightly-integrated Seastar-derived scheduler with less separable
internals and requires Linux kernel ≥5.8 plus `memlock` rlimit tuning, a real
friction point for a locked-down/air-gapped target. Monoio drops `Send`/`Sync`
for performance, complicating reuse of general async tooling, and Iggy's own
bake-off found it behind on io_uring feature parity as of their evaluation.

Maturity is a genuine concern across all three, stated honestly: compio is
pre-1.0 and still churning API surface; glommio has real production pedigree at
Datadog but posted a public "call for maintainers" issue; monoio has hyperscale
production use at ByteDance but was rejected by Iggy on completeness grounds
relevant to a storage engine. None is a "boring, stable" choice.

**Validation spike required before this is final:**
1. Attempt to stub a fake io_uring driver behind compio's driver/executor seam —
   if this hits a hard wall, the core rationale collapses and the choice must be
   revisited.
2. Re-check monoio's current io_uring feature parity — Iggy's assessment may be
   stale by the time Phase 1 storage-engine work starts.
3. Re-check glommio's maintainer situation — if resolved, its production track
   record becomes more attractive relative to compio's pre-1.0 churn.

**Spike results (2026-08-01), item 1 — driver-swappability:**

Ran a minimal compio program (`compio-driver` 0.12.4 / `compio` 0.19.1) against
this environment and inspected the crate's actual source, not just its docs.

- *Functional check, positive:* a basic create/write/fsync/read cycle through
  compio's real io_uring driver works correctly in this sandboxed container
  (kernel 6.18.5, `io_uring` enabled). Not itself part of the spike's ask, but
  good signal that the target environment isn't a blocker on its own.
- *Driver-swappability, falsified:* `compio_driver::Proactor` (lib.rs:101) is a
  plain, non-generic struct. The `Driver` type it wraps
  (`compio-driver-0.12.4/src/sys/driver/mod.rs`) is chosen entirely at compile
  time by a `cfg_select!` macro across a **closed** set of concrete backends —
  `poll`, `iour` (io_uring), `iocp`, `fusion` (poll+iour switchable *at runtime*,
  but still both first-party), and a crate-internal `stub` used only when no OS
  backend feature is enabled. `Driver` itself is `pub(crate)` in the stub
  variant — not part of compio's public API at all. `compio-runtime`'s `Runtime`
  struct holds a concrete `Rc<RefCell<Proactor>>`, not a generic parameter or
  trait object. **There is no public trait a downstream crate can implement to
  inject a fifth, simulated backend** — doing so would require forking
  `compio-driver` and adding a new arm to its internal `cfg_select!`, not
  "swapping a driver" through any exposed extension point. This directly
  contradicts the reasoning above ("the only one of the three engineered to let
  the I/O driver be swapped for a simulated one") — that claim does not survive
  contact with the source and must be treated as false, not provisional.
- *Why this doesn't sink D3 outright:* D4 already committed to abstracting
  storage I/O behind the team's **own** `Storage`/`Clock` traits, independent of
  whichever runtime backs the real implementation — not to relying on any
  runtime's internal driver being swappable. Under D4's actual design, the
  simulated test path never calls into compio's `Proactor` at all; it's a
  separate in-memory implementation of the team's own trait. So compio's
  internal non-pluggability, while a real architectural fact worth recording
  accurately, turns out not to block D4's testing strategy the way the original
  reasoning assumed it would matter. It does mean the stated *reason* to prefer
  compio over glommio/monoio on DST grounds was wrong — if that comparison needs
  to be redone, it should be redone without this axis, or with this same
  source-level check re-run against the other two (neither was checked this
  rigorously either; the original research pass reasoned about their
  architecture at a distance, not from source).
- *Independent, unplanned finding — current compio does not build on stable
  Rust:* `compio` 0.19.0/0.19.1 (`compio-driver` 0.12.4) fails to compile on
  this environment's stable toolchain (rustc 1.94.1) with
  `error[E0658]: use of unstable library feature 'cfg_select'`, unconditionally
  — not gated behind any optional feature, confirmed by testing with only
  default features enabled. It compiles cleanly on nightly
  (rustc 1.99.0-nightly 2026-07-31) with no `#![feature(cfg_select)]` opt-in
  anywhere in the crate, which lines up with std's `cfg_select!` macro having
  been very recently stabilized on the nightly channel
  (rust-lang/rust#152944, tracking release notes for #149783) but not yet
  reaching a stable release as of this environment's toolchain. Bisecting by
  version: `compio` 0.18.0 (`compio-driver` 0.11.4) builds cleanly on stable;
  the regression was introduced going into `compio` 0.19.0
  (`compio-driver` 0.12.4). This is very likely temporary — stable Rust ships
  roughly every six weeks, so this probably self-resolves within one or two
  releases — but it is real *today*, and it's a second, independent data point
  (beyond the driver-pluggability finding) for "pre-1.0, still churning,
  occasionally ahead of what stable Rust actually supports." A team that cares
  about being auditable and built carefully should not currently pin to
  `compio` latest without either accepting a nightly-toolchain dependency or
  deliberately pinning back to 0.18.0.

**Status after this first round:** D3 was not resolved by this spike alone — it
was better-informed and more honest about its risk than before, but not final.
Two concrete, sourced findings now existed that weren't available when this ADR
was first drafted. Closing this out required either (a) pin to `compio` 0.18.0,
explicitly accept that its internal driver cannot be swapped for DST purposes
(relying entirely on D4's higher-level trait abstraction instead), and set a
reminder to re-check stable
Rust's `cfg_select` stabilization before upgrading past 0.18.0; or (b) re-run
this same source-level check (stable-Rust buildability, actual public
pluggability) against glommio and monoio before picking between the three, since
neither of those got the same rigor applied to the axis that was supposed to be
compio's deciding advantage.

Sources: Iggy's thread-per-core/io_uring migration writeup
(https://iggy.apache.org/blogs/2026/02/27/thread-per-core-io_uring/), compio
(https://github.com/compio-rs/compio), glommio
(https://github.com/DataDog/glommio,
https://github.com/DataDog/glommio/issues/707), monoio
(https://github.com/bytedance/monoio), madsim
(https://github.com/madsim-rs/madsim), turmoil
(https://tokio.rs/blog/2023-01-03-announcing-turmoil). Spike sources: direct
source inspection of `compio-driver` 0.12.4 and `compio-runtime` 0.12.3 from
crates.io (`Proactor`/`Driver`/`Runtime` definitions), local build/bisect
against `compio` 0.15.0–0.19.1 on rustc 1.94.1 (stable) and rustc
1.99.0-nightly 2026-07-31, rust-lang/rust#152944 (cfg_select stabilization
tracking) and #115585 (original cfg_select tracking issue).

**Spike results (2026-08-01), path (b) — glommio and monoio, same rigor:**

Rather than pin-and-move-on, ran the identical check (minimal real program,
stable-Rust build, source inspection of the actual driver/reactor/op-dispatch
layer) against glommio 0.9.0 and monoio 0.2.4.

*glommio 0.9.0:*
- Builds clean on stable Rust with default features — no equivalent of compio's
  regression.
- Functional: a real `DmaFile` create/write/fdatasync/read cycle ran correctly
  in this environment (kernel 6.18.5, well above the ≥5.8 floor; the default
  `memlock` ulimit here, 8192, was sufficient — worth re-checking against
  whatever ulimits the actual deployment target enforces, since this is a
  documented friction point elsewhere, just not one this sandbox happened to
  hit). Required manually aligning the write buffer to `file.alignment()` —
  glommio's `DmaFile` is O_DIRECT-based, so callers own alignment; a real
  storage-engine cost, arguably also a feature for a project that wants precise
  control over its I/O path.
- Driver-swappability: **also false, more so than compio.** `Reactor`
  (`glommio-0.9.0/src/reactor.rs`) is `pub(crate)`, held as a single
  non-generic `Rc<Reactor>` on `LocalExecutor`. Unlike compio, there isn't even
  a closed set of alternate backends selected by cfg — `glommio-0.9.0/src/sys/`
  contains only `uring.rs`; the crate is Linux/io_uring-only by construction,
  no polling/IOCP fallback at all. No public seam exists here either.
- Maintenance: re-checked DataDog/glommio#707 ("call for glommio maintainers,"
  opened 2026-03-10) — still open, no resolution, as of this spike. The
  bus-factor risk flagged in the original research is unresolved and current,
  not stale.

*monoio 0.2.4:*
- Builds clean on stable Rust, functional (file create/write/fsync/read cycle
  ran correctly).
- Driver-swappability: **genuinely more nuanced than the other two, but the
  practical answer is the same.** monoio's `Driver` (`monoio-0.2.4/src/driver/
  mod.rs`) is an actual `pub trait`, and `Runtime<D>` (`src/runtime.rs`) is
  generic over it — `IoUringDriver` and `LegacyDriver` (mio-based) both
  implement it, and in principle a third party could implement `Driver` for a
  custom type. But `Driver` only governs executor-level scheduling (`with`,
  `submit`, `park`, `park_timeout`, `unpark`) — it does not cover individual
  I/O operations. Actual op dispatch (open/read/write/fsync) goes through
  `OpAble` (`src/driver/op.rs`), which is `pub(crate)` and hard-matches a
  closed `Inner` enum (`Uring | Legacy`), not parameterized by `D: Driver`. So
  the one public trait in reach of this design doesn't cover the thing DST
  fault-injection would actually need to intercept (per-operation behavior on
  read/write/fsync). Net result: no usable public seam here either, despite the
  more promising-looking API surface.
- Feature parity/maintenance, re-checked fresh (not relying on Iggy's
  potentially-stale assessment): still confirmed behind on io_uring feature
  parity as of this research, and maintenance pace described as not keeping up
  with io_uring's evolution, patches arriving mainly reactively. Iggy's
  original assessment holds; it was not stale.

**What this changes:** the DST-pluggability axis — the reason D3 originally
leaned toward compio — turns out to differentiate **none** of the three. All
three require the same thing: the team's own `Storage`/`Clock` trait
abstraction from D4, sitting above whichever runtime is chosen, because none of
the three expose a per-operation I/O seam a downstream crate can hook. That's
actually a useful result — it means D4's approach is validated as necessary
regardless of D3's outcome, and D3 can now be decided on the axes that remain
real: stable-Rust buildability, maintenance health, and audit/dependency
surface.

**Round 2 outcome (superseded — see round 3 below):** compio, pinned to
0.18.0, was the pick at this point — for different reasons than originally
written, not the original ones. Reasoning on the axes that survived scrutiny:
glommio carries a live, unresolved bus-factor risk (open maintainer-succession
issue) and the least modular/most monolithic dependency structure of the
three; monoio has independently reconfirmed io_uring feature-parity gaps and a
maintenance pace that doesn't track io_uring's evolution; compio's regression
is narrow (one recent release, plausibly self-resolving as `cfg_select`
reaches stable Rust), its dependency structure is the most modular of the
three (pull only the `compio-*` crates a feature needs), and Iggy's own
experience found its maintainers responsive. This round did not consider
`rusty_tokio` — at the time, it lacked thread-per-core scheduling and real
io_uring file I/O, and had never been spiked at all. Round 3 below is what
changed that.

Sources (glommio/monoio spike): direct source inspection of `glommio` 0.9.0
(`Reactor`, `LocalExecutor`, `src/sys/`) and `monoio` 0.2.4 (`Driver` trait,
`Runtime<D>`, `OpAble`, `Inner` enum) from crates.io; local build/run of both on
rustc 1.94.1 (stable) in this environment; DataDog/glommio#707 re-checked
2026-08-01; WebSearch re-check of monoio io_uring feature parity/maintenance
pace, 2026-08-01.

**Spike results (2026-08-01), round 3 — `rusty_tokio`:**

`rusty_tokio` was initially out of the D3 comparison for two concrete reasons,
found by reading its source directly: its scheduler was multi-threaded
work-stealing, not thread-per-core, and its `fs::File` was entirely
`spawn_blocking` — no io_uring involvement in file I/O at all (the existing
`io-uring-reactor` feature only covers *socket* readiness via
`IORING_OP_POLL_ADD`, never a file). Those two gaps were written up as a
concrete engineering handoff (`baileyrd/rusty_tokio#252`) and filed as an
issue rather than assumed away. `baileyrd/rusty_tokio#253` closed it with a
real implementation — verified here directly, not taken on the PR description's
word:

- *Builds clean on stable Rust:* `cargo +stable build --features
  thread-per-core,io-uring-fs` — no nightly requirement, unlike compio 0.19.x.
- *Real io_uring file I/O, not spawn_blocking:* straced the new
  `thread_per_core_uring_smoke` example (`strace -f -e trace=io_uring_enter,
  io_uring_setup,pread64,pwrite64,openat`). File operations go through
  `io_uring_enter`; the only `pread64` calls observed were the dynamic linker
  loading `libc.so.6` at process startup, not application data — confirmed by
  inspecting their actual byte content (ELF program-header bytes at a fixed
  offset), not assumed from the syscall name alone.
- *Genuine per-core scheduling, not just per-core threads:* straced
  `sched_setaffinity` calls pinning four distinct threads to CPU cores
  `[0]`/`[1]`/`[2]`/`[3]`. Source confirms this isn't cosmetic:
  `Builder::build_thread_per_core` (`src/runtime/mod.rs`) loops
  `for _ in 0..n { cores.push(self.new_core_shared(...)) }`, and
  `new_core_shared` constructs a fresh `Reactor::new()` + `TimerDriver::new()`
  per call — each core's socket/timer reactor is a genuinely independent
  instance, not a shared one wearing a per-core label.
- *One honest nuance, not a shortfall:* the file-I/O side (`IoUringDriver` in
  `src/io/uring_fs.rs`) is a single process-wide ring, not one ring per core —
  confirmed via `static GLOBAL_DRIVER: Mutex<Option<Arc<dyn OpDriver>>>` and
  the module's own top-of-file docs: "this isn't a throughput-oriented
  per-core ring setup... it's the minimum needed for correct, real io_uring
  file I/O." This is a deliberate, clearly-documented choice, not a gap
  against what was actually asked for — this ADR's own §4 already ruled out
  competing with Iggy/Redpanda on raw throughput, and the shared-ring design
  is exactly the kind of complexity that tradeoff says to avoid. It does mean
  disk-I/O submission itself synchronizes across cores through one driver
  thread; only the *scheduling* (task execution, socket/timer readiness) is
  fully per-core. Worth knowing, not disqualifying.
- *Cancellation safety verified under real ASAN, not just described:* ran the
  exact command documented in `src/io/uring_fs.rs`'s own module docs
  (`RUSTFLAGS="-Zsanitizer=address" cargo +nightly test --features
  io-uring-fs -Zbuild-std --target x86_64-unknown-linux-gnu --test
  uring_fs_cancellation`) — clean, no use-after-free or double-free detected
  dropping an in-flight `read_at`/`write_at` before completion.
- *The DST seam this ADR's D4 needs, delivered as a side effect, not
  requested as a hard requirement:* `UringFile` is generic over an `OpDriver`
  trait; `IoUringDriver` (real) and `SimDriver` (in-memory, deterministic,
  with `inject_torn_write`/`set_fsync_lies`/`set_disk_full_at`/
  `crash_and_reopen`) both implement it. `tests/uring_fs_sim_driver.rs` (7
  tests) and `tests/rusty_stream_segment_log_recovery.rs` (4 tests) exercise
  exactly the fault classes D4 asked for — torn writes, lying fsyncs,
  disk-full, repeated crash/recovery cycles — and all pass. Full suite run:
  `tests/uring_fs_cancellation.rs` (2), `tests/uring_fs_segment_roll.rs` (4),
  `tests/thread_per_core_uring_fs.rs` (4), `tests/uring_fs_sim_driver.rs` (7),
  `tests/rusty_stream_segment_log_recovery.rs` (4) — 21/21 pass.
- *Dependency footprint, re-measured with the new features on:* `cargo tree`
  reports 28 crates (`thread-per-core` adds zero new dependencies — pure
  scheduler plumbing; `io-uring-fs` adds the same `io-uring` crate
  `io-uring-reactor` already depended on, plus its few transitives), up from
  26 at default. Still roughly a tenth of compio's 231-crate `--all-features`
  tree, and org-owned end to end.

**Why this changes D3's answer:** round 2 closed on compio specifically
because it was "the most modular, most responsive" among three third-party
crates that all failed the DST-pluggability test identically. `rusty_tokio`
was never in that comparison — it wasn't a candidate, it was a gap-filled
afterthought until its two disqualifying gaps closed for real. With both
closed and independently verified: it has a materially smaller dependency
tree than any of the three (28 vs. compio's 231), it's org-owned rather than
a third-party dependency (the sovereignty/audit goal this whole ADR keeps
returning to), it doesn't carry compio's stable-Rust regression, and it ships
a genuine DST fault-injection seam that D4 asked for as an aspiration and got
as a working implementation. There is no axis left where compio still wins.

Sources (`rusty_tokio` spike): direct source inspection of
`src/runtime/mod.rs`, `src/runtime/thread_per_core.rs`,
`src/io/uring_fs.rs` at commit `54fc16c` (baileyrd/rusty_tokio); local
build/test/strace/ASAN runs in this environment, 2026-08-01;
`baileyrd/rusty_tokio#252` (the handoff issue this ADR's D3 gap-analysis
produced) and `#253` (the implementation PR that closed it).

### D4 — Testing strategy: `rusty_tokio`'s `SimDriver` (`io-uring-fs`), not a hand-rolled injectable trait

**Decision (revised 2026-08-01 — the seam now exists, not just the plan):**
Originally scoped as: the storage engine builds its own injectable
`Storage`/`Disk` trait and `Clock` trait from the first commit, with a real
implementation and an in-process simulated one shipping together. That plan
is now superseded by something better than what it asked for: `rusty_tokio`'s
`io-uring-fs` feature (adopted in D3) already ships exactly this seam —
`UringFile` is generic over an `OpDriver` trait, with `IoUringDriver` (real
io_uring) and `SimDriver` (in-memory, deterministic, seeded fault injection)
both implementing it. rusty_stream's storage engine builds directly on
`OpDriver`/`SimDriver` rather than hand-rolling a parallel abstraction that
would just duplicate it. This is *not* a bet on `madsim` or `turmoil` as a
runtime replacement, and it's not a bet on an unimplemented plan either — the
tests in `rusty_tokio`'s `tests/uring_fs_sim_driver.rs` and
`tests/rusty_stream_segment_log_recovery.rs` (verified passing, see D3) are a
working reference for exactly this pattern, not a target to build toward.

**Reasoning:** rusty_stream Phase 1 is single-node — the non-determinism that
actually needs controlling is disk timing/ordering and process crashes, not
network partitions. `madsim` and `turmoil` are primarily network simulators built
by intercepting Tokio internals; their disk-simulation support is thin to
nonexistent, and neither has documented compatibility with an io_uring
thread-per-core runtime. A purpose-built `Disk`/`Clock`-shaped trait pair gives
full control over exactly the fault classes a storage engine needs to
prove — this mirrors how TigerBeetle and comparable Rust storage projects
(KayaDB, S2) actually structured their DST, and it's no longer merely a plan
mirroring theirs: `rusty_tokio`'s `SimDriver` (`inject_torn_write`,
`set_fsync_lies`, `set_disk_full_at`, `crash_and_reopen`) is that pattern,
already built, already org-owned, already exercised by tests that match the
three minimal scenarios below almost line for line.

**Sequencing risk with D3, now resolved rather than merely flagged:** the
original concern was that network-level DST (`madsim`/`turmoil`) for Phase 2
clustering would pull toward a Tokio-based runtime, creating tension with an
io_uring thread-per-core choice. Since D3 landed on `rusty_tokio` — org-owned
code, not a third-party crate to fight for compatibility with — that tension
is gone: a future network-fault-injection seam for Phase 2 can be added to
`rusty_tokio` directly, the same way the `io-uring-fs`/`SimDriver` seam was,
rather than requiring a `madsim`/`turmoil` integration this crate was never
going to have.

**Minimal viable first DST tests** (write these against the storage engine
before considering it Phase-1-complete — see consumer gates; a directly
analogous version of each already exists and passes against `rusty_tokio`'s
`SimDriver` in `tests/rusty_stream_segment_log_recovery.rs`, confirmed in D3 —
rusty_stream's own tests should follow that shape against its actual segment
format, not start from scratch):
1. **Crash during segment roll** — seeded fault kills the process mid-roll;
   recovery must never lose an offset acknowledged before the roll began, and
   must land on a single consistent active segment.
2. **Torn write on last segment** — corrupt/truncate the tail of the active
   segment mid-record; recovery must detect the incomplete record via
   length/checksum and truncate to the last valid boundary, never crash or
   silently serve corrupt data. (`SimDriver::inject_torn_write` — verified,
   see D3.)
3. **fsync fault** — simulate fsync reporting success without persisting, or
   reordering completion versus subsequent writes; verify the durability
   boundary matches the configured fsync policy exactly under simulated
   crash-and-restart. (`SimDriver::set_fsync_lies` — verified, see D3.)

Sources: TigerBeetle's DST writeup
(https://tigerbeetle.com/blog/2023-07-11-we-put-a-distributed-database-in-the-browser/),
DST primer (https://www.amplifypartners.com/blog-posts/a-dst-primer-for-unit-test-maxxers),
madsim (https://github.com/madsim-rs/madsim), turmoil
(https://docs.rs/turmoil/latest/turmoil/), S2's DST writeup
(https://s2.dev/blog/dst), KayaDB (https://lib.rs/crates/kaya-io). See D3 for
`rusty_tokio`-specific sources (`src/io/uring_fs.rs`, `#252`/`#253`).

### D5 — NATS JetStream: coexist with an explicit re-evaluation gate, not an outright replacement

**Decision:** The NATS JetStream recommendation is not replaced outright when
Phase 1 ships. rusty_stream and JetStream coexist through Phase 1 and into early
Phase 2, with an explicit, criteria-based gate for when the JetStream
recommendation retires for new work.

**Reasoning:** JetStream today provides replication/HA (RAFT-based, R=1/2/3) and
consumer-group rebalancing that Phase 1 explicitly does not have (single-node, no
replication, no rebalancing per the scope doc's non-goals). Declaring rusty_stream
the default now would mean recommending a strictly weaker HA story than the thing
it replaces, for workloads that may need HA — not a credible claim for an ADR to
make. The honest framing is strangler-fig-style: the new system grows alongside
the incumbent, proven in low-stakes territory first.

**Coexistence path:**
- **Pilots on rusty_stream first:** new, low-stakes, single-node-tolerant
  pipelines — internal batch/log-shipping, dev/staging — where no HA/failover
  requirement exists and consumers are fixed or manually partitioned (no dynamic
  rebalancing need), by team opt-in.
- **Stays on JetStream:** anything needing replication/HA today or dynamic
  consumer scaling via queue groups — the default for production data-plane
  traffic through Phase 1.
- **Fallback if the Phase 1 pilot doesn't pan out:** pilot workloads roll back to
  JetStream; no other team migrates; rusty_stream is shelved or rescoped,
  consistent with the scope doc's "not a multi-year detour" guardrail.
- **Re-evaluation gate** for retiring the JetStream recommendation for *new*
  work: Phase 2 clustering/replication has shipped, at least one real
  (non-pilot) production workload has run on rusty_stream Phase 2 for a defined
  burn-in (e.g. three months) without a durability or availability incident, and
  consumer-group rebalancing is implemented and tested under node loss.

**Key risks:** coexistence costs dual client SDKs and two operational runbooks
in the interim; replacing outright would risk production data loss/downtime on
an unproven, non-HA system and a costly reverse migration — a strictly worse
failure mode than the coexistence overhead.

Sources: NATS JetStream architecture (https://docs.nats.io/nats-concepts/jetstream),
streams reference (https://docs.nats.io/nats-concepts/jetstream/streams),
JetStream clustering (https://docs.nats.io/running-a-nats-service/configuration/clustering/jetstream_clustering),
strangler fig pattern (https://docs.aws.amazon.com/prescriptive-guidance/latest/cloud-design-patterns/strangler-fig.html).

## Storage engine implications (foundation half of the mandate)

Collecting the concrete Phase 1 design requirements that fall out of the
decisions above, so they aren't scattered:

- Durable-offset vs. committed/high-watermark offset distinction (D2)
- Truncatable uncommitted log tail (D2)
- Epoch/fencing-token field in segment/index metadata (D2)
- No Raft-specific invariants hard-coded into recovery logic (D2)
- Storage I/O built directly on `rusty_tokio`'s `OpDriver`/`UringFile`
  (`io-uring-fs` feature), with `SimDriver` as the simulated implementation —
  not a hand-rolled parallel trait (D3, D4).
- **Revised, post-spike (round 3):** the D3 round-1/round-2 finding that no
  third-party runtime's internal I/O driver was swappable for a simulated one
  (confirmed false for compio, glommio, and monoio by source inspection) is
  now moot for the chosen runtime — `rusty_tokio`'s `io-uring-fs` driver *is*
  swappable, by design, via the public `OpDriver` trait, and this was verified
  working, not just claimed. Storage-engine code should be written directly
  against `UringFile`/`OpDriver` rather than a redundant abstraction on top.

## Consumer gates

Phase 1's storage engine is not considered done until:
- The three minimal DST tests in D4 pass against both `IoUringDriver` (real)
  and `SimDriver` (simulated) — reference versions already exist and pass in
  `rusty_tokio`'s own test suite (see D3); rusty_stream's versions should
  match that shape against its actual segment format.
- The `Cargo.toml` depends on `rusty_tokio` with the `thread-per-core` and
  `io-uring-fs` features enabled, per D3's final decision — not compio.
- The primitives listed under "Storage engine implications" exist and are
  exercised by at least one test each.
- At least one pilot workload (per D5) is running on rusty_stream in a
  non-production capacity.

## Non-goals (this ADR)

- Full Kafka wire-protocol compatibility, in any form, for Phase 1 or as a
  default future direction (D1).
- Any consensus implementation (VSR or Raft) landing as Phase 1 code (D2).
- Hand-rolling a separate `Storage`/`Clock` injectable-trait abstraction now
  that `rusty_tokio`'s `OpDriver`/`SimDriver` already provides this seam —
  build on it directly instead (D3, D4).
- A per-core io_uring ring for file I/O — `rusty_tokio`'s `io-uring-fs`
  deliberately uses one process-wide ring; chasing per-core I/O parallelism
  would be optimizing for throughput this ADR's §4 already ruled out of scope
  (D3).
- Betting DST tooling on `madsim`/`turmoil` as a runtime-replacement strategy
  (D4).
- Declaring NATS JetStream deprecated or unsupported for new work (D5).

These are in addition to, not a replacement for, the Phase 1 non-goals already
recorded in `docs/phase1-scope.md` §2 (multi-broker replication, Kafka
wire-protocol compatibility layer, WASM transforms, consumer-group rebalancing).

## Alternatives considered

Covered inline per decision above (D1: full Kafka compat; D2: adopt `openraft`
now, or adopt VSR now; D3: compio, glommio, monoio — each spiked with equal
rigor and superseded by `rusty_tokio` once its two disqualifying gaps closed;
D4: `madsim`/`turmoil` as the primary DST mechanism, and a hand-rolled trait
duplicating what `rusty_tokio` already ships; D5: replace JetStream outright).
Not repeated here to avoid duplicating the reasoning sections.

## Consequences

- Client SDKs, connectors, and observability integrations must be built
  in-house against `rusty_wire` — no reuse of the Kafka client ecosystem (D1).
- Phase 2 clustering work starts with the concrete consensus protocol still
  undecided, carrying a small reevaluation cost when the forcing function
  arrives, in exchange for not building unused generality now (D2).
- The Phase 1 runtime is `rusty_tokio`, org-owned rather than a third-party
  dependency — this ADR's own repo-inspection process is now a template for
  vetting future `rusty_tokio` changes the same way, not a one-off exercise
  (D3).
- DST infrastructure (`OpDriver`/`SimDriver`-based fault injection) is
  already real, not a first-milestone task — the storage engine's initial
  commit builds on an existing, tested seam rather than creating one (D4).
- Two operational systems (rusty_stream and NATS JetStream) run in parallel
  through Phase 1 and early Phase 2, with an explicit, criteria-based
  re-evaluation gate rather than an open-ended coexistence (D5).
