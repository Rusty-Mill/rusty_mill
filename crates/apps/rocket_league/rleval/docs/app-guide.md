# Running, calibrating, and using the `rleval` app

The unified application (`app` crate, binary `rleval`) ties every engine in the
workspace — `replay-analyzer`, `replay-scoring`, `replay-skills`, `replay-value`,
`replay-pacifist`, `replay-viewer` — into one in-process pipeline behind a single
web page. This is the day-to-day reference for running it, understanding what
you're looking at, and calibrating the engines it runs.

## 1. Run the app

```sh
# Web UI — serve locally, then open the URL and drop a .replay file
cargo run --release -p rleval-app -- serve
# → http://127.0.0.1:8080

# One-shot — analyze a single file into a self-contained HTML report, no server
cargo run --release -p rleval-app -- analyze path/to/game.replay --out game.html
```

Build in `--release` — the analyzer/scoring passes are heavy enough that a debug
build is noticeably slower per replay.

`serve` flags:

| Flag | Default | What it does |
|---|---|---|
| `--host` | `127.0.0.1` | bind address |
| `--port` | `8080` | port |
| `--replays <dir>` | `assets/replays` | sample `.replay` files listed in the UI's picker |
| `--corpus <dir>` | `assets/corpus` | where it looks for calibrated artifacts (`rank_norms.json`, `fitted_config.json`, `value_model.json`) |
| `--enable-admin-run` | off | lets the `/admin` page trigger retrain/recalibrate from the browser (§3.3) |

## 2. Use the app

- **Drop a `.replay`** onto the page (or pick one of the bundled samples) and it
  decodes, scores, and renders in-process — nothing leaves your machine.
- Tabs across the top:
  - **Overview** — one row per player: decision-discipline composite, licence
    band, skills/min, value-impact ΔV, main leak.
  - **Stats** — core scoreboard, turnovers, boost economy, movement,
    positioning (ballchasing-parity aggregates).
  - **3D Viewer** — the full replay, scrubbable, with role/win-prob/impact
    overlays.
  - **Scoring** — the detailed lobby report with heatmaps.
  - **Skills** — mechanical skill counts per player.
  - **Impact** — the ΔV value model's per-touch swing analysis.
  - **Pacifist** — the eight-dimension Pacifist system-adherence score, the
    FM-1 PASS/FAIL driving-test verdict with Minor/Major fault counts, and a
    timestamped list of any Major faults. This measures adherence to a
    specific coaching system, not rank — see
    [`docs/pacifist-score-validation.md`](pacifist-score-validation.md) for
    what the corpus says about that distinction.
- If `assets/corpus/rank_norms.json` is present, every score also gets a
  **"vs rank"** column — the lobby's rank is inferred from the replay (or pass
  `?rank=diamond` etc. on the `/api/analyze` call) and each player is graded as
  a percentile within that bracket.
