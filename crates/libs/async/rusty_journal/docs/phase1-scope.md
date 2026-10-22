# rusty_stream — Phase 1 Scope & Research Brief

Status: pre-RFC research pass. No implementation until the open questions below are resolved.
Fits RustyMill next to `rusty_wire` (wire protocol) and `rusty_tokio` (async runtime tooling).

## 1. Motivation

Came out of the "how should data move between distributed apps/pipelines" thread. Rather than
adopt NATS JetStream wholesale, this scopes a from-scratch Rust durable log — consistent with
the rest of RustyMill (`rusty_libc`, `rusty_tls`, `rusty_h2`) — but scoped tightly enough that
it doesn't become a five-year detour before it's useful.

**Explicit goal per your ask:** don't just re-derive Kafka. Section 4 below is the "where do we
actually improve" analysis — read it before writing any code.

## 2. Phase 1 scope (single node, no replication)

- **Wire protocol** — extend `rusty_wire`, don't start a new one
- **Storage engine** — append-only segment log + sparse offset index (Kafka's `.log`/`.index`
  model is well-proven; see §5.2). WAL-durable, fsync policy configurable.
- **Retention** — size/time-based segment rolling and deletion, no compaction yet
- **Consumers** — offset tracking per consumer, single-node only; no consumer-group rebalancing
  protocol yet (that's a Phase 2+ problem once there's a second real consumer needing it)
- **Client SDK** — Rust first; defer Python bindings until FastAPI side actually needs one
- **Runtime** — thread-per-core candidate; do not pick blind, see §5.5 (Iggy already ran this
  bake-off, use their findings as a starting point rather than repeating the work)

### Explicitly out of scope for Phase 1

- Multi-broker replication / clustering
- Kafka wire-protocol compatibility layer
- WASM-based stream transforms (Fluvio's SmartModules pattern)
- Consumer group rebalancing

These are Phase 2+ candidates, gated on an actual forcing function (a second enclave needing
HA, a real consumer-group workload) — not built speculatively.

## 3. Prior art landscape (as of mid-2026)

| Project | Language | Model | Notes |
|---|---|---|---|
| Apache Kafka | Java/Scala | segment log, replicated | the reference design; heavy (JVM), KRaft removed ZooKeeper |
| Redpanda | C++/Seastar | Kafka-wire-compatible | thread-per-core, Raft replication, no JVM |
| Apache Iggy (Incubating) | Rust | own protocol | thread-per-core, io_uring, VSR-based clustering (in progress), transparent benchmarks |
| Fluvio (InfinyOn) | Rust | own protocol + WASM transforms | SmartModules for in-stream compute, cloud-native focus |
| Blink (Cleafy) | Rust | Kafka-wire-compatible | memory-first, single-node-simple, drop-in Kafka replacement |
| RobustMQ | Rust | multi-protocol (MQTT/NATS/Kafka) | openraft-based, positions itself as protocol-unifying rather than perf-maximizing |
| NATS JetStream | Go | own protocol | what I recommended earlier as the pragmatic default — still worth benchmarking against |

## 4. Where to actually improve, not just re-derive

A critical (and fair) outside take on Iggy is worth reading before committing to a direction:
it's technically excellent (zero-copy, io_uring, honest benchmarking) but architecturally it's
"Kafka's data model reimplemented in Rust" — no fundamental rethink, and a near-nonexistent
ecosystem as a result. That's the trap to avoid repeating for its own sake.

Candidate real differentiators, given your actual constraints (none of Fluvio/Iggy/Redpanda/
Blink target these):

- **Sovereignty-first by design, not retrofit** — single-enclave, air-gapped-deployable,
  auditable at the binary level, no external dependency fetch at runtime. This is a genuine gap:
  every project above assumes cloud or at least internet-reachable infra as the default case.
- **Correctness-first testing from day one** — deterministic simulation testing (§5.6) baked in
  from the first storage-engine commit, not retrofitted after a data-loss bug. This is the
  single biggest lever FoundationDB/TigerBeetle credit for their reliability, and it's cheap to
  adopt early, expensive to bolt on later.
- **Consensus choice made deliberately** — Iggy picked VSR for its upcoming clustering; most
  Rust projects reflexively reach for Raft via `openraft` because it's the famous one. Decide
  based on your actual failure model (§5.4), don't default.

Where NOT to try to differentiate: raw throughput/latency records. Iggy and Redpanda already
compete hard on that axis with real engineering teams behind them; you won't out-benchmark them
and it isn't your actual constraint (a single governed enclave, not a trading floor).

> **Note:** an earlier pass of this brief also proposed a "governance-native data contracts"
> differentiator (baking topic/schema ownership into the broker's admin plane). That's out of
> scope for the time being and has been dropped from this document; revisit only if it comes
> back up deliberately.

## 5. Required reading before implementation starts

### 5.1 Foundational
- Kreps, *The Log: What every software engineer should know about real-time data's unifying
  abstraction* — https://engineering.linkedin.com/distributed-systems/log-what-every-software-engineer-should-know-about-real-time-datas-unifying
- Kreps et al., *Kafka: a Distributed Messaging System for Log Processing* (original paper) —
  https://www.semanticscholar.org/paper/Kafka-:-a-Distributed-Messaging-System-for-Log-Kreps/9f948448e7a5f0cc94cd53656410face8b31b18a

### 5.2 Storage engine / log internals
- Kafka segment/index/retention deep dive — https://strimzi.io/blog/2021/12/17/kafka-segment-retention/
- Kafka storage internals walkthrough — https://rohithsankepally.github.io/Kafka-Storage-Internals/
- Kafka storage internals (segments, rolling, retention) — https://www.geeksforgeeks.org/apache-kafka/deep-dive-into-apache-kafka-storage-internals-segments-rolling-and-retention/

### 5.3 Rust-native prior art — read the code, not just the pitch
- Apache Iggy — https://github.com/apache/iggy · docs https://iggy.apache.org/
- Iggy's own thread-per-core/io_uring migration writeup (compio vs glommio vs monoio, why they
  switched) — https://iggy.apache.org/blogs/2026/02/27/thread-per-core-io_uring/
- Iggy origin post (motivation, design tradeoffs) — https://blog.iggy.rs/posts/building-message-streaming-in-rust/
- Outside critique of Iggy's architectural novelty (RobustMQ's writeup — read skeptically, but
  the point stands) — https://robustmq.com/en/Blogs/34
