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

### D3 — Runtime: compio, provisional, pending a validation spike

**Decision:** compio is the working choice for the thread-per-core/io_uring
runtime, but this is explicitly provisional — not a locked-in decision — pending
a validation spike (defined below). This is a "best of three imperfect options"
call, not a strong conviction, and the ADR says so rather than manufacturing
false certainty.

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

Sources: Iggy's thread-per-core/io_uring migration writeup
(https://iggy.apache.org/blogs/2026/02/27/thread-per-core-io_uring/), compio
(https://github.com/compio-rs/compio), glommio
(https://github.com/DataDog/glommio,
https://github.com/DataDog/glommio/issues/707), monoio
(https://github.com/bytedance/monoio), madsim
(https://github.com/madsim-rs/madsim), turmoil
(https://tokio.rs/blog/2023-01-03-announcing-turmoil).

### D4 — Testing strategy: injectable disk/clock traits from commit one, not a runtime-replacement bet

**Decision:** The storage engine is built from its first commit behind an
injectable `Storage`/`Disk` trait and a `Clock` trait. A real io_uring-backed
implementation and an in-process `SimDisk`/`SimClock` (byte-level fault
injection: torn writes, partial writes, fsync that silently fails or reorders
completions, disk-full, corruption — all seeded and replayable) ship together.
This is *not* a bet on `madsim` or `turmoil` as a runtime replacement.

**Reasoning:** rusty_stream Phase 1 is single-node — the non-determinism that
actually needs controlling is disk timing/ordering and process crashes, not
network partitions. `madsim` and `turmoil` are primarily network simulators built
by intercepting Tokio internals; their disk-simulation support is thin to
nonexistent, and neither has documented compatibility with an io_uring
thread-per-core runtime. Building a purpose-built `Disk`/`Clock` trait pair gives
full control over exactly the fault classes a storage engine needs to prove,
independent of the runtime choice in D3 — this mirrors how TigerBeetle and
comparable Rust storage projects (KayaDB, S2) actually structured their DST.

**Sequencing risk with D3, flagged explicitly:** if rusty_stream later wants
network-level DST for Phase 2 clustering (via `madsim`/`turmoil`), that pulls
toward a Tokio-based runtime, or toward an unproven wrapping of an io_uring
runtime behind Tokio's task model. The disk/clock trait-injection approach here
is runtime-agnostic and should proceed regardless of the D3 spike outcome; but if
network-level DST tooling is wanted down the line, the runtime decision needs to
be revisited with that constraint in view before Phase 2 clustering work starts.

**Minimal viable first DST tests** (write these against the storage engine before
considering it Phase-1-complete — see consumer gates):
1. **Crash during segment roll** — seeded fault kills the process mid-roll;
   recovery must never lose an offset acknowledged before the roll began, and
   must land on a single consistent active segment.
2. **Torn write on last segment** — corrupt/truncate the tail of the active
   segment mid-record; recovery must detect the incomplete record via
   length/checksum and truncate to the last valid boundary, never crash or
   silently serve corrupt data.
3. **fsync fault** — simulate fsync reporting success without persisting, or
   reordering completion versus subsequent writes; verify the durability
   boundary matches the configured fsync policy exactly under simulated
   crash-and-restart.

Sources: TigerBeetle's DST writeup
(https://tigerbeetle.com/blog/2023-07-11-we-put-a-distributed-database-in-the-browser/),
DST primer (https://www.amplifypartners.com/blog-posts/a-dst-primer-for-unit-test-maxxers),
madsim (https://github.com/madsim-rs/madsim), turmoil
(https://docs.rs/turmoil/latest/turmoil/), S2's DST writeup
(https://s2.dev/blog/dst), KayaDB (https://lib.rs/crates/kaya-io).

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
- Disk and Clock access behind injectable traits from the first commit, with a
  real and a simulated implementation shipping together (D4)
- Runtime I/O calls isolated behind a seam that could plausibly be swapped for a
  simulated driver — validate this is actually true for compio before locking in
  (D3, D4)

## Consumer gates

Phase 1's storage engine is not considered done until:
- The three minimal DST tests in D4 pass against both the real and simulated
  disk implementations.
- The D3 validation spike has run and either confirms compio or names a
  replacement with the same rigor this ADR applied.
- The primitives listed under "Storage engine implications" exist and are
  exercised by at least one test each.
- At least one pilot workload (per D5) is running on rusty_stream in a
  non-production capacity.

## Non-goals (this ADR)

- Full Kafka wire-protocol compatibility, in any form, for Phase 1 or as a
  default future direction (D1).
- Any consensus implementation (VSR or Raft) landing as Phase 1 code (D2).
- Treating compio as beyond-question final before the D3 spike runs (D3).
- Betting DST tooling on `madsim`/`turmoil` as a runtime-replacement strategy
  (D4).
- Declaring NATS JetStream deprecated or unsupported for new work (D5).

These are in addition to, not a replacement for, the Phase 1 non-goals already
recorded in `docs/phase1-scope.md` §2 (multi-broker replication, Kafka
wire-protocol compatibility layer, WASM transforms, consumer-group rebalancing).

## Alternatives considered

Covered inline per decision above (D1: full Kafka compat; D2: adopt `openraft`
now, or adopt VSR now; D3: glommio, monoio; D4: `madsim`/`turmoil` as the primary
DST mechanism; D5: replace JetStream outright). Not repeated here to avoid
duplicating the reasoning sections.

## Consequences

- Client SDKs, connectors, and observability integrations must be built
  in-house against `rusty_wire` — no reuse of the Kafka client ecosystem (D1).
- Phase 2 clustering work starts with the concrete consensus protocol still
  undecided, carrying a small reevaluation cost when the forcing function
  arrives, in exchange for not building unused generality now (D2).
- The runtime choice is not fully locked in until the D3 spike runs — Phase 1
  storage-engine work should not deeply couple to compio-specific APIs before
  that spike completes.
- DST infrastructure (disk/clock trait injection) is a first-class, non-optional
  part of the initial storage-engine commit, not a follow-up task (D4).
- Two operational systems (rusty_stream and NATS JetStream) run in parallel
  through Phase 1 and early Phase 2, with an explicit, criteria-based
  re-evaluation gate rather than an open-ended coexistence (D5).
