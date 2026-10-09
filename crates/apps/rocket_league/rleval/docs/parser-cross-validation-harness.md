# Parser cross-validation harness — scope & handoff

Status: **Phase 1 + Phase 2 shipped.** The last open M3 item (spec §13, "second
parser adapter for redundancy"). **Phase 1** — the ballchasing.com header
cross-check (comparator `scoring/src/contract.rs`, `contract` bin, offline CI test
`scoring/tests/ballchasing_contract.rs` on sanitized real fixtures, key-gated fetch
shim `scripts/ballchasing_fetch.py`). **Phase 2** — the independent *reconstruction*
cross-check (the `recon-check` crate, comparing our `build_canonical` against
`subtr-actor` frame-by-frame; offline CI test on the committed `42f2`/`419a`). Both
built and green.

Capture note: uploading our `42f2`/`419a` samples was blocked by this
environment's egress **request-body cap** (`POST` bodies over ~64 KB → 413;
`GET` is fine), so the committed real pair is a corpus replay already on
ballchasing (`990e4485-…`), fetched by id. To pair the exact `42f2` sample,
relax the egress body limit and upload it privately via the shim.

## Decision (made)
This is **not** a drop-in `ReplayParser` adapter. carball/ballchasing operate above
our decode port's network-actor granularity, and the spec's contract (§4.2, §10) is
at the **canonical-match** level. So: a **canonical-vs-external comparator** that
cross-checks our boxcars-derived match facts against an independent decoder.
- **Phase 1 (this task):** ballchasing.com header cross-check.
- **Phase 2 (deferred):** carball frame-level reconstruction cross-check.

## What it validates (and what it can't)
| Layer | Example fields | External corroboration |
|---|---|---|
| Header facts | `team_scores`, per-goal scorer/team, roster/teams, `map`, `team_size` | **Exact** |
| Header per-player stats | `goals`, `assists`, `saves`, `shots`, `score` | **Partial** — goals/score exact; saves/shots/assists may differ (ballchasing recomputes) |
| Reconstructed aggregates | `boost_used`, `touches`, `possession_time_s` | Loose at best (different definitions) |
| Scoring-rubric metrics | `overcommit_rate`, `aerial_presence`, … (our IP) | **None** |

This directly guards §11 "parser drift between RL patches": if a patch shifts the
header layout and our goals/saves silently go wrong, the cross-check catches it.

## Contract surface (tiered pass criteria)
- **Tier 1 — exact, FAIL on mismatch:** goal count + each goal's `(scorer, team)`;
  per-team final scores; roster (name ↔ team); `map`; `team_size`.
- **Tier 2 — advisory, WARN don't fail:** per-player `saves`/`shots`/`assists`
  (our header `PlayerStats` vs ballchasing's recomputed). `goals`/`score` stay exact.
- **Tier 3 — deferred:** boost economy — our `boost_used` (0–100 integral, can exceed
  100) vs ballchasing `bpm`/`amount_collected` are differently defined; needs a
  mapping spike first.

## Architecture (no `ReplayParser` involvement)
1. **Comparator** (Rust, scoring crate, CI-tested): `cross_check(our: &CanonicalMatch,
   bc: &BallchasingReplay) -> CrossCheckReport` over Tier-1/2 fields, + a thin
   `scoring/src/bin/contract.rs` (two JSON paths → diff → nonzero exit on any Tier-1
   mismatch). Match players by normalized name, fall back to platform id.
2. **CI contract test** `scoring/tests/ballchasing_contract.rs`: runs the comparator
   on **checked-in fixtures** (`scoring/tests/fixtures/42f2.{canonical,ballchasing}.json`);
   asserts Tier-1 pass on the real pair **and** that a deliberately-drifted fixture is
   caught. **No network in CI.**
3. **Fetch shim** `scripts/ballchasing_fetch.py` (manual, network/ToS-gated, never in
   CI): reads `BALLCHASING_API_KEY`, uploads a replay `visibility=private`, polls,
   writes a sanitized fixture. Used only to capture/refresh fixtures.

## Security
- API key from env `BALLCHASING_API_KEY` only — never echo, never commit.
- Upload replays `visibility=private`; **sanitize** the `uploader`/steam identity out
  of committed fixtures.
- Any key shared in chat must be rotated; get a fresh one via env in the new session.

## Blocker
ballchasing.com must be in the environment's **network egress allowlist** (a prior
session hit `403 Host not in allowlist: ballchasing.com`). The egress policy applies
at container start, so an allowlist edit needs a **fresh session**. If it stays
blocked, fall back to a schema-faithful **synthetic** fixture (+ a drifted one) so the
comparator + CI test still get built, and swap a real capture in later.

