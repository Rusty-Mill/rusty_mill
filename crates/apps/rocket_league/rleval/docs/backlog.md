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

A separate FastAPI + SQLModel codebase under `service/` (not this Rust
workspace), shelling out to the `replay-scoring` worker. **Foundation shipped** —
see `service/README.md`. Pure-ish, ports-and-adapters, tested without the Rust
toolchain (the scorer is faked behind a `Scorer` protocol); CI runs the suite
(`.github/workflows/ci.yml`, `service` job).

- **API service.** ✔ FastAPI, ports-and-adapters. The full §7 surface:
  `POST /v1/replays` (upload→guard→hold→enqueue), `GET /v1/replays/{id}`,
  `GET /v1/reports/{id}` (+`/pdf`), `GET /v1/leaderboard`, `GET /v1/account/credits`,
  `POST /v1/account/lock-profile`, `POST /internal/webhooks/purchase`,
  `POST /internal/rescore/{id}`, `GET /healthz`. Upload guards: `owns_book` +
  positive balance + a **per-account rate limit** (§7, `RLS_UPLOAD_RATE_LIMIT`).
  Auth is a dev `X-Account-Email` stub (real auth = web layer). Where:
  `service/app/main.py`.
- **Persistence.** ✔ SQLModel tables (§5): `Account`, `CreditLedger`, `Replay`,
  `Report`, `LeaderboardEntry`; SQLite (→ Postgres via `RLS_DATABASE_URL`).
  Idempotency `replay_id = sha256(blob)`; raw blobs content-addressed on disk for
  re-scoring without re-upload; the **canonical-match** blob is optionally cached
  (gzipped) so a re-score skips the parse (§9, `RLS_CACHE_CANONICAL`). *Remaining:*
  encryption-at-rest (§12).
- **Credit ledger.** ✔ Balance = sum of non-expired deltas; soft-hold → confirm →
  **auto-refund on failure**, idempotent by `replay_id`; holds inherit the grant's
  expiry (no rollover/drift). Property covered (`test_credits`, `test_persistence`).
  Where: `service/app/credits.py`.
- **Job queue + worker split.** *Partial* — upload enqueues an in-process
  background task (the Rust parse+score worker via `SubprocessScorer`). *Remaining:*
  a real queue (Celery/RQ) so workers scale independently (§9).
- **PDF rendering.** ✔ The worker's PDF-ready report HTML is captured at score
  time (`replay-scoring --html`, stored as an artifact) and rendered to PDF behind
  a `PdfRenderer` port (weasyprint adapter, lazy/optional `pdf` extra), **cached
  per replay**; served owner-gated at `GET /v1/reports/{id}/pdf` (§4.10).
  Tested with a fake renderer (`service/tests/test_pdf.py`: render, cache-once,
  owner-gate, 409-not-ready); the real weasyprint path verified on the 42f2 report
  (75 KB HTML → 63 KB PDF). Where: `service/app/pdf.py`. *Remaining:* per-player PDF
  scoping (currently the full-lobby report).
- **Entitlement webhook.** ✔ `POST /internal/webhooks/purchase` flips `owns_book`
  and grants credits — monthly (`monthly_grant`, expires period end) or top-up
  (+10, +30 d) — idempotent by the provider's event id (`WebhookEvent`). Completes
  §8's grant side (spend/refund already shipped). Tested
  (`service/tests/test_webhook.py`: grant, idempotency, top-up, and end-to-end
  account provisioning → upload) + **`X-Signature` HMAC-SHA256 verification** when
  `RLS_WEBHOOK_SECRET` is set (idempotent, valid/invalid/missing covered). Where:
  `service/app/webhooks.py`, `main._verify_webhook_signature`.

## M3 — leaderboard, seasons, redundancy

- **Leaderboard + seasons + Founding-N.** ✔ *Core shipped* — `service/app/leaderboard.py`
  materializes the **best composite per account per season** (`YYYY-Sn` quarters);
  only **locked-profile** reports with `confidence == "ok"` are eligible, recomputed
  on each successful score; public `GET /v1/leaderboard?season=…&limit=…`. Tested
  (`service/tests/test_leaderboard.py`: eligibility gates + best-per-season + sorted
  feed). *Remaining:* Founding-N seeding and the "recent" feed; season rollover/close
  policy. Where: service.
- **Re-score endpoint.** ✔ `POST /internal/rescore/{id}` re-runs the scoring core
  at the worker's current `score_config_version` and **swaps in the fresh reports**
  (no re-upload), re-materializing the leaderboard (FK-safe) and dropping the stale
  PDF. **No-re-parse re-score ✔** — with `RLS_CACHE_CANONICAL`, score caches the
  gzipped canonical blob and re-score runs the worker's `--from-canonical` mode
  (byte-identical to a fresh parse, proven by `scoring/tests/canonical_roundtrip.rs`);
  else it re-parses. Tested (`service/tests/test_rescore.py`: replace-at-new-version,
  leaderboard refresh, PDF invalidation, canonical-vs-reparse selection, 404/409).
  Where: `service/app/service.py` (`rescore`) + `scoring/src/main.rs`
  (`--dump-canonical` / `--from-canonical`).
- **Second parser adapter** for redundancy/contract-testing: `carball` or the
  ballchasing.com API behind the existing `ReplayParser` port (§6). Where:
  `src/decode/` (this repo).

---

## Feature directions (this repo, beyond the original spec)

These extend the workspace past `replay-scoring-service-spec.md`, driven by
product direction rather than the milestone plan.

