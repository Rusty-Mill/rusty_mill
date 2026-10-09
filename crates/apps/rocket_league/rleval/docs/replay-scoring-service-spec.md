# Replay-Scoring Service — Design Spec

> A "Pacifist Score"-style service: ingest a Rocket League 2v2 `.replay`, return a composite
> decision-discipline score (0–100) with `1st_man` / `2nd_man` / `general` sub-scores, a player-type,
> a single highest-impact "leak", a book-chapter deep-link, heatmaps, and a PDF.
> Target stack: FastAPI + SQLModel + SQLite (→ Postgres), async-first, ports-and-adapters,
> with a Rust parse/feature worker. Config-driven, versioned, reproducible.

---

## 0. The uncomfortable truth up front (read this first)

Three constraints shape every downstream decision:

1. **Replays contain kinematics, not intent.** You get ~30 Hz actor snapshots — car/ball positions, velocities, rotations, boost amounts, jump/dodge/demo flags, goals. You do **not** get controller inputs. Every "decision" judgment (overcommit, panic challenge, good support) is **inferred** from motion. Treat scores as heuristic, defensible-by-rule, **not** ground truth.
2. **Scores are only as good as their calibration.** Raw metrics (e.g. "mean teammate spacing = 1400 uu") mean nothing until mapped to 0–100 against a reference distribution. That requires a **labeled corpus** (replays bucketed by rank, or coach-labeled). No corpus → arbitrary numbers. This is the real project risk, not the code.
3. **Reproducibility is non-negotiable.** A score must be recomputable. Pin `parser_version` + `score_config_version` into every report; same inputs + same versions ⇒ identical output.

Framing the product as **"decision discipline, not mechanics"** (as the reference does) is also a smart *engineering* hedge: it scopes you to positional/rotational signals, which are exactly what kinematics support well, and away from input-dependent mechanical judgments you can't see.

---

## 1. Scope

**In scope:** standard **2v2** replays only. Single-replay analysis. Per-player report for the requesting account's player (identified by platform ID / chosen in UI). Leaderboard write. Credit decrement.

**Out of scope (v1):** 1v1/3v3, multi-replay trend aggregation, live/streamed analysis, mechanical skill rating, coaching text generation beyond templated leak→chapter mapping.

---

## 2. System context

```
            ┌─────────────┐   upload .replay    ┌──────────────────┐
   client ──┤  FastAPI     ├────────────────────┤  object store    │
            │  (api)       │   enqueue job       │  (raw replays)   │
            └──────┬───────┘                     └──────────────────┘
                   │ job msg (replay_id)
                   ▼
            ┌──────────────┐   parse + features   ┌──────────────────┐
            │  queue       ├──────────────────────┤  Rust worker     │
            │ (NATS/Redis) │                       │  (boxcars)       │
            └──────┬───────┘                       └────────┬─────────┘
                   │                                        │ canonical match JSON
                   ▼                                        ▼
            ┌──────────────────────────────────────────────────────────┐
            │  scoring worker (Python): roles → metrics → scores →       │
            │  classifier → leak → chapter map → report + heatmaps + PDF │
            └───────────────────────────┬──────────────────────────────┘
                                         ▼
                          ┌──────────────────────────┐
                          │  DB (SQLModel/SQLite)     │
                          │  reports, scores, ledger, │
                          │  leaderboard, configs      │
                          └──────────────────────────┘
```

Entitlement (owns_book) and credit checks happen at the **API** before a job is enqueued.

---

## 3. Architecture principles

- **Ports & adapters.** The parser is an interface (`ReplayParser` port). v1 adapter = Rust `boxcars`. Alternative adapters: `carball` (Python, richer event detection, slower) or the **ballchasing.com** API (offload parsing entirely, network dependency, ToS check). Swappable without touching scoring.
- **Pure scoring core.** The scoring engine is a pure function `(CanonicalMatch, ScoreConfig) -> Report`. No I/O, no DB, no clock. This makes it unit-testable with golden files and trivially re-runnable for re-scoring.
- **Two worker roles.** Parse/feature extraction is CPU-bound and lives in Rust. Scoring/classification/report assembly is Python (fast enough on extracted features, easier to iterate the rubric).
- **Config over code.** Weights, target bands, calibration curves, licence thresholds, and leak→chapter maps live in a versioned `ScoreConfig` (YAML in repo, hash-pinned), never hard-coded.

