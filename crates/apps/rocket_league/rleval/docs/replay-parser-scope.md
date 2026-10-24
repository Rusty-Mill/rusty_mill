# Our own replay parser: scope

Status: proposed, 2026-10-24. The owner chose to write a first-party `.replay` parser
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

They also pull in `bitter`, `encoding_rs`, `fnv` and `phf`, and they pin each other:
`subtr-actor ~1.2` needs `boxcars 0.11`, which is why the lock holds `boxcars` back.

The seam already exists. `replay-analyzer`'s analyze layer depends only on the
`ReplayParser` trait and its neutral `DecodedReplay` (header metadata plus a per-frame
stream of `ActorUpdate`s), so a new parser is one more adapter. The format is written up in
[`replay-file-format.md`](replay-file-format.md), including the full attribute table
(Appendix A) and archetype catalog (Appendix C).

## What we would write

A new crate, `rl_replay`, in `crates/apps/rocket_league/crates`, Tier S: no external
normal dependencies. UTF-16 strings come from `std`, Windows-1252 from a 128-entry table,
CRC-32 from a 256-entry table, class lookups from `match`.

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

**Version scope.** Replays with net version 10 or later (the committed samples are engine
868, licensee 27, net 10; the private corpus is recent ranked play). Anything older fails
with a clear "unsupported version" error rather than a guess.

## How we prove it

`boxcars` and `subtr-actor` stay as **oracles only**: dev-dependencies, which Tier S allows
as test oracles that never ship. A dev-dependency is visible to `tests/`, `examples/` and
benches, not to a crate's library or `[[bin]]` targets, so oracle use has to move there:

- **`recon-check`** today is a library plus a bin with `boxcars` and `subtr-actor` as normal
  dependencies. Its comparator stays in the library, typed on our model only; the
  `subtr-actor` side moves into its `tests/` (the committed samples, in CI) and an
  `examples/recon_corpus.rs` for the private corpus (`cargo run --example`).
- **`corpus_check`** (a `[[bin]]` in `rb_replay_ingest` that runs `boxcars` and
  `subtr-actor`) becomes `examples/corpus_check.rs`, with the new differential mode.
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
| P1 | `rl_replay`: bit reader, container, CRC, header | Headers of all three samples equal `boxcars`'s |
| P2 | Tables and the network stream, all attribute types | Every frame of the samples decodes; differential test green |
| P3 | `RlReplayParser` adapter; `replay-analyzer` switches to it; its `boxcars` use moves to tests | All downstream goldens unchanged; corpus run clean on the game machine |
| P4 | `rb_replay_ingest` off `subtr-actor`; `corpus_check` and `recon-check`'s oracle side move to tests and examples | Its tests and `rb-verify` replay scores unchanged |
| P5 | Input hints for r6: `ReplicatedThrottle`/`Steer`, `ReplicatedTorqueInput`, dodge `DodgeTorque`, jump, double-jump and dodge `ReplicatedActive`, handbrake | New `ActorUpdate` variants and per-car input hints in `PhysicsFrame` |
| P6 | Drop `boxcars` and `subtr-actor` from normal dependencies everywhere; record `rl_replay` as Tier S | `cargo tree -e normal` (no dev edges) on every workspace member shows neither crate, checked from manifests or `cargo metadata` dependency kinds; `Cargo.lock` cannot show this, it does not record kinds |

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
