# Parser cross-validation harness — scope & handoff

Status: **Phase 1 shipped.** The last open M3 item (spec §13, "second parser
adapter for redundancy"). The ballchasing.com header cross-check — comparator
(`scoring/src/contract.rs`), `contract` bin, offline CI test
(`scoring/tests/ballchasing_contract.rs`) on sanitized real fixtures, and the
key-gated fetch shim (`scripts/ballchasing_fetch.py`) — is built and green.
**Phase 2 (carball reconstruction cross-check) remains deferred.**

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

## Non-goals
Not a drop-in parser; not validating the scoring rubric; no live API call in CI;
Phase 2 (carball reconstruction cross-check) is out of scope here.
