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
  `Report`, `WebhookEvent`, `LeaderboardEntry`. **Postgres-ready**: SQLite for dev,
  Postgres via `RLS_DATABASE_URL` (connect-args are dialect-aware — `check_same_thread`
  only for SQLite — + `pool_pre_ping`), schema by **Alembic** (`migrations/`, initial
  baseline) rather than create-all in prod (`RLS_DB_AUTO_CREATE=0`). Verified on a
  real Postgres 16: `alembic upgrade head` + `check` (no drift) + a full
  credit+scoring flow through the service's engine; an in-CI `alembic check` guards
  model/migration drift. Idempotency `replay_id = sha256(blob)`; artifacts
  content-addressed for re-scoring without re-upload; the **canonical-match** blob is
  optionally cached (gzipped) so a re-score skips the parse (§9, `RLS_CACHE_CANONICAL`).
- **Blob store (§5).** ✔ A `BlobStore` port (`service/app/blobs.py`):
  `FilesystemBlobStore` (default, `blob_dir`) or `S3BlobStore`
  (`RLS_BLOB_BACKEND=s3`; AWS or MinIO/localstack via `RLS_S3_ENDPOINT_URL`) so API
  and workers don't share a local disk. The backend moves opaque `(key, bytes)`;
  cipher/gzip/dedupe sit above it, so encryption-at-rest works identically on
  either. Tested with moto (real boto3, faked S3): per-artifact round-trips, prefix,
  sealed-on-S3, and a full upload→score pipeline landing artifacts in the bucket.
  *Remaining (prod):* the bucket + IAM standup.
- **Encryption-at-rest (§12).** ✔ Every stored artifact is written through a
  pluggable cipher (`service/app/cipher.py`): `NullCipher` by default, stdlib AEAD
  (HMAC-SHA256 CTR + encrypt-then-MAC, HKDF-separated subkeys) when
  `RLS_ENCRYPTION_KEY` is set. Transparent to the worker (it only sees decrypted
  bytes); verified end-to-end against the real worker on a sealed store.
  **Prod ciphers ✔** — `RLS_CIPHER_BACKEND` selects `hmac` (default) / `aesgcm`
  (AES-256-GCM, `crypto` extra) / `kms` (per-blob data key wrapped by AWS KMS).
  `unseal` dispatches by blob MAGIC so a backend migration keeps old blobs
  readable; `RLS_CIPHER_KEY_VERSION` folds into key derivation (version "0" is
  byte-compatible with legacy blobs; a bump is a hard rotation). moto-tested.
  *Remaining (prod, infra):* provision the KMS key + IAM grant.
- **Credit ledger.** ✔ Balance = sum of non-expired deltas; soft-hold → confirm →
  **auto-refund on failure**, idempotent by `replay_id`; holds inherit the grant's
  expiry (no rollover/drift). Property covered (`test_credits`, `test_persistence`).
  Where: `service/app/credits.py`.
- **Job queue + worker split.** ✔ Upload enqueues scoring through a `JobQueue`
  port (`service/app/queue.py`); `BackgroundTaskQueue` runs it in-process,
  `CeleryJobQueue` (+ `celery_app.py`) hands the `replay_id` to Redis for dedicated
  workers (`RLS_QUEUE_BACKEND=celery`). The payload is just the `replay_id` (worker
  reloads from DB + blob store) and the `Scorer` is built worker-side, so nothing
  app-specific crosses the broker. Covered by an eager-mode test (CI) **and**
  verified live end-to-end (real Redis + Celery worker + Rust scorer:
  `queued→scoring→done`, credit confirmed). `docker compose up` runs the stack
  (multi-stage image bundles the Rust binary). *Remaining (prod):* Postgres +
  object storage for true multi-node (the demo shares a SQLite/blob volume).
