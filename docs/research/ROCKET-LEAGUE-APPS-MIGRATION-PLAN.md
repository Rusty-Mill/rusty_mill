# Rocket League apps family: migration plan

Status: **proposal, nothing executed.** Source repos are never modified or deleted.
Inspected 2026-10-08: `baileyrd/rusty_bullet` (public), `baileyrd/RLEvalSystem` (private), full-history fetches of both.

## 0. What is actually being moved

| Source | Shape | Moves as |
| --- | --- | --- |
| `rusty_bullet` | 7-crate Rust workspace (`rb_*`), 794 commits, 3 MiB pack, `tools/rb_tape_bot` (own Cargo package + ~30 Python analysis scripts), `bakkesmod-plugin` (C++/CMake), 85 files of docs/ADRs | history-preserving import |
| `RLEvalSystem` | **Rust-first** (9 packages: root `replay-analyzer` + `scoring`, `skills`, `value`, `viewer`, `pacifist`, `bc-clone`, `recon-check`, `app`), plus a Python FastAPI/Celery `service/`, Python `spire_capture/` + `assets/corpus/*.py` tooling, Docker/compose. 222 commits, **98 MiB pack** | history-preserving import, **after stripping replay blobs** |
| RLBot port | **does not exist yet** (no repo; roadmap says build natively here) | new crates, no import |

Not a rewrite case: RLEval's core is already Rust. Only the Python pieces stay Python; they move as non-crate directories. Nothing needs rewriting to land; the rewrites are small and mechanical (section 5).

## 1. Target layout (ADR-0003: `crates/<layer>/<family>/crates/<crate>`)

```
crates/apps/rocket_league/            # one family; edges inside it are unconstrained
  README.md  ARCHITECTURE.md          # family README lists crates whose prefix differs (ADR-0003 rule)
  crates/                             # every Cargo member, flat
    rb_domain rb_env rb_physics_bullet rb_replay_ingest rb_capture_ingest rb_scenario rb_verify_cli
    replay-analyzer replay-scoring replay-skills replay-value replay-viewer replay-pacifist
    bc-clone recon-check rleval-app
    rlbot_*                           # new, later
  rusty_bullet/                       # product dir: README, CHANGELOG, AGENTS.md, docs/, bakkesmod-plugin/, tools/rb_tape_bot/
  rleval/                             # product dir: README, docs/, service/, spire_capture/, scripts/, corpus/ (scripts+manifests only), Dockerfile, compose
  rlbot/                              # product dir: docs only until code exists
```

Decisions and why:
- **One family, `rocket_league`.** The roadmap has RLEval using the sim to recover inputs and the RLBot port driving `rb_env`; that is cross-product coupling. Inside one family it is legal. Splitting into three families would hit the ADR-0003 "nothing outside a family depends on an app crate" check on day one.
- **Per-product docs stay in per-product dirs.** `rusty_bullet` ADRs run to 0059+ and RLEval docs are flat; ADR-0001's remit rule already tolerates per-product numbering. Do not merge them into root `docs/adr/`.
- **Package names unchanged in the import** (`rb_*`, `replay-*`, `rleval-app`). Renaming touches every `use replay_analyzer::` and hides the move in the diff. If wanted, a later pure-rename PR to `rl_*`/`rleval_*`. Directory name = package name.
- **New reusable pieces are libs, not family crates.** The FlatBuffers/Protobuf crates the roadmap wants go under `crates/libs/protocol/` (`rusty_flatbuffers`, `rusty_protobuf`); the RLBot port depends on them as workspace deps. Only the RLBot-specific crates (`rlbot_core`, `rlbot_server`, ...) live in the family.
- **CI lane is automatic.** `ci_components.py` keys lanes on `crates/apps/<family>`, so the whole family becomes one `apps--rocket_league` lane.

## 2. History-preserving move: `git filter-repo`, then merge (not bare `git subtree`)

Reason for deviating from ADR-0001's `git subtree`: bare subtree would import RLEvalSystem's history verbatim, and that history holds **1,128 raw `.replay` blob versions (111 MiB)** plus a 12.8 MB `canonical-match.json`. `.gitattributes` declares `assets/myReplays/*.replay` as LFS, but the 76 committed files are real blobs, not pointers; the 1,055 `assets/corpus/*.replay` files are LFS pointers. Importing as-is would add ~100 MiB to rusty_mill forever. Replays must stay out of git, so they must be stripped from the imported history.