---

## 4. Pipeline stages

### 4.1 Ingest & validation (API, sync)
- Accept upload; cap size (~10–50 MB). Verify magic/header, then a cheap structural parse to confirm it's a valid replay and **playlist == 2v2 / 4 players**.
- Reject early (refund nothing — no credit taken yet): wrong playlist, corrupt, >N players.
- Store raw blob keyed by content hash (`sha256`) → dedupe identical uploads.
- Reserve a credit (soft hold). Enqueue `{replay_id, account_id, target_player_hint}`.

### 4.2 Parse adapter → Canonical Match (Rust worker)
`boxcars` decodes the network stream. Emit a **canonical, parser-agnostic** JSON so the scoring core never sees boxcars types:

```jsonc
{
  "replay_id": "…", "parser_version": "boxcars-0.x",
  "playlist": "ranked-doubles", "team_size": 2,
  "fps": 30.0, "num_frames": 9012, "duration_s": 300.4,
  "players": [{"id":"steam:…","name":"…","team":0,"is_target":true}, …],
  "teams": [{"id":0,"score":3},{"id":1,"score":2}],
  "frames": [
    {"t": 12.33,
     "ball": {"p":[x,y,z], "v":[vx,vy,vz]},
     "cars": {
        "steam:…": {"p":[x,y,z],"v":[…],"rot":[pitch,yaw,roll],
                    "boost":48,"supersonic":false,"on_ground":true,
                    "jumped":false,"dodged":false,"demolished":false}
     }},
    …
  ],
  "events": [
    {"type":"goal","t":…,"scorer":"…","team":0},
    {"type":"demo","t":…,"attacker":"…","victim":"…"},
    {"type":"touch","t":…,"player":"…"}   // derive if parser lacks it
  ]
}
```

Notes:
- **Resample to a fixed grid** (e.g. 30 Hz) by interpolation so every frame has all actors; raw replays are delta/keyframe encoded. Do this in the worker, once.
- **Touches**: if the parser doesn't emit them, derive from ball-velocity discontinuities + nearest car proximity.
- Field constants: standard arena half-length ~5120 uu, width ~4096, ceiling ~2044, goal at ±5120 on Y. Encode in `field.py`/`field.rs` constants.

### 4.3 Feature extraction (Rust worker, same pass)
Per-frame derived features attached to the canonical model (cheap, vectorizable):
- `dist_to_ball[player]`, `closing_speed[player]`, `time_to_ball[player] = dist / max(closing_speed, ε)`.
- `goalside[player]` (bool): is player between ball and own goal along Y.
- `third`: which third of the field the ball is in (def/mid/att) **relative to target's team**.
- `teammate_sep[team]`: distance between the two teammates.
- `boost[player]`, `supersonic_time`, `zero_boost_time` accumulators.
- `on_wall/in_air` flags from z + on_ground.

Output: canonical match **plus** a `features` block. Hand off to scoring worker.

### 4.4 Role assignment — 1st vs 2nd man (scoring core)
Per team, per frame, decide who is **1st man** (committed/pressuring) vs **2nd man** (support).

- Primary signal: **lower `time_to_ball` ⇒ 1st man.** Tie-break by `dist_to_ball`.
- **Hysteresis** to stop role flicker: smooth with an EMA on the role-score, and require a role to persist ≥ `role_min_frames` (e.g. 0.4 s) before a swap registers. This matters — naive per-frame assignment produces garbage during rotations.
- Output: `role[player][frame] ∈ {first, second}` for the target's team (only the target's team is scored, but you compute the opponent team too because some metrics need opponent's 1st man, e.g. challenge detection).

### 4.5 Metric library (scoring core)
Each **metric** is a pure function over `(canonical, features, roles, target)` → a raw scalar (or a list of per-event scalars reduced to a scalar). Metrics are role-tagged. See **§6** for the full rubric. Each metric is independently unit-testable.

### 4.6 Scoring engine (scoring core)
1. **Normalize** each raw metric to 0–100 via its **calibration curve** (monotonic piecewise-linear or logistic; target band → saturates near 100; far from band → toward 0). Curves live in `ScoreConfig`.
2. **Aggregate** normalized metrics into the three sub-scores by config weights:
   `first = Σ w_i·m_i (i∈first-tagged)`, likewise `second`, `general`. Renormalize weights to 1.
