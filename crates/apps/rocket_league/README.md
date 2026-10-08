# rocket_league

One family for the Rocket League work: a physics reimplementation verified
against recorded ground truth, a replay analysis app, and (planned) a native
Rust RLBot framework. Edges between its crates are family-internal and
unconstrained (ADR-0003); reusable pieces (FlatBuffers/Protobuf) go to `libs/`.

| Dir | What |
| --- | --- |
| [`rusty_bullet/`](./rusty_bullet) | Physics port and verification pipeline: docs/ADRs, BakkesMod capture plugin, `tools/rb_tape_bot` (excluded from the workspace: own lockfile, external `rlbot` client, retired by the native RLBot port) |
| `rleval/` | Replay analyzer app (arrives in a follow-up import) |
| `crates/` | Every Cargo member of the family, flat |

Crates: `rb_domain` (pure domain + ports), `rb_physics_bullet` (Bullet3 port), `rb_env`
(stepping environment), `rb_replay_ingest` / `rb_capture_ingest` (adapters),
`rb_scenario`, `rb_verify_cli` (composition root).

## Not in git

Replays, captures and corpora (some are other players' matches). `.gitignore`
here blocks `*.replay` outside `fixtures/`; they live in the private companion
repo `baileyrd/rocket_league_private`, found through `RL_REPLAY_DIR`.

## History

Imported from `baileyrd/rusty_bullet` with `git filter-repo` + merge (not
`git subtree`; see `docs/research/ROCKET-LEAGUE-APPS-MIGRATION-PLAN.md`), so
`git log --follow` works but SHAs differ from the source repo. The old-to-new
map is in the import PR.

## Dependency notes

`rb_replay_ingest` pins `subtr-actor ~1.2` and the workspace lock keeps `boxcars`
on one 0.11.x: see the comment in that crate's `Cargo.toml` before running
`cargo update` here. Licence: MIT OR Apache-2.0 (`LICENSE-*` in this dir).
