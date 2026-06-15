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
- **Corpus reproducibility.** The 180 corpus `.replay` files live in an
  uncommitted local cache (gitignored, ~230 MB); only the distilled fixtures are
  committed. Document/script their retrieval so calibration is reproducible from
  scratch.
- **Heavier value learner.** `ValueModel` is a deliberately simple logistic model
  behind a stable interface; swap in a GBM/NN once the corpus justifies it
  (`value/src/model.rs` doc note). Held-out VAL AUC is currently ≈0.71.
