# Backlog

Tracking the remaining productization work against the milestones in
`replay-scoring-service-spec.md` (§13). Items are grouped by milestone, then by
priority within each. "Where" names the home a piece would live in.

## Status snapshot

Done (this Rust workspace — the parse/feature/scoring/value worker):
- **M1 — "does the score mean anything":** boxcars → canonical model → role
  assignment → full metric set → composite + 3 sub-scores → licence band →
  leak→chapter. Calibrated against a 180-replay ranked corpus; composite tracks
  rank (CV Spearman ≈ 0.79, per-rank means monotonic). ✔
- **Reconstruction validated** against ballchasing.com ground truth (supersonic
  ρ≈0.997, dist-to-ball ρ≈0.97, boost ρ≈0.98; coverage 1.0). `validate` binary +
  `external_validation` test. ✔
- **Team binding fixed** at source (network `Engine.PlayerReplicationInfo:Team`
  → team-actor archetype), recovering 92 players across 23 empty-header replays;
  calibration + value-model artifacts regenerated. ✔
- **Two-track reconciliation:** value-model ΔV used as an independent validator of
  the rubric. `reconcile` binary + `reconcile` module + tests. ✔
- **M2 report assembly (Rust slice):** lobby comparison table (§4.10), SVG
  position/touch heatmaps, self-contained HTML report; `replay-scoring --html`. ✔
- **Mechanical skill detection + verification (`replay-skills` crate):** a
  12-skill catalog (aerial, air dribble, ceiling/wall play, ground dribble,
  flick, power shot, redirect, kickoff first touch, boost steal, demo,
  supersonic) detected purely from the canonical grid/events, plus a
  verification API (`performed` / `performed_by` / `count_for` /
  `performed_in_window`) and a `replay-skills --verify <skill>` CLI gate. The
  complement to decision-discipline scoring (mechanics, not positioning).
  Synthetic per-detector tests + golden digest. Versioned `SkillConfig`.
  See `docs/skill-detection.md`. ✔

Everything below is **not started** unless noted.

---

## M2 — service & monetization (web/DB layer, Python per spec §3/§6)

This is a separate FastAPI + SQLModel + SQLite codebase, not this Rust workspace.
The Rust worker already emits the full scored `LobbyReport` (JSON) + report HTML
that this layer persists and renders.

- **API service.** FastAPI, async-first, ports-and-adapters. Endpoints from §8:
  upload/status, `GET /v1/reports/{id}`, `POST /v1/account/lock-profile`,
  `GET /v1/account/credits`. Where: new `service/` (Python).
- **Persistence.** SQLModel tables (§7): `Replay`, `Report`, `Account`,
  `LedgerEntry`, `LeaderboardEntry`, `Config`. SQLite → Postgres. Idempotency:
  `replay_id = sha256(blob)`; cache the canonical-match blob so re-scoring never
  re-parses (§9). Where: service DB layer.
- **Job queue + worker split.** Rust parse/feature worker (this repo, invoked as
  a subprocess or service) feeding a Python scoring/report worker; heavy work
  off-thread (§9). Where: service workers.
- **PDF rendering.** Feed the existing report HTML (`replay_scoring::render::html`)
  to headless Chromium or weasyprint; cache by `replay_id+player`; `GET
  /v1/reports/{id}/pdf` (§4.10, §8). The HTML is already PDF-ready; this is the
  only missing piece of §4.10. Where: report worker.
- **Credit ledger.** Soft-hold on upload → commit on success → **auto-refund on
  any post-hold failure** (§4.10, §10 property test). Where: service.
- **Entitlement webhook.** `POST /internal/webhooks/purchase` (Stripe/PayPal) →
  set `owns_book`, grant monthly credits (§8). Where: service.

## M3 — leaderboard, seasons, redundancy

- **Leaderboard + seasons + Founding-N.** Materialized best-per-account-per-season;
  only **locked-profile** reports with `confidence == ok` are eligible; recompute
  on each eligible write (§7, §8). Where: service.
- **Re-score endpoint.** `POST /internal/rescore/{id}` re-runs the pure scoring
  core at a new `score_config_version` from the cached canonical blob, no
  re-parse (§8, §9). The core is already pure; needs the cached-blob plumbing.
- **Second parser adapter** for redundancy/contract-testing: `carball` or the
  ballchasing.com API behind the existing `ReplayParser` port (§6). Where:
  `src/decode/` (this repo).

---

## Feature directions (this repo, beyond the original spec)

These extend the workspace past `replay-scoring-service-spec.md`, driven by
product direction rather than the milestone plan.