- **Flesh out individual player skills.** *Proficiency profiles ✔* +
  *goal-outcome linking ✔* + *structured per-skill quality ✔* + *ΔV value link ✔* —
  `replay-skills` has `profile::profiles` (`--profile`): per-player `skills/min` +
  per-skill `SkillStat` (count, rate, `mean_metric`, `mean_quality`);
  `outcome::outcomes` (`--outcomes`): per-player skill→goal buildup involvement;
  and `value_link::skill_values` (`--value`): each ball-contact skill credited the
  value model's per-touch **ΔV** swing (from `replay-value`'s new
  `per_touch_delta_v`), the continuous outcome the sparse goal count only
  approximates — signed `+`helped/`-`hurt, gated to `Skill::is_ball_contact` so a
  run-start isn't credited a coincidental touch. Each `SkillInstance` also carries
  a structured `metric` (primary evidence magnitude in its natural unit — aerial
  peak height uu, dribble duration s, power-shot ball speed uu/s, redirect angle
  deg, boost stolen %), so profiles report **mean aerial height / dribble
  duration** rather than only the normalized confidence proxy
  (`Skill::metric_label` / `metric_unit` self-describe it). *Remaining:* harder
  mechanics (wave dash, half-flip, double touch) where a kinematic signature is
  separable. Pairs with the "calibrate skill thresholds" follow-up. Where:
  `skills/` (this repo).
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
- [x] Win-probability / momentum strip above the timeline (P(blue scores next) from `replay-value`; model is basic, read as rough momentum)
- [x] Impact (ΔV) overlay — per-player roster impact chip + per-touch swing in the ticker + ball-contact skill callouts tinted by swing (from `replay-value`'s `per_touch_delta_v`; `--no-impact` omits)

**Correctness**
- [x] Map-aware drawn field — non-standard maps draw via `geometry_for_map` + a warning banner (Phase 1; see the map-geometry follow-up)
- [x] Car orientation verified from data — the yaw-forward (+X, rotated about Z)
  aligns with car velocity on 89% of moving samples (mean cos ≈ 0.80; the +Y
  convention scores ≈ 0). Cars drive nose-first; the mapping is correct.
- [x] Rotation slerp between frames (smoother spin)

**Performance & portability**
- [x] `--hz` playback downsample to shrink the embedded payload (e.g. `--hz 15` ≈ halves it)
- [x] Vendor three.js for a fully-offline single file (no CDN) — `--offline` embeds three.js as `data:` URLs

**Testing**
- [x] Headless-GL render smoke test in CI (`viewer/tests/gl_smoke.mjs` + `.github/workflows/ci.yml`; renders the `--offline` file under swiftshader)

---

## Engineering follow-ups (this repo, smaller)

- **Reconciliation as a standing gate.** ✔ `reconcile --gate` exits non-zero on
  any rank-vs-impact **sign disagreement** (mirrors `validate --gate`); the
  `disagreements()` logic is covered by `flags_sign_disagreement`. Note: it reads
  the corpus replays (gitignored) + `value_model.json`, so it's a local/with-corpus
  gate — not wired into the no-corpus CI job. Wiring it in CI needs the corpus
  fetch step (see corpus reproducibility). Currently 0 disagreements.
- **Team-anchored rank join in `calibrate`.** ✔ The two-pass team-anchored matcher
  (exact names, then the unique residual per team) is now a shared, tested
  primitive — `analyze::roster_match::team_anchored_pairs` — reused by both
  `analyze::validate::pair_replay` (refactored onto it, behavior unchanged) and the
  calibrate rank join. `calibrate::join_ranks` joins each scored player to their
  manifest tier through it, using the ballchasing **stats fixture** (loaded by
  replay `id`) to supply each mangled rank key's team, so a name-mangled player
  (~38/720, where ballchasing's name ≠ the true in-replay name) still resolves to
  its rank instead of being dropped. Degrades to exact-name matching if the fixture
  is absent. Unit-tested (`team_anchored_pairs` cases; `join_ranks` recovery vs.
  exact-only); the end-to-end recovered count needs the gitignored corpus to
  measure.
- **Spurious `<unknown>` tracks.** ✔ `analyze::identity::coalesce` now drops short
  unbound (`<unknown>`) tracks — a brief actor that never binds a PRI is a glitch,
  not a player (real players always bind one). Threshold `MIN_ORPHAN_SAMPLES`;
  substantial unbound tracks are kept. Unit test (`short_unbound_orphan_track_is_dropped`);
  the 42f2 golden is unaffected (it has no orphan).
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
- **Map-aware field geometry.** *Phase 1 (classify + flag) ✔* — `field.rs` now
  has geometry-as-data (`FieldGeometry`, `geometry_for_map`) and a `classify_map`
  / `is_standard_geometry` registry. The competitive Standard arenas (cosmetic
  reskins, identical collision — the common case) classify standard and are
  unaffected; recognized non-standard maps/modes (Hoops, Dropshot, Octagon,
  Pillars, Badlands, Starbase ARC) are flagged: scoring forces such reports
  **low-confidence**, the viewer shows a **"non-standard map" banner** (and draws
  via `geometry_for_map`), and both CLIs warn. Authoritative data + raw
  reconstruction were already map-agnostic.
  *Phase 2 (remaining):* `geometry_for_map` still returns standard geometry for
  every map — substituting **measured** non-standard dimensions (and threading
  `FieldGeometry` through the analyzer kickoff/normalization, skills wall/ceiling,
  and scoring thirds) needs real collision data per arena/mode, which isn't
  reliably documented. Until then, flag-not-transform is the honest behavior. The
  non-standard registry (`field::NON_STANDARD_MAPS`) is conservative; extend as
  maps are confirmed.