- **`/admin`** — a read-only snapshot of what's actually running: active config
  versions, the fitted artifacts on disk (with mtimes, so you can see if
  they're stale), the corpus size per rank bucket, and the value model's
  feature importances. Good first stop to check "is this running defaults or a
  corpus fit?"

## 3. Calibrate

Calibration is a **corpus** operation, separate from serving. Three layers, run
in order, each optional — the app runs sane defaults with none of it.

### 3.1 Get the corpus

The `.replay` files are gitignored (large, re-downloadable); only
`assets/corpus/manifest.json` (ballchasing IDs + ranks) is committed. You need a
ballchasing API token first:

```sh
cp .env.example .env
# edit .env: BC_TOKEN=<your token from https://ballchasing.com/upload -> Account -> API key>
```

`.env` is gitignored — never commit a token. The corpus data-tooling scripts
load it automatically (`assets/corpus/_env.py`); a real environment variable, if
set, still takes precedence.

Download the replays the manifest already lists:

```sh
python assets/corpus/refresh_corpus_replays.py           # all ~1,057 (a couple are gone upstream)
python assets/corpus/refresh_corpus_replays.py --limit 2   # just try the tooling
python assets/corpus/refresh_corpus_replays.py --bucket gold --limit 5
```

To grow the corpus with fresh replays instead of just re-downloading the
current list:

```sh
python assets/corpus/expand_manifest.py --per-tier 250       # search ballchasing, append manifest entries
python assets/corpus/refresh_corpus_replays.py                 # download the newly-listed files
python assets/corpus/refresh_manifest_playlist.py               # backfill playlist/team_size from headers
```

### 3.2 Calibrate each engine

```sh
# Decision-discipline scoring: fits curves to the corpus, writes fitted_config.json
cargo run --release -p replay-scoring --bin calibrate -- assets/corpus/manifest.json

# Cross-checks the rubric against the independent value model, promotes vindicated
# candidate metrics, writes back to fitted_config.json
cargo run --release -p replay-scoring --bin reconcile -- assets/corpus/manifest.json --promote
# add --gate in CI to fail the build on any rank-vs-impact sign disagreement

# Value model: trains the GBT/logistic predictor with a replay-level train/val split
cargo run --release -p replay-value --bin train_corpus -- assets/corpus/manifest.json

# Mechanical skills: refits confidence-ramp thresholds per skill
cargo run --release -p replay-skills --bin calibrate-skills -- assets/corpus/manifest.json
```

Each writes its fitted artifact into `assets/corpus/` (`fitted_config.json`,
`value_model.json`, `fitted_skill_config.json`). The CLI tools and the app's
`/admin` page pick these up automatically; `serve`'s default
`--corpus assets/corpus` already points at them.

### 3.3 Calibrate from the browser

Start with `--enable-admin-run` and the `/admin` page exposes buttons that run
`calibrate` → `reconcile --promote` and `train_corpus` for you — the same
commands as §3.2, no terminal needed. Handy for a non-technical operator, but
it shells out to `cargo run --release`, so the first click is slow.

### 3.4 Pacifist calibration — the one open item

The Pacifist rubric's thresholds (engagement radii, `empty_boost`, `major_cap`,
the trail-distance and depth/lateral bands) are currently **hand-set
approximations of the coaching guide's own language**, not fit to data — by
design, since the guide is explicit that rank is *not* the right calibration
target for a system-adherence score (see
[`docs/pacifist-score-validation.md`](pacifist-score-validation.md)). What
exists today is validation, not calibration:

```sh
# Does the current rubric track rank at all, per dimension and per bracket?
# A regression check after any threshold change, not a fitting step.
cargo build --release -p replay-pacifist --features corpus-validate --bin validate_pacifist
./target/release/validate_pacifist assets/corpus/manifest.json
```

A true calibration pass needs a small set of replays labeled "very Pacifist" vs.
"very not," then tuning the thresholds against that judgment — there's no
shortcut for that part; it's a human-judgment input, not more engineering.

### 3.5 Pacifist multi-match aggregation

A single match's Pacifist read is noisy (5–15 episodes per dimension); a
corpus experiment confirmed aggregating a player's matches roughly doubles to
triples rank correlation, strengthening with more matches per player (see
[`docs/pacifist-score-validation.md`](pacifist-score-validation.md)'s
"multi-match aggregation" section). There's no per-account history yet (that
needs the M2 service layer in `docs/backlog.md`), but you can get the
aggregated read for one player manually:

```sh
# Oldest replay first — the trend line and "most recent Major" both read that order.
cargo run --release -p replay-pacifist --bin pacifist_history -- \
  "PlayerName" match1.replay match2.replay match3.replay
```

Matches a replay's roster by exact display name; a file where the name
doesn't appear is skipped with a warning rather than failing the run.

## 4. One-off CLI usage (no server)

```sh
cargo run --release -p replay-analyzer -- assets/replays/42f2.replay --json out.json
cargo run --release -p replay-scoring -- assets/replays/42f2.replay --html report.html
cargo run --release -p replay-skills -- assets/replays/42f2.replay
cargo run --release -p replay-viewer -- assets/replays/42f2.replay --html 42f2.html
cargo run --release -p replay-pacifist -- assets/replays/42f2.replay   # Pacifist score + verdict to stderr
```
