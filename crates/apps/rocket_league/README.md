# rocket_league

One family for the Rocket League work: a physics reimplementation verified
against recorded ground truth, a replay analysis app, and (planned) a native
Rust RLBot framework. Edges between its crates are family-internal and
unconstrained (ADR-0003); reusable pieces (FlatBuffers/Protobuf) go to `libs/`.

| Dir | What |
| --- | --- |
| [`rusty_bullet/`](./rusty_bullet) | Physics port and verification pipeline: docs/ADRs, BakkesMod capture plugin, `tools/rb_tape_bot` (excluded from the workspace: own lockfile, external `rlbot` client, retired by the native RLBot port) |
| [`rleval/`](./rleval) | Replay analyzer app: scoring, skills, value model, 3D viewer, web UI. Also holds the Python `service/` (FastAPI/Celery) and its Docker files. The service and Docker setup were moved as-is and are **not built or tested in CI**; the Dockerfile's build context assumes the old standalone repo and needs rework before use |
| `crates/` | Every Cargo member of the family, flat |

Crates: `replay-analyzer` (canonical match model), `replay-scoring`, `replay-skills`, `replay-value`, `replay-viewer`, `replay-pacifist`, `bc-clone`, `recon-check`, `rleval-app` (the `rleval` binary); `rb_domain` (pure domain + ports), `rb_physics_bullet` (Bullet3 port), `rb_env`
(stepping environment), `rb_replay_ingest` / `rb_capture_ingest` (adapters),
`rb_scenario`, `rb_verify_cli` (composition root).

## Not in git

Replays, corpora, saved player sessions and third-party capture kits (some are other players' matches)
live in the private repo `baileyrd/rocket_league_private`. Check it out, or symlink its dirs, under
`rleval/` (`assets/{corpus,myReplays,modes,case-studies}`, `data/local`); those paths are gitignored here.
Without it, tests that need the corpus skip and everything else runs. `.gitignore` also blocks `*.replay`
outside `fixtures/` and `rleval/assets/replays/` (two small test fixtures, kept on purpose). Run the
`rleval` CLIs from `rleval/`: their default paths are relative to it.

## History

Imported from `baileyrd/rusty_bullet` and `baileyrd/RLEvalSystem` (private and personal content stripped) with `git filter-repo` + merge (not
`git subtree`; see `docs/research/ROCKET-LEAGUE-APPS-MIGRATION-PLAN.md`), so
`git log --follow` works but SHAs differ from the source repo. The old-to-new
maps are `docs/research/rl-migration/*-commit-map.txt`.

## Dependency notes

`rb_replay_ingest` and `recon-check` pin `subtr-actor ~1.2` and the workspace lock keeps `boxcars`
on one 0.11.x: see the comment in that crate's `Cargo.toml` before running
`cargo update` here. Licence: MIT OR Apache-2.0 (`LICENSE-*` in this dir).