- Fluvio — https://github.com/infinyon/fluvio · docs https://www.fluvio.io/docs/
- Blink (Kafka-wire-compatible, memory-first, single-node) — https://github.com/cleafy/blink
- Redpanda's own Kafka-log explainer (useful even though it's C++/Seastar, not Rust) —
  https://www.redpanda.com/guides/kafka-performance-kafka-logs

### 5.4 Consensus & replication (read now so the storage layer doesn't box you in for Phase 2)
- `openraft` — https://github.com/databendlabs/openraft (used by RobustMQ, Databend, Walrus,
  Hiqlite — worth seeing how differently each of those applies it)
- `raft-rs` (TiKV) — https://github.com/tikv/raft-rs
- Liskov & Cowling, *Viewstamped Replication Revisited* — https://pmg.csail.mit.edu/papers/vr-revisited.pdf
  (what Iggy is building its clustering on — read this before defaulting to Raft)

### 5.5 Thread-per-core / io_uring runtimes
- Glommio — https://github.com/DataDog/glommio · intro — https://www.datadoghq.com/blog/engineering/introducing-glommio/
- compio (what Iggy settled on) — https://github.com/compio-rs/compio
- monoio — https://github.com/ukernel/monoio

### 5.6 Correctness & testing strategy
- Deterministic Simulation Testing primer — https://www.amplifypartners.com/blog-posts/a-dst-primer-for-unit-test-maxxers
- Antithesis, *What DST is and when to use it* — https://antithesis.com/docs/resources/deterministic_simulation_testing/
- Curated DST resource list (includes `madsim`, the Rust-native DST framework used by RisingWave) —
  https://github.com/ivanyu/awesome-deterministic-simulation-testing
- TigerBeetle's DST writeup (the VOPR) — https://tigerbeetle.com/blog/2023-07-11-we-put-a-distributed-database-in-the-browser/
- Accessible intro to why DST matters — https://notes.eatonphil.com/2024-08-20-deterministic-simulation-testing.html

### 5.7 For comparison against the NATS JetStream default
- JetStream architecture — https://docs.nats.io/nats-concepts/jetstream
- JetStream streams reference — https://docs.nats.io/nats-concepts/jetstream/streams

## 6. Open questions to resolve before drafting an ADR

1. Kafka wire-protocol compatibility: adopt it (Blink's approach — instant ecosystem) or skip it
   (Iggy's approach — clean slate, no ecosystem)? This is a one-way door once client tooling
   exists — decide deliberately.
2. VSR or Raft for Phase 2 clustering — or defer the decision entirely and keep the storage
   engine consensus-agnostic until there's a concrete second-enclave need?
3. Does this replace the NATS JetStream recommendation outright, or run alongside it until
   `rusty_stream` is proven — i.e., what's the actual migration/coexistence path?
4. Runtime choice — validate compio against your workload rather than inheriting Iggy's choice
   uncritically; their bake-off criteria (raw throughput at massive scale) may not match yours
   (governed, single-enclave, moderate throughput).

## 7. Suggested next step

Once the reading above is done and the four questions in §6 have answers, draft an ADR sized
like `rustils`' RFC v2 — dual mandate (understanding + foundation), consumer gates, explicit
non-goals — before any code lands.
