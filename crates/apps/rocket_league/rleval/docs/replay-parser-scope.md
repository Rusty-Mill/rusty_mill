# Our own replay parser: scope

Status: proposed, 2026-10-24; P1 done 2026-10-24 (the bit reader moved to P2, the
first slice that reads bits). The owner chose to write a first-party `.replay` parser
instead of keeping `boxcars` (board card w7). This note sizes that work before any code
is written. ADR-0008 kept `boxcars` and `subtr-actor` as Tier T until a first-party
parser was decided; this is that decision's plan. The estimates and slices below are a
proposal: each PR is still reviewed and approved on its own, and this note does not
authorize implementation.

## What we use today

| Crate | Lines | What we take from it | Where |
|---|---|---|---|
| `boxcars` 0.11.5 (MIT) | 6.0k (attributes 1.75k, parser 0.7k, object-to-attribute table 0.6k, header 0.4k) | The whole decode: header properties, CRC, the object/name/net-cache tables, the bit-level network stream, about 44 attribute types | `replay-analyzer` (through `decode::boxcars_adapter`, 678 lines), `rb_replay_ingest`, `recon-check` |
| `subtr-actor` 1.2 (MIT) | 95k, nearly all stats we do not call | Its frame collector: per-tick ball and player rigid bodies, boost, and throttle/steer bytes, resampled to a fixed rate | `rb_replay_ingest` (`convert.rs`, 322 lines); `recon-check` as an independent oracle |

They also pull in `bitter`, `encoding_rs`, `fnv` and `phf`. `subtr-actor` 1.2.0 accepts
`boxcars >=0.11.3, <1.0`; it does not require 0.11. Our direct `boxcars = "0.11"`
dependencies and workspace lock unify both callers on 0.11.5 so they share the same
`boxcars::Replay` type. A fresh resolution can select a separate newer version for
`subtr-actor`, so this unification is a lock choice, not its upper bound (see
[`rb_replay_ingest/Cargo.toml`](../../crates/rb_replay_ingest/Cargo.toml)).

The seam already exists. `replay-analyzer`'s analyze layer depends only on the
`ReplayParser` trait and its neutral `DecodedReplay` (header metadata plus a per-frame
stream of `ActorUpdate`s), so a new parser is one more adapter. The format is written up in
[`replay-file-format.md`](replay-file-format.md), including the full attribute table
(Appendix A) and archetype catalog (Appendix C).

## What we would write

A new crate, `rl_replay`, in `crates/apps/rocket_league/crates`, Tier S: no external
normal or build dependencies. UTF-16 strings come from `std`, Windows-1252 from a
128-entry table, CRC-32 from a 256-entry table, class lookups from `match`.

| Part | Estimate (lines, excluding tests) |
|---|---|
| Bit reader, compressed ints, vectors, rotations, quaternions | 300 |
| Container, CRC, header properties | 450 |
| Object/name/net-cache tables, class hierarchy, attribute-type lookup | 700 |
| Network frame decoder: spawn, initial trajectory, updates, delete | 450 |
| Attribute decoders (the ~44 types in use; see Appendix A) | 1,300 |
| Errors (fail closed: name the unknown class or attribute, frame and bit offset) | 150 |
| **Parser total** | **about 3,350** |
| `ReplayParser` adapter replacing `boxcars_adapter` | 600 |
| `rb_replay_ingest` off `subtr-actor`: build `PhysicsFrame`s from `DecodedReplay` | 400 |
| **All new code** | **about 4,350**, plus about 1,500 of tests |

For scale, that is a bit over two thirds of `boxcars` (we skip serde output and very old
replay versions) and under a fifth of `rb_physics_bullet` (24.8k).

**Version scope.** Replays from version 868.23.8 on (engine.licensee.net). The committed
samples are 868.23.8 (`419a`), 868.27.10 (`42f2`) and 868.32.11 (the `rb_replay_ingest`
sample); the private corpus is recent ranked play. Anything older fails with a clear
"unsupported version" error rather than a guess. (This note first said net 10 or later,
which would have refused `419a`; P1 found that.)

## How we prove it

`boxcars` and `subtr-actor` stay as **oracles only**: dev-dependencies, which Tier S allows
as test oracles that never ship. A dev-dependency is available to tests, examples and
benches, not to ordinary library or binary builds, so oracle use has to move there:

- **`recon-check`** today is a library plus two binaries (`recon-check` and
  `batch-recon-check`) with `boxcars` and `subtr-actor` as normal dependencies. Its
  comparator stays in the library, typed on our model only; oracle decoding and
  reconstruction move out of the library into test/example support. The single-replay
  runner becomes `examples/recon_check.rs`, and the batch runner becomes
  `examples/recon_corpus.rs`, retaining its manifest/directory/limit options, R1 `--gate`
  exit behavior and no-op when the corpus is absent. Committed-sample tests run in CI;
  the private corpus runs locally with
  `cargo run -p recon-check --release --example recon_corpus -- ...`.
- **`corpus_check`** (an ordinary binary in `rb_replay_ingest` using its current
  `boxcars`/`subtr-actor` pipeline) becomes `examples/corpus_check.rs`, with the new
  differential mode. Preserve the directory argument and corpus-absent no-op; invoke it
  with `cargo run -p rb_replay_ingest --release --example corpus_check -- <dir>`.
- **`boxcars_adapter`** remains only in test/example support for differential decoding,
  with no oracle types exposed by ordinary libraries or binaries.
- No bounded tooling exception is proposed. If a later slice finds an oracle that cannot
  live in tests or examples, it asks for one explicitly, naming the crate and its exit.

1. **Differential decode.** For every replay we can reach, our `DecodedReplay` must equal
   the one `boxcars_adapter` builds, update for update. In CI: the three committed replays.
   On the game machine: the private corpus, through the `corpus_check` example's
   differential mode (real replays never enter the repo).
2. **Downstream unchanged.** `replay-analyzer`, `replay-viewer` and `bc-clone` goldens pass
   unmodified after the switch; `recon-check`'s tests and corpus example keep comparing
   against `subtr-actor`.
3. **`rb_replay_ingest`.** Same `PhysicsFrame`s as today within float rounding, checked by
   the existing tests and `rb-verify`'s replay mode.

## Slices

| PR | Content | Done when |
|---|---|---|
| P1 | `rl_replay`: container, CRC, header (done) | Headers of all three samples equal `boxcars`'s |
| P2 | Bit reader, tables and the network stream, all attribute types | Every frame of the samples decodes; differential test green |
| P3 | `RlReplayParser` adapter; `replay-analyzer` switches to it; its `boxcars` use moves to tests | All downstream goldens unchanged; corpus run clean on the game machine |
| P4 | `rb_replay_ingest` off `subtr-actor`; `corpus_check` and both `recon-check` runners move to examples; library oracle code moves to test/example support | Its tests and `rb-verify` replay scores unchanged; example runners retain the corpus options and gate behavior above |
| P5 | Input hints for r6: `ReplicatedThrottle`/`Steer`, `ReplicatedTorqueInput`, dodge `DodgeTorque`, jump, double-jump and dodge `ReplicatedActive`, handbrake | New `ActorUpdate` variants and per-car input hints in `PhysicsFrame` |
| P6 | Keep `boxcars` and `subtr-actor` only as dev-dependencies everywhere; record `rl_replay` as Tier S | Manifests and `cargo metadata` dependency kinds show oracle declarations only as dev dependencies; normal/build dependency paths from every workspace member reach neither oracle, including optional/target-specific paths |

For P6, validate the resolved graph with
`cargo tree --locked --workspace --all-features --target all -e normal,build` (no dev
edges), alongside manifest/metadata inspection. `Cargo.lock` records resolved packages,
not dependency kinds: both oracles may remain there for development, so their presence
or absence alone cannot prove the dev-only boundary. These are future acceptance checks;
this documentation PR changes no dependency declarations or runner commands.

P1 to P4 are the replacement; P5 is what input recovery (r6) wants from a parser. P5 does
not strictly need our own parser: `boxcars` already decodes those attributes and our
adapter just drops them. Doing it after P3 avoids writing it twice.

## Costs and risks

- **We own format changes.** A Rocket League patch can add classes or attributes; an
  unknown one desyncs the bit stream. Today the `boxcars` maintainer ships the fix; after
  P6 we do. The fail-closed error names exactly what to add, and the corpus run on the game
  machine after a patch catches it before analysis does.
- **Bit-level encodings** of vectors and quaternions changed across versions. The version
  scope above keeps us to the current ones; `boxcars` remains the reference for the bits.
- **Licence.** Both oracles are MIT. Where we follow `boxcars` closely (bit encodings, the
  attribute table) we credit it in `THIRD_PARTY_NOTICES.md`.
- **Size of the bet.** About 4,350 lines and six PRs before the parser pays for itself,
  against two working external crates. The owner chose this knowingly (w7); r6 can start
  research in parallel, since its first validation uses BakkesMod captures, not replays.