## Schema reference (so the next session doesn't re-derive)
- **Our canonical:** `replay-scoring --dump-canonical can.json`. `CanonicalMatch` in
  `src/model.rs`: `team_scores: BTreeMap<i32,i32>`; `players: Vec<PlayerMeta>`
  {name, team, score, goals, assists, saves, shots — header}; `events: Vec<Event>`
  incl. `Goal{t, scorer?, team?}` (header); `map`; `team_size`. Reconstructed
  (not for cross-check): tracks/frames/resampled/features.
- **Per-player report:** `scoring/src/report.rs` `Report` (composite, sub-scores,
  licence, player_type, main_leak, `metrics[]`). The 14 rubric metrics are our IP —
  no external equivalent, never cross-check them.
- **Worker CLI:** `cargo run -p replay-scoring -- <file.replay> --json o.json
  --html o.html [--dump-canonical can.json] [--from-canonical can.json]`
  (or `target/release/replay-scoring`). Samples: `assets/replays/42f2.replay`, `419a.replay`.
- **ballchasing API (re-verify against current docs — it changes):** ping `GET /api/`
  (Authorization header) → 200 {chaser,type,name,steam_id}; upload
  `POST /api/v1/upload?visibility=private` (multipart `file`) → 201 {id} or 409 {id}
  on dup; `GET /api/v1/replays/{id}` → when `status=="ok"`: `blue`/`orange` teams,
  each `{players:[{name, id, stats:{core:{goals,assists,saves,shots,score,mvp},
  boost:{…}}}], stats}`, plus `map_code`, `team_size`. Processing is async → poll.

## Resume checklist
1. `export BALLCHASING_API_KEY=<fresh key>`; `curl -s -H "Authorization:
   $BALLCHASING_API_KEY" https://ballchasing.com/api/` → expect 200. If 403, allowlist
   `ballchasing.com` + new session (or take the synthetic fallback).
2. Capture + sanitize fixtures for `42f2` (and `419a`) via the shim; commit under
   `scoring/tests/fixtures/`.
3. Generate our canonical for the same replays (`--dump-canonical`).
4. Build the comparator + `bin/contract.rs` + the CI test; verify Tier-1 passes on real
   data and the drift case fails.
5. Update `service/README.md` tail + this repo's `docs/backlog.md` M3 section (mark the
   second-parser item done, Phase 1; note Phase 2 deferred). Small PR → `main`, merge
   after CI is green.

## Phase 2 — independent Rust reconstruction cross-check (design)

Deferred (not started). Phase 1 corroborates *header facts* against ballchasing;
Phase 2 corroborates the *frame-level reconstruction* (ball/car trajectories and
event timing) against a **second, independent** decoder — the thing nothing else
checks today.

### What gap it fills (and what already covers the rest)
Reconstruction is already validated three ways, so Phase 2 must add something new:
- `tests/external_validation.rs` + the `validate` bin — per-player **aggregates**
  (supersonic time, dist-to-ball, boost) vs the committed ballchasing ground-truth
  fixture, gated on Spearman + relative error + coverage.