3. **Composite** = `W_first·first + W_second·second + W_general·general` (top-level weights, config).
4. **Validity gate**: if analyzable possessions/frames < `min_sample`, mark report `low_confidence` and widen CI / suppress leaderboard write.

All weights/curves carry a `score_config_version`; persisted on the report.

### 4.7 Licence banding
Pure threshold lookup on `composite` from config, e.g. `Silver < Elite < Master` (+ intermediate bands you define). Banding table is versioned with the config.

### 4.8 Player-type classifier
Start **rule-based / nearest-centroid** (interpretable; no training data needed day one):
- Feature vector = a handful of normalized behavioral metrics: `challenge_rate`, `overcommit_rate`, `ball_chase_index`, `support_fraction`, `boost_hoard`.
- Define archetype centroids in config (e.g. *Calm Controller* = high support_fraction, low overcommit; *Diver* = high challenge_rate + high overcommit; *Stacker* = low spacing / frequent double-commit). Assign nearest centroid.
- Upgrade path: swap for a small trained classifier once you have a labeled corpus, behind the same interface.

### 4.9 Leak selector
Pick the **single highest-impact deficit**:
`impact_i = weight_i · (target_band_center_i − normalized_i)⁺` across all metrics; choose `argmax`. This guarantees the leak is both *bad* and *heavily weighted* — i.e. the change with the most score upside. Map the chosen metric → **book chapter** via the `leak_chapter_map` (config). Emit one focus line + the deep-link.