- **Flesh out individual player skills.** Take `replay-skills` past
  presence/counts to per-player *proficiency*: rate the quality and consistency
  of each mechanic (aerial height distribution, dribble duration, flick
  conversion, redirect accuracy, kickoff-win rate, demo efficiency), roll them
  into a per-player mechanical profile / rating, and tie skill instances to
  outcomes (which led to a shot or goal — reuse the value model's ΔV). Grow the
  catalog toward the harder mechanics (wave dash, half-flip, double touch) where
  a kinematic signature is separable, each with reported precision. Pairs with
  the "calibrate skill thresholds" follow-up below. Where: `skills/` (this repo).
- **3D replay simulation / viewer.** ✔ *First increment shipped* — the
  `replay-viewer` crate distills the resampled grid + events + detected skills
  into a compact `Scene` and embeds it in a self-contained three.js viewer
  (ball + cars with boost/labels, scrubable timeline, orbit camera, event/skill
  ticker). Pure scene core, golden-tested; `replay-viewer <replay> --html`. See
  `docs/replay-viewer.md`. Remaining: vendor three.js for a fully-offline file;
  tune car orientation against footage + a chase/ball cam; overlay scoring
  (roles / 1st-2nd man / leak) alongside the skill ticker; optional `--hz`
  playback downsample; a headless-GL render smoke test.

---

## Viewer improvements (`replay-viewer`)

Tracked churn-list. Items needing a product/visual decision are marked
**(needs input)** and left open; the rest are implementable directly.

**Readability**
- [x] Closer default camera + presets (overview / behind-goal / ball-cam)
- [x] Ground / drop shadows for cars + ball (depth perception)
- [x] Clearer car mesh (cabin + nose) + forward marker
- [x] More prominent boost gauge

**Navigation & UX**
- [x] Event markers on the timeline (goals/demos/kickoffs), clickable to seek
- [x] Click-to-focus / follow a player (HUD row → camera locks + row highlights); click a ticker entry to seek
- [x] Jump to next/prev goal & kickoff (n/p, k/j); loop; ◀▶ ±1s
- [x] Show/hide toggles (trails, labels, boost)
- [x] HUD polish: bottom-bar layout + SVG play/pause (emoji glyph dropped)

**Analysis overlays**
- [x] Scoring roles (1st/2nd man): gold ring + HUD tag on the 1st man (from `replay-scoring`; leak still TODO)
- [x] Skill highlight callouts on the car in 3D (fading "★ <skill>" above the performer)
- [x] Possession indicator; per-player trails
- [x] Heatmap floor projection (toggle; occupancy of the ball or followed player, binned in-viewer from the grid)
- [ ] Win-probability / ΔV strip synced to playback (from `replay-value`)

**Correctness**
- [ ] Map-aware drawn field (see the map-geometry follow-up above)
- [ ] **(needs input)** Validate car orientation (yaw/pitch/roll → forward) against footage — could add a velocity-alignment self-check
- [x] Rotation slerp between frames (smoother spin)

**Performance & portability**
- [x] `--hz` playback downsample to shrink the embedded payload (e.g. `--hz 15` ≈ halves it)
- [x] Vendor three.js for a fully-offline single file (no CDN) — `--offline` embeds three.js as `data:` URLs

**Testing**
- [x] Headless-GL render smoke test in CI (`viewer/tests/gl_smoke.mjs` + `.github/workflows/ci.yml`; renders the `--offline` file under swiftshader)

---

## Engineering follow-ups (this repo, smaller)

- **Reconciliation as a standing gate.** Add a `--gate` to `reconcile` (and a
  test) that fails on a rank-vs-impact **sign disagreement**, so a future metric
  change that tracks the ranked cohort but not in-match value is caught in CI.
  Mirrors `validate --gate`. (Currently reports 0 disagreements.)
- **Team-anchored rank join in `calibrate`.** The manifest keys ranks by
  ballchasing-mangled player names, so ~38/720 players go unranked (name
  mangling, not a team issue). Reuse the team-anchored matching from
  `analyze::validate` so every player gets its rank label.
- **Spurious `<unknown>` tracks.** A few replays coalesce an extra unnamed track
  (an unbound/short-lived PRI). Harmless downstream (excluded from validate /
  reconcile / lobby), but worth eliminating in `analyze::identity`.
- **Corpus reproducibility.** ✔ The 180 corpus `.replay` files are gitignored
  (~230 MB); only the distilled fixtures are committed. Retrieval is now scripted
  and documented: `assets/corpus/refresh_corpus_replays.py` re-downloads them by
  manifest `id` from the ballchasing API (token, rate-limit, `--limit`/`--ids`/
  `--bucket`, atomic writes), alongside `refresh_ballchasing_stats.py` for the
  ground-truth fixture; see `assets/corpus/README.md`. Token-free checkout
  samples (`assets/replays/{42f2,419a}.replay`) ship for quick spot-checks.
- **Heavier value learner.** `ValueModel` is a deliberately simple logistic model
  behind a stable interface; swap in a GBM/NN once the corpus justifies it
  (`value/src/model.rs` doc note). Held-out VAL AUC is currently ≈0.71.
- **Calibrate skill thresholds against a corpus.** `replay-skills`' `SkillConfig`
  defaults are pre-calibration geometry guesses; fit them (e.g. aerial-height,
  dribble-proximity, power-shot-speed bands) against the labeled corpus so the
  per-skill counts track reality, and validate detector precision against
  ballchasing/hand-labeled clips. Extendable to more mechanics (wave dash,
  half-flip) only if a kinematic signature proves separable.
- **Map-aware field geometry.** The map name *is* read from the replay
  (`MapName` → `CanonicalMatch.map`) but is used only as a display label — all
  geometry is the fixed standard-Soccar constants in `field.rs`, applied to every
  replay. Correct for the competitive Standard arenas (cosmetic reskins, identical
  collision), but wrong for genuinely non-standard geometry: non-standard Soccar
  arenas (Wasteland, Starbase ARC, Pillars/Octagon) and alternate modes (Hoops,
  Dropshot, Snow Day). On those the wrong dimensions degrade kickoff detection →
  attack-direction normalization, the wall/ceiling/boost-steal skills, the
  field-third / central-zone scoring, and the viewer's drawn field box + goals.
  Fix: make `field.rs` constants a `FieldGeometry` value selected by `MapName`
  (default = standard), thread it through `analyze`, `skills`, `scoring`, and
  `viewer`. Authoritative data (goals/demos/scores/boost) and raw reconstruction
  (positions/velocities) are already map-agnostic and unaffected. Note the spec
  scopes the product to standard 2v2; the analyzer neither detects nor rejects
  non-standard maps today — a cheap interim step is to flag/quarantine them.