- `tests/golden.rs` — pins reconstruction output to a digest (catches *regressions*,
  but it's our output vs our own past output, not correctness).
- Phase 1 — header facts vs ballchasing.

The uncovered surface is the **trajectories themselves and event timing**: an
independent decoder agreeing the ball/cars were *there* at *that* time, not just
that aggregates correlate.

### The independence constraint (the whole point)
A second reconstruction is only worth building if it is genuinely independent —
two copies of the same bug agreeing proves nothing. Rules:
- **May** depend on `boxcars` (the parse layer) and the public `CanonicalMatch`
  model (to compare against).
- **Must not** reach into `replay_analyzer::analyze::*` internals — enforce this by
  putting it in a **separate crate** so the compiler blocks accidental reuse.
- **Honest caveat:** sharing `boxcars` means parse-layer decode bugs stay invisible
  to this check (that boundary is what the ballchasing check covers, for header
  facts). Document it; don't oversell it as full end-to-end independence.

### Two ways to build it
- **Option A — reuse an existing independent Rust reconstructor** (candidate:
  `subtr-actor`, the boxcars-based actor-state reconstructor used around the
  rlgym/rlbot ecosystem). Genuine reconstruction-layer independence (different
  authors/algorithm), least code. Needs a ½-day spike to confirm it exposes
  ball + car position/velocity/boost per frame and builds offline. Adds one
  dependency. **Recommended if viable.**
- **Option B — hand-roll a minimal reconstructor** over `boxcars` (actor tracking →
  ball/car kinematics → goal/demo timing). Full control, but ~1–2 weeks plus an
  ongoing "don't share helpers" discipline tax. Lower ROI.

### What it cross-checks (tiered, with tolerances — it's float/time data)
- **Tier R1 (tight tolerance → fail):** resample both reconstructions to a common
  fixed-Hz grid (reuse `Resampled`), then ball position + each car position within
  ε; player count/identity; goal & demo event timestamps within ±~1 frame.
- **Tier R2 (statistical → warn):** per-frame velocity/boost correlation (aggregates
  are already covered by `external_validation`).
- **Touches/possession:** advisory (definitional differences).
- The real engineering is **time alignment + resampling to a shared grid** — compare
  on the grid, not raw native frames.

### CI shape — cleaner than Phase 1
No network, no key, no ToS, no fixtures to capture/sanitize. Reuse the committed
`assets/replays/42f2.replay` + `419a.replay`; decode *both* reconstructions live
in-test (the `canonical_roundtrip` test and the viewer GL-smoke job already decode
42f2, so the pattern is established) and compare with tolerances. Fully offline.

### Where it lives
A small `recon-check` crate (depends on `boxcars` + `replay-analyzer`'s public model
only) + a thin diff bin + one integration-test gate.

### Effort & priority
Option A ≈ 1–2 days incl. the spike; Option B ≈ 1–2 weeks. This competes for
priority with getting CI/Actions back online and wiring `reconcile --gate` into CI.
Its distinct value is *trajectory-level* independence; if a crate makes Option A
cheap it's worth doing, otherwise the marginal coverage over golden +
`external_validation` may not yet justify a hand-rolled reimplementation.

### Spike result (done — Option A confirmed viable)
A `subtr-actor` spike (v1.0.2) settled it:
- It pins **`boxcars 0.11.3` — our exact version**, so it shares our parse layer
  with no type conflict, and builds/runs fully offline (no network, no key).
- `NDArrayCollector` + `FrameRateDecorator::new_from_fps(30.0, …)` yields a 30 Hz
  matrix with ball position/rotation/linear+angular velocity and per-player
  position/velocity/rotation + **boost** — exactly the per-frame fields we need.
- On `42f2` it independently recovered the same **3-player roster** and **12560
  frames @ 30 Hz** (frame-for-frame with our grid; both share the clock origin
  `current_time ≈ 23 s` at kickoff, so they align by frame index).
- Ball-position agreement vs our resampled grid: **0 uu at kickoff, ~20–30 uu
  mid-match** — well under one ball radius (~93 uu), in the same coordinate frame.
  That sets a sensible Tier-R1 tolerance (~50–100 uu).

So Option A was chosen and built.

### Implemented (Option A — `recon-check` crate)
`recon-check` decodes a replay, builds **our** reconstruction via the public
`build_canonical` and the **independent** one via `subtr-actor`
(`NDArrayCollector` at our grid's Hz), aligns the two grids by nearest game-clock
time, and tiers the comparison:
- **Tier R1 (gated):** roster (player set), frame coverage (≥ 90% aligned), and
  ball-position agreement — **median ≤ 60 uu** *and* an **agree-rate ≥ 80%** of
  frames within 200 uu. The agree-rate (not p95) is the gate because goal
  celebration / reset windows are non-gameplay frames where the ball legitimately
  diverges (they blow up p95/max but not the median or the bulk agreement).
- **Tier R2 (advisory):** per-player car position (matched by name) and boost
  (boost units differ by convention — reported, never gated).

Measured on the committed replays (offline): **42f2** median 15 uu, 97% agree;
**419a** median 28 uu, 88% agree — both pass with margin, and a synthetic +600 uu
ball drift trips R1 (the reconstruction-regression tripwire). The `recon-check`
bin prints the tiered diff (non-zero exit on an R1 breach);
`recon-check/tests/recon_contract.rs` is the offline CI gate (decodes
`42f2`/`419a` and runs both reconstructions in-process — no network, no fixtures).
Caveat stands — sharing `boxcars` means this is **reconstruction-layer**
independence, not parse-layer.

## Non-goals
Not a drop-in parser; not validating the scoring rubric; no live API call in CI.
Parse-layer decode bugs (shared `boxcars`) are out of scope for Phase 2 — the
ballchasing header check (Phase 1) covers that boundary for header facts.