### 4.10 Report assembly
- Compose `Report` object (scores, licence, type, leak, focus, chapter link, per-metric breakdown, lobby comparison table).
- **Heatmaps**: bin target's positions (and touches) into a 2D field grid → render PNG/SVG (matplotlib/plotly headless). Cache by `replay_id+player`.
- **PDF**: HTML template → weasyprint or headless Chromium. Store; link from dashboard.
- Persist; **commit the credit** (convert soft hold to spent). On any failure after the hold, **auto-refund** the credit (matches the reference's "failed analysis returns the credit" rule).

---

## 5. Data model (SQLModel sketch)

```python
class Account(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    email: str = Field(index=True, unique=True)
    owns_book: bool = False          # entitlement gate, set by purchase webhook
    locked_player_id: str | None = None   # the "locked Pacifist Licence profile"

class CreditLedger(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    account_id: int = Field(index=True, foreign_key="account.id")
    delta: int                       # +20 monthly grant, +10 top-up, -1 spend, +1 refund
    reason: str                      # "monthly_grant" | "topup" | "spend" | "refund"
    expires_at: datetime | None      # top-ups: +30d; monthly: end of period
    created_at: datetime = Field(default_factory=utcnow)

class Replay(SQLModel, table=True):
    id: str = Field(primary_key=True)         # = sha256 of blob
    account_id: int = Field(index=True, foreign_key="account.id")
    playlist: str
    status: str = "queued"                    # queued|parsing|scoring|done|failed
    parser_version: str | None = None
    created_at: datetime = Field(default_factory=utcnow)

class Report(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    replay_id: str = Field(index=True, foreign_key="replay.id")
    player_id: str
    composite: float
    first_man: float
    second_man: float
    general: float
    licence: str
    player_type: str
    main_leak: str                  # metric key
    focus_chapter: str              # deep-link slug/anchor
    confidence: str = "ok"          # ok|low_confidence
    score_config_version: str
    parser_version: str
    metrics_json: str               # full per-metric breakdown (raw + normalized)
    created_at: datetime = Field(default_factory=utcnow)

class LeaderboardEntry(SQLModel, table=True):
    # materialized "best locked-profile score per account per season"
    id: int | None = Field(default=None, primary_key=True)
    account_id: int = Field(index=True, foreign_key="account.id")
    season: str = Field(index=True)
    player_id: str
    report_id: int = Field(foreign_key="report.id")
    composite: float = Field(index=True)
    first_man: float; second_man: float; general: float
    uploaded_at: datetime
```

Leaderboard integrity (mirrors the reference): only **locked-profile** reports with `confidence == "ok"` are eligible; keep one best per account per season; recompute the materialized table on each eligible write.

---

## 6. The scoring rubric (the actual IP)

This is the part that *is* the product. Each row: how to compute it from kinematics, which way is "good", and where its leak points. Bands/weights are starting guesses — **calibrate against a corpus before trusting numbers.**

### 1st-Man metrics (pressure quality)
| Key | Raw signal (from frames/events) | Good direction | Target band (pre-calibration) | Chapter |
|---|---|---|---|---|
| `challenge_timing` | On detected 50/50s (our 1st man + opp 1st man contest within radius R, small Δt), score arrival-with-control: had boost, on-ground or controlled approach, not late | higher | win/neutral ≥ 60% of challenges | "Trigger discipline" |
| `overcommit_rate` | Fraction of attacks where 1st man crosses ball/midline without retained possession **and** 2nd man can't cover (no goal-side support) | lower | ≤ 15% | "Controlled counterattacks" |
| `goalside_discipline_1st` | Fraction of **defensive** frames the 1st man stays goal-side of ball | higher | ≥ 85% | "Core game states / defence" |
| `first_touch_value` | Mean Δ in possession-expectancy after target's touches as 1st man (toward opp half / kept central = +, boom-away = −) | higher | positive | "Ground control / stop booming" |

### 2nd-Man metrics (support quality)
| Key | Raw signal | Good | Target band | Chapter |
|---|---|---|---|---|
| `support_spacing` | Mean teammate separation while target is 2nd man; penalize both too-close (double commit) and too-far (unreachable) | toward band | ~1200–2600 uu | "Central support" |
| `central_support_fraction` | Fraction of 2nd-man frames spent in central, goal-side support zone (not wide, not ball-watching) | higher | ≥ 70% | "Central support" |
| `second_boost_economy` | Mean boost held as 2nd man; time boost-starved while supporting | higher boost | mean ≥ 45, starved ≤ 10% | "String Theory / readiness" |
| `double_commit_rate` | Fraction of frames both teammates have low time_to_ball (both pressuring) | lower | ≤ 10% | "Central support" |
| `transition_readiness` | On possession flips, was 2nd man positioned to become 1st man (correct rotation, goal-side, facing play) | higher | ≥ 70% | "Core game states / transitions" |

### General metrics (cross-role habits)
| Key | Raw signal | Good | Target band | Chapter |
|---|---|---|---|---|
| `boost_management` | Composite: mean boost, %time at 0, big-pad efficiency, supersonic %time | higher | mean ≥ 40, zero ≤ 12% | "Fundamentals / boost" |
| `recovery_speed` | Mean time from aerial/challenge end → wheels-down + facing ball | lower | ≤ ~1.2 s | "Air system / recovery" |
| `ball_chase_index` | Correlation of **both** teammates' velocity vectors toward ball (high = stacking/chasing) | lower | ≤ 0.35 | "Positioning" |
| `possession_retention` | Touches keeping possession ÷ total touches; "booms" (high-power upfield clears, no target) as penalty | higher | ≥ 55% | "Ground control / stop booming" |
| `goalside_discipline_team` | %time at least one teammate is goal-side of ball on defence | higher | ≥ 95% | "Defence structure" |

**Behavioral features for the classifier** (`challenge_rate`, `overcommit_rate`, `ball_chase_index`, `central_support_fraction`, `boost_hoard = mean_boost/100`) are reused from the rows above — no extra computation.

---

## 7. API surface (FastAPI)

```
POST /v1/replays                 # multipart upload; validates, holds credit, enqueues → {replay_id}
GET  /v1/replays/{id}            # status: queued|parsing|scoring|done|failed
GET  /v1/reports/{id}            # full report JSON (own reports only)
GET  /v1/reports/{id}/pdf        # signed link / stream
GET  /v1/leaderboard?season=…    # public; top / season / recent feeds
POST /v1/account/lock-profile    # set locked_player_id (leaderboard eligibility)
GET  /v1/account/credits         # ledger-derived balance
POST /internal/webhooks/purchase # Stripe/PayPal → set owns_book, grant monthly credits
POST /internal/rescore/{id}      # admin: re-run scoring core at a new config version
```

Guards: `owns_book` required on `POST /v1/replays`; credit balance > 0; rate-limit uploads per account.

---

## 8. Credit & entitlement logic
- **Balance** = sum of non-expired ledger deltas. Monthly grant (+20) on billing webhook, expiring at period end (no rollover). Top-up (+10) expires +30 d, **consumed after** monthly credits (order the spend query by `expires_at`).
- **Spend lifecycle**: soft-hold at enqueue (−1, reason `spend`, tagged `pending`); confirm on `done`; **auto-refund** (+1, reason `refund`) on `failed` or if parse rejects post-hold. Idempotent by `replay_id`.

---

## 9. Concurrency, scaling, reproducibility
- API is async/non-blocking; all heavy work is off-thread via the queue. Workers scale horizontally; parsing (Rust) and scoring (Python) scale independently.
- **Idempotency**: `replay_id = sha256(blob)`; re-upload returns the existing report (unless `score_config_version` changed → offer re-score).
- **Re-scoring**: because the core is pure and inputs (canonical match) are cached, a new rubric version re-runs scoring **without re-parsing**. Keep canonical match blobs.
- Pin and persist `parser_version` + `score_config_version` on every report.

---

## 10. Testing strategy
- **Golden-file tests** on the scoring core: canonical-match fixtures → expected report JSON. The core is pure, so these are deterministic and fast.
- **Synthetic replays / canonical fixtures**: hand-construct frame sequences that isolate one behavior (e.g. a clean double-commit, a goal-side-loss, a boom-away) and assert the targeted metric moves and the leak selector fires.
- **Property tests** (hypothesis): scores ∈ [0,100]; sub-score weights sum to 1; role assignment partitions each team into exactly {first, second} per frame; refund leaves net-zero credit on failure.
- **Calibration tests**: against the labeled corpus, assert monotonic relationship between rank tier and composite (sanity that the metric tracks skill).
- **Parser adapter contract tests**: each adapter (boxcars/carball/ballchasing) must emit identical canonical schema for a shared sample.

---

## 11. Failure modes & edge cases
- Forfeits / early-leaves / mid-game disconnects → short or ragged frame counts → `low_confidence`, suppress leaderboard.
- Demolitions and kickoffs distort role/spacing → exclude kickoff windows and post-demo respawn windows from positional metrics (still count them for boost/possession).
- Overtime / variable match length → normalize per-minute or per-possession, never per-frame raw counts.
- Bots / AFK teammate → support metrics meaningless; detect near-zero teammate movement → flag.
- Smurf/alt integrity → handled by the locked-profile rule on the leaderboard, not in scoring.
- Parser drift between RL patches (network schema changes) → contract tests + parser version pinning; quarantine replays the parser can't fully decode.

---

## 12. Observability & privacy
- Per-stage timings (parse ms, score ms), queue depth, failure-rate by reason, credit refund rate.
- Log `score_config_version` distribution to catch un-rescored stragglers.
- Privacy: replays contain platform IDs/usernames (personal-ish). Store raw blobs encrypted at rest; allow account deletion to purge raw + reports; surface a retention policy. Consent already handled at web layer.

---

## 13. MVP cut (validate before building the rest)
**Milestone 1 — "does the score mean anything":** boxcars adapter → canonical model → role assignment → **4 metrics only** (`overcommit_rate`, `central_support_fraction`, `boost_management`, `possession_retention`) → composite + 3 sub-scores → licence band → leak→chapter. No classifier, no heatmaps, no seasons. Hand-label ~50 replays across ranks to calibrate the 4 curves and confirm composite tracks tier.

**Milestone 2:** full metric set, player-type classifier, heatmaps, PDF, credit ledger, entitlement webhook.

**Milestone 3:** leaderboard + seasons + Founding-N, re-score endpoint, second parser adapter for redundancy.

If Milestone 1's composite doesn't separate ranks on the labeled set, the rubric is wrong — fix that before writing anything else.

---

## 14. Open calibration questions
- Field/role thresholds (challenge radius R, support-zone polygon, spacing band) need empirical fitting per rank — they likely shouldn't be constants across the ladder.
- Sub-score top-level weights (`W_first/W_second/W_general`) are a product/coaching decision, not a technical one — get them from the framework owner's priorities.
- Licence band cutoffs depend entirely on the composite distribution you observe; set after corpus calibration, not before.
- Decide parser strategy early: self-host `boxcars` (control, no dep) vs `carball` (richer events, slower, Python) vs ballchasing API (least work, network + ToS dependency). This is the single highest-leverage build decision.