- **PDF rendering.** ✔ The worker's PDF-ready report HTML is captured at score
  time (`replay-scoring --html`, stored as an artifact) and rendered to PDF behind
  a `PdfRenderer` port (weasyprint adapter, lazy/optional `pdf` extra), **cached
  per replay**; served owner-gated at `GET /v1/reports/{id}/pdf` (§4.10).
  Tested with a fake renderer (`service/tests/test_pdf.py`: render, cache-once,
  owner-gate, 409-not-ready); the real weasyprint path verified on the 42f2 report
  (75 KB HTML → 63 KB PDF). Where: `service/app/pdf.py`. **Per-player scoping ✔** —
  `render::html_for_player` (scoring) renders just one player's card + the lobby
  comparison table (their column marked); `replay-scoring --html` honours
  `--player`. `GET /v1/reports/{id}/pdf?player=<id>` renders that player on demand
  from the cached canonical (`Scorer.render_player_html`), 404/409/503 on
  unknown-player / no-canonical / render-fail; the full-lobby PDF stays cached.
- **Observability.** ✔ A stdlib metrics registry (`service/app/metrics.py`,
  no `prometheus_client`) at `GET /metrics` in Prometheus text format: upload
  outcomes, scoring results, **per-stage scoring timings** (parse+score / persist /
  leaderboard via `Histogram.time()`), and HTTP RED (count + latency by route
  template, pure-ASGI middleware recording at response-start so background scoring
  isn't disturbed). `Histogram.time()` also emits structured stage logs. Exposition
  validated against a real Prometheus parser. *Remaining (prod):* a tracing/metrics
  backend (OTel) and per-worker process labels.
- **Entitlement webhook.** ✔ `POST /internal/webhooks/purchase` flips `owns_book`
  and grants credits — monthly (`monthly_grant`, expires period end) or top-up
  (+10, +30 d) — idempotent by the provider's event id (`WebhookEvent`). Completes
  §8's grant side (spend/refund already shipped). Tested
  (`service/tests/test_webhook.py`: grant, idempotency, top-up, and end-to-end
  account provisioning → upload) + **`X-Signature` HMAC-SHA256 verification** when
  `RLS_WEBHOOK_SECRET` is set (idempotent, valid/invalid/missing covered). Where:
  `service/app/webhooks.py`, `main._verify_webhook_signature`.

## M3 — leaderboard, seasons, redundancy

- **Leaderboard + seasons + Founding-N.** ✔ `service/app/leaderboard.py`
  materializes the **best composite per account per season** (`YYYY-Sn` quarters);
  only **locked-profile** reports with `confidence == "ok"` are eligible, recomputed
  on each successful score. **Founding-N** ✔ — the first `RLS_FOUNDING_N` accounts to
  qualify get an immutable ordinal (`Account.founding_number`, migration `0002`),
  surfaced in the feed. **Recent feed** ✔ (`GET /v1/leaderboard/recent`, newest
  personal bests) and **season close** ✔ (`season_bounds`/`is_closed`; the feed
  carries a `closed` flag, closed seasons never take new entries since materialize
  only writes the current season; `GET /v1/leaderboard/seasons`). Tested
  (`service/tests/test_leaderboard.py`: eligibility gates, best-per-season, sorted
  feed, founding order+cap+idempotence, recent ordering, season bounds/closed).
  *Remaining:* none material. Where: service.
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
- **Second parser adapter** for redundancy/contract-testing. **Phase 1 ✔** —
  *not* a drop-in `ReplayParser` adapter (carball/ballchasing operate above our
  decode port's network-actor granularity; the spec's contract §4.2/§10 lives at
  the **canonical-match** level). Instead a **canonical-vs-external comparator**:
  `replay_scoring::contract::cross_check(&CanonicalMatch, &BallchasingReplay)`
  tiers the cross-check — **Tier 1 (exact, fail):** goal count + each goal's
  `(scorer, team)`, per-team score, roster (name ↔ team), `map`, `team_size`;
  **Tier 2 (advisory, warn):** per-player saves/shots/assists (ballchasing
  recomputes); **Tier 3 (deferred):** boost economy (different definitions). A
  thin `contract` bin diffs two JSON files and exits non-zero on any Tier-1
  mismatch; the offline CI test `scoring/tests/ballchasing_contract.rs` asserts a
  Tier-1 pass on a real ranked pair and that a deliberately-drifted fixture is
  caught — the §11 parser-drift tripwire. Fixtures captured (private upload was
  blocked by the egress request-body cap, so a corpus replay already on
  ballchasing was fetched by id) + sanitized (uploader + player ids stripped) via
  the manual, key-gated `scripts/ballchasing_fetch.py` (never in CI). The 14
  scoring-rubric metrics are our IP and never cross-checked. Where:
  `scoring/src/contract.rs`, `scoring/src/bin/contract.rs`, `scripts/`.
  **Phase 2 ✔** — the independent *reconstruction* cross-check: the `recon-check`
  crate compares our `build_canonical` against **`subtr-actor`** (a separately
  authored Rust reconstructor pinning our exact boxcars) frame-by-frame on a shared
  30 Hz grid. Tier R1 gates roster + coverage + ball-position agreement (median
  ≤ 60 uu and ≥ 80% of frames within 200 uu — the agree-rate gate tolerates
  goal-celebration windows); Tier R2 reports per-player car position + boost.
  Measured: 42f2 median 15 uu / 97% agree, 419a 28 uu / 88% agree; a synthetic ball
  drift trips R1. Offline CI test `recon-check/tests/recon_contract.rs` (decodes
  `42f2`/`419a` and runs both reconstructions in-process — no network). This is
  *reconstruction-layer* independence (boxcars is the deliberately-shared parse
  layer; Phase 1 covers that boundary for header facts). Where: `recon-check/`.
  **Corpus-wide run ✔** — the `batch-recon-check` bin runs the cross-check across
  the whole manifest and aggregates (R1 pass/fail, ball-median + agree-rate
  distributions, offender list); `--gate` exits non-zero on any R1 fail. It's a
  local/with-corpus gate (corpus replays gitignored — `refresh_corpus_replays.py`),
  gating only replays that actually ran, so a corpus-less checkout is a no-op. Run
  in `--release`. **Full 180-replay run ✔** — investigated the 35 R1 "failures" and
  found none were reconstruction bugs, just harness strictness: (a) the roster check
  compared the *header* player list, empty for empty-header replays — now matches on
  the network-derived **tracks** (the names were always correct there); (b) the ball
  agree-rate counted **post-goal reset frames** (celebration/replay/kickoff, where the
  ball is non-gameplay and the two reconstructions legitimately diverge) — now ball
  stats are taken over gameplay frames only, excluding `[goal, next kickoff)` windows
  (no thresholds loosened). After both: **R1 PASS 180/180**, ball median 18.7 uu,
  agree ≥99.2%. The gameplay-divergence tripwire is preserved (unit-tested).
- **Ballchasing stat parity** *(in progress)* — close the gap between our 5-field
  `PlayerFeatures` and ballchasing's full boost/movement/positioning surface. Full
  audit + parity matrix + plan in `docs/ballchasing-parity.md`. Mostly an
  aggregate-and-expose problem:
  - **(1) `BallchasingStats` reducer ✔** — `analyze::bcstats::ballchasing_stats`
    reduces the existing 30 Hz grid + tracks + events into a ballchasing-shaped
    per-player block: boost (avg/collected/used/bpm/bcpm, zero/full, 0–25…75–100
    quartiles), movement (avg speed, total distance, slow/boost/supersonic,
    ground/low-air/high-air), positioning (dist-to-ball, dist-to-mates,
    thirds/halves, behind/in-front of ball — in the attack frame), and demos.
    Pure consumer (no canonical-model change); synthetic + invariant tests
    (`tests/bcstats.rs`); surfaced in the analyzer summary + `--bc-stats <json>`.
  - **(2) boost-pad pickup model ✔** — `analyze::boost_pads` attributes every
    boost-gauge gain (above a jitter floor) to the nearest of the 6 big / 28 small
    pads (`field::BIG/SMALL_BOOST_PADS`), tags stolen (opponent-half), computes
    overfill, and derives jitter-free collected/used via the conservation budget;
    folded into `BcBoost`. `tests/boost_pads.rs`; validated vs ballchasing ground
    truth (bpm/bcpm correlate, ours ~10–15% low — honest inferred-model undercount).
    *Remaining:* per-pad spatial heatmaps + the timestamped `PadPickup` stream
    feeding the viewer pickup-map overlay.
  - **(3) per-team Y-ordering ✔** — most-back/forward, goals-against-while-last-
    defender, and possession-split distance-to-ball in `analyze::bcstats` (per-frame
    per-team Y-ordering + a goal→back-most join). `tests/bcstats.rs`; validated on a
    corpus replay. `time_ball_in_side` ✔ (team/ball stat, denormalized per player).
  - **(4) cross-check against ground truth ✔** — two committed corpus gates in
    `tests/external_validation.rs`: `bcstats_agrees_with_ballchasing` (bpm/bcpm/
    supersonic/dist) and `bcstats_full_agrees_with_ballchasing` (the **whole
    35-channel block** vs an enriched, sanitized fixture `ballchasing_bcstats.json`
    from `refresh_bcstats_fixture.py`). Across 96 players the pad model is near-exact
    (count_collected_big rel 0.000, collected ρ 0.997, stolen ρ 0.985), thirds/halves
    ρ≈0.999, speed buckets ρ 0.98–0.997. Gate: every channel ρ≥0.50, strong core
    ρ≥0.90/rel≤0.12. A single-fixture `contract.rs` per-field tiered band is an
    optional remaining nicety.

---

## Feature directions (this repo, beyond the original spec)

These extend the workspace past `replay-scoring-service-spec.md`, driven by
product direction rather than the milestone plan.

- **Unified application + UI.** ✔ The `app` crate ships the `rleval` binary — a
  **single unified application** that replaces juggling the separate
  `replay-analyzer` / `replay-scoring` / `replay-skills` / `replay-value` /
  `replay-viewer` executables. `rleval serve` runs a tiny dependency-free
  (std-only `TcpListener`) HTTP server hosting a vanilla single-page UI: drop a
  `.replay` (or pick a bundled sample) and one in-process `pipeline::analyze`
  runs decode → score → skills → value → 3D scene, returning everything as one
  bundle. The UI lays it out across tabs — **Overview** (per-player composite +
  skills/min + impact ΔV), **3D Viewer** (the offline viewer doc in an iframe),
  **Scoring** (the lobby report doc), **Skills** (per-player counts), and
  **Impact** (per-player ΔV). `rleval analyze <replay> --out bundle.html` writes
  the same UI as a static, self-contained file (analysis JSON inlined, no
  server). Each engine runs at its default config — identical results to the
  standalone CLIs (which stay for scripting/golden tests). Offline pipeline smoke
  test + helper unit tests; fmt/clippy clean. Where: `app/` (this repo).
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
  (`Skill::metric_label` / `metric_unit` self-describe it). *Double touch ✔* —
  `detect::double_touches` (13th skill, `skcfg-v2`): two same-player contacts in
  quick succession with the ball elevated (pop-and-strike); kinematically
  separable, so unlike the input-only mechanics it earns a detector. *Remaining:*
  **wave dash / half-flip** are input-only (replays carry motion, not controller
  inputs) — only worth a best-effort kinematic heuristic if the false-positive
  risk is acceptable (**needs a product call**). Pairs with the "calibrate skill
  thresholds" follow-up. Where: `skills/` (this repo).
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
- [x] Consolidate settings onto a single settings panel — a ⚙ button (and `g`
  key) opens a centered Settings panel holding the show/hide toggles
  (trails + trail-len, labels, boost, team-colour, pads, heatmap, pad-heatmap,
  player-stats) and the field-overlay controls; backdrop/✕/Esc close it. Camera
  presets stay in `#left`; the play/scrub transport + speed/loop stay on the bar.
  Pure `render.rs` reorg (`#setOverlay`/`toggleSettings`); ids preserved.

**Ballchasing-parity adds** (see `docs/ballchasing-parity.md` §4 and the 3D-viewer
look-&-feel comparison against ballchasing's own viewer in
`docs/viewer-vs-ballchasing-video.md`)
- [x] Better car models — a rounded procedural hull (`carHullGeometry`): an Octane-ish side silhouette (sloped nose, cabin hump, rear deck) extruded across the width with bevelled edges, + a glass canopy; kept procedural so the offline file stays self-contained
- [x] Boost amount above each car in 3D — a **refilling boost pill** over each car (rounded track that fills left-to-right with the level, green/amber/red, value inside; `makeBoostPill`/`setBoostPill`), tied to the `boost` toggle; replaced the earlier vertical bar + numeric sprite
- [x] Roster boost fill bar (side panel) — each roster row's `.bz` pill fills with the boost level (matches ballchasing's side gauge), alongside speed / role / G·A·Sv / impact
- [x] Game-clock countdown (click the clock to toggle an RL-style 5:00 countdown vs elapsed/total; `gameClock`/`playedBy`), kickoff 3-2-1 indicator (`updateKickoff` — a digit pops over the ball during each kickoff window), top-down camera preset (`top` button + `t` key; `TOPDOWN` pose), and settings parity — one-colour-per-team toggle (`team colour`/`applyCarColours`) + trail-duration slider (`trailSecs`); the split hide-names/hide-boost ballchasing has were already our `labels`/`boost` toggles — see `docs/viewer-vs-ballchasing-video.md`
- [x] Score reflects playback time — `updateScore(t)` tallies `goal` events with `t <= playhead` instead of the final `team_scores`
- [x] Boost-pad pickup map — the 6 big / 28 small pads flash when collected, tinted by team (`Scene.pad_pickups` → `updatePads`/`padByKey`; `pads` toggle)
- [x] "Pressure" / ball-side timeline strip — which half the ball sits in over the match (ball y in team 0's attack frame; blue pressing above / orange below), next to the momentum strip (`drawPressure`)
- [x] Per-player analysis strips — Settings → "player stats" adds, per roster
  player, a boost-over-match sparkline (from per-frame `SceneCar.boost`), a
  speed-bucket bar (slow/boost/supersonic), a thirds-occupancy bar, and a
  most-back tag (`playerstats::attach_player_stats` → `Scene.player_stats` from
  `analyze::bcstats`; `buildPlayerStats` in `render.rs`)
- [x] Per-pad boost heatmap — Settings → "pad heatmap" colours each pad by how
  often it was collected over the match (`padHeatCounts`/`paintPadHeat`, reusing
  the `heatColor` ramp; aggregates `Scene.pad_pickups`)

**Analysis overlays**
- [x] Scoring roles (1st/2nd man): gold ring + HUD tag on the 1st man (from `replay-scoring`; leak still TODO)
- [x] Skill highlight callouts on the car in 3D (fading "★ <skill>" above the performer)
- [x] Possession indicator; per-player trails
- [x] Heatmap floor projection (toggle; occupancy of the ball or followed player, binned in-viewer from the grid)
- [x] Win-probability / momentum strip above the timeline (P(blue scores next) from `replay-value`; model is basic, read as rough momentum)
- [x] Impact (ΔV) overlay — per-player roster impact chip + per-touch swing in the ticker + ball-contact skill callouts tinted by swing (from `replay-value`'s `per_touch_delta_v`; `--no-impact` omits)
- [x] **Distance tool** — a `↔ distance` panel (key `m`): pick any two anchors
  (a player, the ball, or a team's goal) for a live-updating 3D line + label and
  a side-panel readout (uu), tracking playback/scrubbing; add several at once,
  each with its own colour and a remove button (`render.rs`'s `ANCHORS` /
  `updateMeasurements`). A same-team player pair is additionally checked against
  `replay_scoring`'s calibrated `support_spacing` band (`Scene::support_spacing_band`,
  attached in `viewer::roles::attach_roles` — reads the live `ScoreConfig`, so a
  rubric recalibration updates the hint for free) — green in-band, amber
  too-close (double-commit) / too-far (unreachable). This is the first
  "measure → coaching cue" bridge in the viewer: a step toward the tool
  suggesting improvements, not just displaying numbers. *Remaining:* pick anchors by
  clicking in 3D (currently dropdown-only), and a calibrated band for ball/goal
  distances (none exists in the rubric yet, unlike teammate spacing).

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
- **Heavier value learner.** *Spike ✔* — `value/src/gbt.rs` adds a dependency-free,
  deterministic gradient-boosted-trees model (second-order log-loss boosting) behind
  the same `predict(&[f32; N_FEATURES])` shape as the logistic `ValueModel`, and
  `train_corpus` now prints a logistic-vs-GBT head-to-head on the same replay-level
  held-out split. On the 180-replay corpus the GBT **clearly wins**: VAL AUC
  0.7147 → **0.7406** (+0.026), VAL log-loss 0.2844 → **0.2738** (no leakage; 92.5k
  train / 21.7k val rows). *Production swap ✔* — the ΔV evaluators are generic over a
  `Predict` trait and the loaders/storage go through a tagged `ValuePredictor` enum
  (`{"kind":"gbt"|"logistic", …}`); `value_model.json` now ships the GBT, while the
  per-match `evaluate` stays logistic (one replay is too few rows to boost). Consumers
  (`eval`, `skills`, `reconcile`, viewer impact/winprob) updated. The **production GBT
  is more regularized** than the library default (rounds 100, min_leaf 250, λ 3) —
  same held-out AUC, but a smoother per-touch ΔV that keeps `reconcile --gate` at 0
  sign-disagreements (the default GBT flipped the near-zero `first_touch_value` and
  tripped it). Re-validated: all crate tests, `skills --value`, and `reconcile --gate`
  (exit 0) on the corpus.
- **Calibrate skill thresholds against a corpus.** *Harness ✔* — `calibrate-skills`
  (`skills/src/bin/calibrate_skills.rs` + pure `skills/src/calibrate.rs`) detects
  across the ranked-2v2 corpus and reports, per skill, the metric distribution
  (p10/p50/p90) and **Spearman(per-player mean metric, rank)** (the detector-
  precision check), then writes a fitted `SkillConfig` to
  `assets/corpus/fitted_skill_config.json` (`replay-skills --config`). **Candidate-
  mode ✔** — `candidate_metrics` (`skills/src/lib.rs`) emits each detector's metric
  *pre-gate* (holding the context gates, dropping only the magnitude floor), so the
  fit sees the sub-threshold tail and can place **floors**, not just ramp tops. Now
  covers every skill with a magnitude/duration floor — the touch metrics **aerial**
  (car height), **flick** (up-velocity of carried balls), **power-shot** (goalward
  speed), **redirect** (turn angle), and the **duration-gated runs** **supersonic** /
  **ceiling** / **ground-dribble** (each detector's run loop factored into a `*_runs`
  extractor shared by the gated detector and the candidate twin, so every run's
  duration — short ones included — is emitted pre-gate). Each floor ←
  `fit_floor_valley`, the valley between weak attempts and the real mechanic within a
  per-skill band, **self-guarding** (no clear gap ⇒ keep the hand-set default, so the
  continuous metrics only move on a real separation — e.g. on the sample, dribble
  duration fits 0.75→1.03s while supersonic/ceiling hold). Aerials also fit
  `high_aerial_height` ← candidate p99.5 (other ramps use fixed offsets, so they're
  floor-only). The report prints a `└ candidates` line per skill, including "none
  cleared the gate" when a floor admits nothing. See `docs/skill-calibration.md`.
  *Remaining:* the real fit needs the gitignored corpus + `BALLCHASING_API_KEY`.
  Harder mechanics (wave dash, half-flip) stay out — input-only, no kinematic
  signature (double-touch shipped; see the skills feature-direction item).
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