Procedure per repo (all in a fresh clone under the scratchpad; sources untouched):

1. `git clone --mirror <src> work.git`, then a working clone of it. Tag the source tip (`pre-migration-<sha>`) in the **clone only**.
2. `git filter-repo` with, for RLEval: `--invert-paths --path assets/myReplays --path-glob 'assets/corpus/*.replay' --path-glob 'assets/modes/*.replay' --path-glob 'assets/case-studies/*/*.replay' --path canonical-match.json --path replay-analyzer-main.rs --path replay-analyzer-Cargo.toml --path .serena` (the last three are stray root files; confirm before dropping, see open questions). Then `--path-rename` into the final layout (section 1), e.g. `scoring/:crates/apps/rocket_league/crates/replay-scoring/`, `src/:crates/.../replay-analyzer/src/`, `service/:.../rleval/service/`. Root `Cargo.toml`, `Cargo.lock`, `.github/`, `.gitignore`, `.gitattributes`, LICENSE files are dropped from history (they are rebuilt in section 4), not renamed.
3. For rusty_bullet: no big blobs (largest is a 3 MB jsonl), so only `--path-rename`. Keep `crates/rb_physics_bullet/assets/*.cmf` (188 KB collision meshes `include_bytes!`'d by `arena.rs`; they are code assets, not replays) and the 860 KB `subtr-actor-sample.replay` test fixture (see open question 2).
4. Verify the rewritten repo before touching rusty_mill: `git log --follow` on a few files, commit count equals source (filter-repo only drops commits that become empty; report any), `git count-objects -vH` shows no blob over ~2 MB.
5. In rusty_mill, on a branch: `git remote add rb-import ../work` / `git fetch` / `git merge --allow-unrelated-histories --no-ff rb-import/main -m "Import rusty_bullet into crates/apps/rocket_league"`. Because paths are already final, this is a plain merge and `git log` per file works. One PR per source repo (ADR-0001), RLEval after rusty_bullet.
6. SHAs change versus the source repos. Record the old-to-new map (`.git/filter-repo/commit-map`) in the PR body and in the family README so issue/PR references in docs remain resolvable.

## 3. Replay assets (must not enter git)

- Final home: out of tree, resolved by one env var, `RL_REPLAY_DIR`, default `$XDG_DATA_HOME/rusty_mill/rocket_league/replays` (Windows: `%LOCALAPPDATA%`). Subdirs mirror today's names: `myReplays/`, `corpus/`, `modes/`, `case-studies/`.
- In git: only manifests and fixtures. `corpus/manifest.json`, `fitted_*.json`, `rank_norms.json`, `ballchasing_*.json` and the refresh scripts stay (small); `refresh_corpus_replays.py` becomes the way to repopulate `RL_REPLAY_DIR` from ballchasing (`BC_TOKEN`, `.env.example` moves with it, `.env` gitignored).
- Add to rusty_mill `.gitignore` and enforce in CI: a check that fails on any tracked `*.replay` over 1 MB or any tracked `*.replay` outside `**/fixtures/**`. Drop the Git LFS rules entirely (rusty_mill has no LFS config and CI does not fetch LFS).
- Code that reads replays by repo-relative path must switch to `RL_REPLAY_DIR`. RLEval hits (from grep): `tests/external_validation.rs` (6), `app/src/main.rs` (5), `app/tests/panels_api.rs` (5), `viewer/src/render.rs`, `src/bin/validate.rs`, `scoring/src/bin/xg_fit.rs`, `recon-check/src/bin/batch_recon_check.rs`, `bc-clone/tests/clone.rs`, `app/tests/history_flow.rs`, `tests/real_replay_guards.rs`, `tests/golden.rs`, `tests/authoritative_parity_419a.rs`, `skills/*`, `value/src/bin/train_corpus.rs`, `scripts/retrain_value_model.{sh,ps1}`, `.github` viewer smoke (`assets/replays/42f2.replay`). Tests that need a corpus replay must skip with a clear message when `RL_REPLAY_DIR` is unset (never fail CI for a missing private asset).
- Needs a decision (open question 2): `assets/replays/` (2.8 MB plain test fixtures, used by CI's viewer smoke) and rusty_bullet's 860 KB fixture are small and already plain git. Default plan keeps tracked fixtures **under 1 MB each** as `crates/*/fixtures/`, and moves anything larger to `RL_REPLAY_DIR`.

## 4. Workspace and CI changes (rusty_mill root)

Root `Cargo.toml`:
- Add 16 members (7 `rb_*` + 9 RLEval) as `crates/apps/rocket_league/crates/<name>`. `cargo_toml_diff.py` treats pure member additions as non-global, so no forced full sweep.
- `exclude` `crates/apps/rocket_league/rusty_bullet/tools/rb_tape_bot`: it is a standalone package with its own `Cargo.lock` and the external `rlbot = "0.6.0"` dependency. Without `exclude`, cargo errors because it sits under the workspace root. It is retired when the native RLBot port replaces it (roadmap item 1), which also removes the external dep (ADR-0002).
- Each moved manifest gets `[package.metadata.rusty_mill] layer = "apps"` (required by `check_workspace_layers.py`). Added in a post-import commit, not in rewritten history.
- `rusty_bullet` crates use `version.workspace/edition.workspace/license.workspace/repository.workspace/publish.workspace` and `[lints]`. rusty_mill has its own `[workspace.package]` (line 328) and `[workspace.lints]` (line 902); the crates would silently inherit those. Diff the two blocks first; if values differ (license, repository URL, `unsafe_code = "deny"`, clippy `unwrap_used` etc.), pin the differing fields explicitly in the manifests.
- `rleval-app` depends on `rusty_multimodal_db_engine`, `rusty_oauth`, `rusty_http`, `rusty_tls` via `git = ".../rusty_mill", rev = ...`. **This is exactly the shadowing `check_workspace_deps.py` rejects (ADR-0002).** Rewrite to `{ workspace = true }`; the four entries already exist in `[workspace.dependencies]`.
- RLEval's root package + `path = ".."` sub-crate deps become `path = "../replay-analyzer"`. RLEval's `[workspace]`/`[package]` root, `build.rs`, `tests/`, `src/` all move under `crates/replay-analyzer/`.
- Hoist external deps already shared (`serde`, `serde_json`) or add `boxcars`, `subtr-actor` to `[workspace.dependencies]`. Both repos want `boxcars 0.11`; `subtr-actor` is `1.0` in RLEval and `1.2` in rusty_bullet, so cargo will unify on 1.2 (check `recon-check`, whose purpose is an independent cross-check, still passes).
- `Cargo.lock`: do not hand-merge. Delete nothing; run `cargo update -w` style resolve once and review the lockfile diff for version bumps to existing deps (the `lockfile_diff.py` planner will report impact).
- `rust-toolchain.toml` (`stable`) is dropped; the monorepo pins `1.98.1` via `RUST_TOOLCHAIN`. Both codebases must pass `clippy -D warnings` on 1.98.1 first.

CI (`.github/workflows/ci.yml` and scripts):
- Source repos' own `ci.yml` files are not carried over (Actions reads only the root `.github/workflows/`). Port what the monorepo lacks:
  - `viewer-gl-smoke` (puppeteer, software WebGL, needs a fixture replay): new job gated to `rocket_league` path changes, using a tracked small fixture, not `RL_REPLAY_DIR`.
  - `rleval-app` with `--features mmdb,oidc`: add to the existing per-feature pattern, as REMIND_ME_PACKAGES does, and exclude from the generic `--all-features` job if it conflicts.
  - Python `service/` pytest (3.11) and `spire_capture` pytest: new path-filtered job.
  - `git diff --check`: already covered by `.gitattributes` handling.
- `bakkesmod-plugin` (C++/CMake, Windows-only plugin SDK): no CI job; document it as build-by-hand, same as today (not built in rusty_bullet CI).
- Add the tracked-replay-size guard from section 3.
- Update root `docs/WORKSPACE-MAP.md` (regenerate with `generate_workspace_map.py`, CI verifies it) and the README/ARCHITECTURE family tables.
- Dockerfile/compose: `COPY . .` of the whole monorepo is unacceptable. Rebuild context to `crates/apps/rocket_league/` plus the workspace root files the build needs, or keep the image build out of CI initially (open question 4).
- Dependabot, issue/PR templates, CODE_OF_CONDUCT, SECURITY: the monorepo's win; source copies are dropped. Fold any rusty_bullet-specific PR checklist items into the family README.

## 5. Move vs rewrite

| Item | Action |
| --- | --- |
| All `rb_*` and `replay-*` crate sources, tests, goldens | **Move** unchanged |
| Crate `Cargo.toml`s | **Rewrite (mechanical):** layer metadata, `workspace = true` for rusty_mill deps, inheritance diffs, `path = ".."` fixes |
| `rb_verify_cli` `CARGO_MANIFEST_DIR`-relative paths (`../../tools/rb_tape_bot/...`, `../rb_replay_ingest/fixtures`) in `lib.rs`, `scenario.rs`, `golden_captures.rs` | **Rewrite:** depth changes (tools moves to `../../rusty_bullet/tools/...`); better, one `rb_paths` helper than six literals (only if it has two call sites, it has more) |
| Replay-path reads in RLEval (list in section 3) | **Rewrite** to `RL_REPLAY_DIR` + skip-if-absent |
| `service/` (FastAPI, Celery, alembic), `spire_capture/`, `assets/corpus/*.py` | **Move, stay Python.** Update `RLS_WORKER_BIN` build path and Dockerfile context. Python is out of the Rust sovereignty tiers but should still have its own pinned `pyproject` and be excluded from Rust CI lanes |
| `bakkesmod-plugin/` | **Move** unchanged |
| `tools/rb_tape_bot` | **Move**, workspace-`exclude`d, retire with the native RLBot port |
| Docs (85 files in rusty_bullet reference `crates/rb_*`, `tools/...`, `docs/...`) | **Rewrite links** with a scripted rewrite + `lychee`/link-check; ADR text stays as written |
| `AGENTS.md` (rusty_bullet) | Move into `rusty_bullet/`; monorepo `CLAUDE.md`/CONTRIBUTING win on conflict |
| RLBot port | **New work**, separate plan; do not start in the import PRs |

## 6. Phases (one PR each)

0. ADR-0008 "Rocket League apps family" accepted; open questions below answered. Docs-only PR.
1. Dry run on throwaway clones: filter-repo both, build/test the combined workspace locally in a scratch clone of rusty_mill. No push.
2. Import `rusty_bullet` + manifest/CI changes. Green CI is the gate.
3. Import `RLEvalSystem` (stripped) + `RL_REPLAY_DIR` switch + Python CI jobs.
4. Docs/link pass, WORKSPACE-MAP, family README.
5. Later: native RLBot port crates and libs; retire `rb_tape_bot`; optional `rl_*` rename.

## 7. Risks

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Replay blobs imported with history | +100 MiB permanent, un-removable without a force-push rewrite of rusty_mill | filter-repo strip + object-size check **before** merge (section 2 step 4); size guard in CI |
| `rev`-pinned git deps on rusty_mill from `rleval-app` | `check_workspace_deps` fails; two copies diverge | switch to workspace deps in the import PR |
| Silent inheritance of rusty_mill `[workspace.package]`/`lints` | license/lint drift | diff blocks, pin explicitly |
| Toolchain jump (`stable` to pinned 1.98.1) | clippy `-D warnings` failures on first CI run | run clippy on 1.98.1 in dry run |
| Lockfile merge bumps shared deps (`subtr-actor` 1.0 to 1.2, others) | golden/parity tests change numerically (physics parity is the whole point of rusty_bullet) | run all golden tests in dry run; compare before/after on the pre-migration tag |
| Hard-coded relative paths (`../../tools/...`) and asset paths | test failures after move | section 5 rewrites; grep gate in dry run |
| Private repo history in a (public?) monorepo | RLEvalSystem is **private**; rusty_mill visibility must permit it; history may contain tokens or personal player data (`case-studies`, `myReplays` are the owner's own matches, `data/local/*.json` 75 files) | confirm rusty_mill is private or accept exposure; run secret scan and review `data/local` before import; strip if personal |
| Docker/compose `COPY . .` against monorepo root | huge context, slow or broken image | rebuild context; defer image from CI |
| Windows-only pieces (BakkesMod plugin, `.ps1` scripts) | untested in Linux CI | document; keep `.ps1` CRLF per root `.gitattributes` |
| SHAs change | broken references in docs/issues | commit-map in PR body; links use paths |
| CI cost: one big `apps--rocket_league` lane | slower PRs | acceptable; planner still scopes by affected packages |
| External deps (`boxcars`, `subtr-actor`, `rlbot`) vs the sovereignty direction | ADR-0002 tier classification required | classify as Tier T (replace later) with owner; `rlbot` leaves with `rb_tape_bot`; own replay parser is a later, separate decision |

## 8. Rollback

Because sources are never touched, rollback is cheap:
- **Before merge:** delete the scratch clones and the PR branch. Nothing changed anywhere.
- **After merge to main:** `git revert -m 1 <import-merge-sha>` for the PR (one revert per import; the later PRs revert first). History of the other crates is unaffected. The source repos remain the source of truth and keep working (they were never archived or altered).
- **If a replay blob leaks in anyway:** do not fix forward; revert the merge before others branch from it, redo the filter, re-merge. A post-hoc history rewrite on rusty_mill is the expensive case, which is why the size gate runs before the merge.
- The source repos stay un-archived and un-deleted until at least one release cycle after phase 4, then archiving (not deleting) is a separate decision for you.

## 9. Decisions (2026-10-08) and what they change

1. **rusty_mill is public.** RLEvalSystem is private, so everything imported becomes public. Section 10 applies.
2. **Small fixtures stay in git (<1 MB each); the corpus goes to a separate private repo** (section 10). `RL_REPLAY_DIR` can point at a checkout of it.
3. `canonical-match.json` and the stray root copies are dropped from history.
4. Docker/Celery `service/` is **move-only**: no CI job, no image build in CI.
5. Package names unchanged at import.
6. RLEval may depend on `rb_env`; ADR-0008 records the edge.

## 10. Public-repo consequences (new, from a read-only scan of RLEval history)

No credentials found (no tokens/keys in any revision; `client_secret` hits in `app/src/oidc.rs` are identifiers). But the following become public on import and need your call before any push:

| Path | Content | Proposed |
| --- | --- | --- |
| `assets/myReplays`, `assets/corpus/*.replay`, `assets/modes`, `assets/case-studies/*/*.replay` | your matches and other players' replays | strip; private repo |
| `data/local/*.json` (75 files) | saved sessions with real player names/gamertags and per-player scores | strip; private repo |
| `assets/case-studies/*/manifest.json` | named third-party players (`3am-4usion`, `marthy-mcgray`) | private repo |
| `spire_capture/`, `docs/competitor/spire/` | kit and notes for capturing and reverse engineering a commercial service's premium trial (mitm of its traffic) | private repo (ToS/legal exposure if public) |
| `docs/ballchasing-*teardown*`, `bc-clone` | reverse-engineered clone of ballchasing's analyzer, validated against its API | your call: keep code, review the teardown docs for ToS-sensitive content |
| commit author | `David Bailey <baileyrd@gmail.com>` (already the rusty_mill author) | no action |
| licence | RLEvalSystem has no LICENSE file; rusty_bullet is MIT OR Apache-2.0 | add family licence before import |

Private repo (name to choose, e.g. `rocket_league_private`): created by you or with your approval. It receives the stripped paths **with history preserved** via a second filter-repo pass (`--path` selecting just those paths), so nothing is lost by keeping them out of rusty_mill. rusty_mill refers to it only through `RL_REPLAY_DIR` and a README pointer.

Revised order: phase 0 (ADR + licence + private repo exists) -> dry run -> rusty_bullet import -> RLEval import (only after you review the section 10 table).

## 11. Dry-run results (2026-10-08, scratch clones only, nothing pushed)

Reproduce with `rl-migration/filter.sh` (history rewrite) and `rl-migration/apply_manifests.py` (post-import manifest edits). Toolchain 1.98.1, as CI.

| Check | Result |
| --- | --- |
| rusty_bullet rewrite | 793 of 794 commits (one became empty), 4.9 MiB pack |
| RLEvalSystem public set | 201 of 222 commits, **3.9 MiB pack (was 98 MiB)**; no `myReplays`/`corpus`/`modes`/`case-studies` content in any revision |
| RLEvalSystem private set | 59 commits, 94 MiB, history preserved; ready for the private repo |
| Merge into rusty_mill | no conflicts; `git log --follow` crosses the import for both products |
| `check_workspace_deps.py`, `check_workspace_layers.py` | pass (after the git-dep rewrite and layer metadata) |
| `Cargo.lock` | +308 lines, **no changes to existing entries** |
| `cargo test` on the 16 packages | 1,058 pass; remaining failures all explained below |
| `fmt --check` / `clippy --all-targets -D warnings` (1.98.1) | 37 fmt diffs (fixed by `cargo fmt`); 4 clippy errors, all `chunks_exact_to_as_chunks` in `rleval-app/src/sha256.rs`; clean after; `--features mmdb,oidc` also clean |
| `generate_workspace_map.py --verify` | fails until `docs/WORKSPACE-MAP.md` is regenerated (expected) |

Corrections to the plan found by the dry run:
1. **Lockfile pins are required.** Fresh resolution picks `subtr-actor 1.4.0`, which fails to compile `rb_replay_ingest`. Pin `subtr-actor 1.2.0` (rusty_bullet's lock). Pin `boxcars 0.11.3` (RLEval's lock), otherwise `replay-analyzer`'s golden fails: the digest embeds `parser_version`. Both pins keep every golden unchanged; remove them only as a deliberate upgrade PR.
2. **Lints and repository were not safe to inherit**, as predicted: rusty_mill's `[workspace.lints]` is rustils's (`unsafe_code = "warn"`, `undocumented_unsafe_blocks`), and `[workspace.package].repository` points at `rusty_search`. `apply_manifests.py` inlines rusty_bullet's lints and the correct repository into the `rb_*` manifests.
3. **Path-depth rewrites** touched 19 `.rs` files (tests and bins using `CARGO_MANIFEST_DIR`-relative paths): `../assets/replays` -> `../../rleval/assets/replays`, and `tools/rb_tape_bot` -> `rusty_bullet/tools/rb_tape_bot`. Crate-local `include_bytes!("../assets/soccar/*.cmf")` is unaffected.
4. **Two tests need the private corpus.** `history_flow` asserts `composite.is_some()`, which needs `assets/corpus/rank_norms.json`. It passes with the corpus present and fails without. Required change: skip when `RL_ASSETS_DIR` has no corpus. `external_validation.rs` already degrades gracefully.
5. **Two fixtures exceed 1 MB** (`419a.replay` 1.5 MB, `42f2.replay` 1.3 MB). They are used by most golden tests and the viewer smoke job. Plan keeps them (still ~2.8 MB in total); say so if you want a hard 1 MB ceiling.
6. **Dangling references after the private split:** 9 broken relative links in `rleval/` (teardown docs) and 7 in `rusty_bullet/` (`CONTRIBUTING.md`, `SECURITY.md`, LICENSE paths), plus 6 files citing `docs/competitor/spire/`. Fix by link rewrite (or deleting the dead links); the family needs its own `LICENSE-*` links and a README pointer to the private repo.
7. **`panels_api::a_signed_upload...` flaked once** under 16 parallel test binaries and passed 3/3 alone. It opens a local TCP server, so it is a CI risk independent of the migration; the monorepo's nextest timeout config applies.
8. **Duplication spotted, not acted on:** `rleval-app/src/sha256.rs` is a hand-rolled SHA-256 inside a workspace that has its own crypto crates. Candidate for a `dedupe-loop` pass after the import; not in scope here.

Not done by this session: creating `baileyrd/rocket_league_private`. The GitHub integration returned `403 Resource not accessible by integration` on repository creation, so the private repo must be created by you (empty, private); then push `rlpriv` to it.

Still undecided: `bc-clone` and its teardown docs. You said everything identified stays private, but `rleval-app` has a path dependency on `bc-clone`, so moving the crate out breaks the app. The dry run kept the crate public and moved only the teardown docs; decide whether `bc-clone` should be feature-gated or its dependency removed.
