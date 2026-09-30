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
| `--corpus <dir>` | `assets/corpus` | where it looks for calibrated artifacts (`rank_norms.json`, `fitted_config.json`, `value_model.json`, `xg_model.json`) |
| `--enable-admin-run` | off | lets the `/admin` page trigger retrain/recalibrate from the browser (§3.3) |
| `--data-dir <dir>` | off | save every analysis to per-account history and enable the **History** view (§2.1) |
| `--store fs\|mmdb` | `fs` | session backend under `--data-dir`; `mmdb` needs a build with `--features mmdb` (§2.3) |
| `--oidc-client-id`, `--oidc-redirect-uri`, `--oidc-users <file>` | off | Google sign-in; needs a build with `--features oidc` (§2.4) |
| `--teams <file>` | off | team workspaces (§2.2); requires `--accounts` and `--data-dir` |
| `--accounts <file>` | off (single-user) | require `Authorization: Bearer <token>`; one `account:token` per line, tokens ≥ 16 chars (§2.1) |

## 2. Use the app

- **Drop a `.replay`** onto the page (or pick one of the bundled samples) and it
  decodes, scores, and renders in-process — nothing leaves your machine.
- A **coordinate warning banner** appears when a replay breaks the geometry the
  position metrics assume — blue spawning on the `+Y` side (Pacifist depth and
  lateral scores would be mirrored) or positions far outside the arena (wrong
  units). It is checked from the replay's own kickoffs (`analyze::coords`) and is
  silent for a healthy replay; bounds are only checked on standard maps.
- Tabs across the top:
  - **Overview** — one row per player: decision-discipline composite, licence
    band, skills/min, value-impact ΔV, main leak.
  - **Improve** — turns the scoring leak, Pacifist faults, and the next-worst
    metric into short, concrete "what to work on" notes per player: templated
    from the same rubric definitions the scores come from (not generated), so
    the app doesn't just report a leak/fault happened but says what to do
    about it. A prioritized way into the Scoring and Pacifist tabs, not a
    replacement for them.
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

### 2.0 The analysis API

`POST /api/analyze` (and `GET /api/analyze/sample/<name>`) returns the analysis
**data** only — about 100 KB for a full lobby, versus 8.9 MB with the HTML panels
embedded. The 3D viewer, scoring report and ballchasing dashboard stay on the
server, and the UI fetches each from `GET /api/analysis/<analysis_id>/<panel>`
(`viewer`, `scoring`, `ballchasing`) the first time its tab is opened. The server
keeps the panels of the last four analyses, per account; an evicted or unknown id
is a 404 (re-run the analysis). Text and JSON responses over 1 KB are gzip-compressed when the client sends `Accept-Encoding: gzip` (a built-in encoder, no dependency): the 7.6 MB viewer panel travels as about 1.6 MB. Add `?inline=1` for the previous shape, with the
panels embedded in the response and no `analysis_id`. `rleval analyze --out` bundles
are unchanged (everything inline).

### 2.1 History and habits

Start with `--data-dir` and each analysis is saved (the Pacifist headline, fault
counts and per-dimension scores, plus whether the player's team won). Re-analyzing
the same replay is a no-op — sessions are keyed by a hash of the replay bytes.
The **History** link in the header lists your saved matches and, per player, a
cross-match read:

- **Work on this next** — the Pacifist dimension that drops most in your losses
  versus your wins (needs ≥ 2 wins *and* ≥ 2 losses and a gap of ≥ 5 points).
  With less data it names your weakest dimension and says so.
- Per-dimension overall / in-wins / in-losses scores (opportunity-weighted),
  a recurring Minor fault (top Minor in ≥ 2 matches), and a win/loss trend bar.

Players are identified by the **platform id** in the replay header (`steam:…`,
`xbox:…`, `epic:…`), so a rename does not split someone's history; the player
list shows the most recent name. Bots, and sessions saved before ids were
recorded, fall back to the exact display name — and those are kept as a
separate identity from the same person's id-keyed sessions (no guessing that two
are the same).

**Accounts.** With no `--accounts` file everything is one `local` account (fine on
loopback; the server warns if you bind elsewhere). With one, supply
`alice:<token>` lines and each request needs `Authorization: Bearer <token>`;
the History panel has a token field (kept in `localStorage`). Sessions are stored
at `<data-dir>/<account>/<key>.json`. Tokens are operator-supplied and compared
in constant time; this is a local seam meant to be replaced by a real identity
provider, not a hardened public-internet login — put it behind TLS if exposed.

**Play sessions and the training plan.** Add `?session=<name>` to an analysis (the
"Session" box in the UI) to group matches; unnamed matches are grouped by time — a
gap of over 2 hours starts a new session. History then also shows each session's
record and mean composite, and a **training plan** for the selected player: the three
metrics where they sit lowest against their rank bracket, each with the next
bracket's median as the target and whether the latest session moved toward it. A
metric needs 3 matches with rank norms applied before it is planned (fewer is one
match's noise). Sessions saved before this only carry the Pacifist data and do not
feed the plan.

### 2.2 Team workspaces

For a coach and a roster: one shared pool of matches, one read of how the team
plays. Enable with `--teams <file>` (needs `--accounts` and `--data-dir`); one
team per line:

```text
# team: role:account[=In-Game Name], ...
aces: coach:bailey, player:alice=Schutzein, player:bob=Nadir
```

- Roles are `coach` or `player`; every team needs at least one coach. The
  in-game name is how a replay's player is attributed to a member (defaults to
  the account name). It may also be a **platform id**, e.g.
  `player:alice=steam:76561198154819830`, which keeps attribution correct when
  they rename — prefer it once you know it (it appears as the player key in the
  History list). Startup fails on an unknown account, a
  duplicate account or in-game name, or a team with no coach.
- When a member analyzes a replay, **Share with team** puts it in the team's
  pool (`<data-dir>/_teams/<team>/`) as well as their own history. Re-sharing the
  same replay is a no-op, whoever uploads it.
- The History view gains a **Team** section: a **team rollup** ("work on this
  next" for the roster as a whole; a match counts once however many members
  played it) and a roster table. **Coaches** see every member; **players** see
  the rollup and only their own row. A non-member gets 404, same as for a team
  that does not exist.
- The rollup follows the same rules as §2.1: it names a loss habit only with ≥ 2
  wins and ≥ 2 losses in the pool, otherwise the weakest dimension.

Limits: membership is a file read at startup (restart to change it), and a
roster entry that is a display name (rather than a platform id) needs updating
when that member renames.

### 2.3 Storage backends

`--data-dir` keeps sessions with one of two backends, both behind the same
`SessionStore` port and held to the same tests:

- **`fs`** (default) — one JSON file per session at
  `<data-dir>/<account>/<key>.json`. No extra dependencies; easy to inspect.
- **`mmdb`** — the embedded record store from the Rusty-Mill monorepo
  (`rusty_multimodal_db_engine`): mmap-backed, an fsync'd insert log, and a
  directory lock. Build with `cargo build --release -p rleval-app --features mmdb`
  and serve with `--store mmdb`. Its constraints matter: every record lives in
  RAM (fine for per-account histories), and **one process per directory** — a
  second server on the same `--data-dir` is refused with an error rather than
  allowed to corrupt it.

Sessions are stored inside the engine as JSON in a small fixed envelope, not as
native engine records: the engine encodes records with bincode, which is not
self-describing, so a new field would otherwise be a breaking on-disk change.

**Moving from `fs` to `mmdb`:**

```sh
cargo run --release -p rleval-app --features mmdb -- import-json --data-dir ./data
cargo run --release -p rleval-app --features mmdb -- serve --data-dir ./data --store mmdb
```

`import-json` copies every account history and team pool, is idempotent, and
leaves the JSON files in place (delete them once you are satisfied). The engine
dependency is a git dependency pinned to a commit of `Rusty-Mill/rusty_mill`;
cargo prints a few harmless "invalid character in package name" lines about
template `Cargo.toml` files inside that repository when it resolves it.

### 2.4 Google sign-in

Bearer tokens (§2.1) suit scripts; people sign in with Google instead. Build with
`--features oidc`, then:

```sh
export RLEVAL_OIDC_CLIENT_SECRET=...          # from the Google Cloud console
rleval serve --data-dir ./data \
  --oidc-client-id 1234-abc.apps.googleusercontent.com \
  --oidc-redirect-uri https://rleval.example.com/auth/callback \
  --oidc-users ./oidc-users.txt
```

`oidc-users.txt` is `email:account`, one per line. **Only verified emails on that
list can sign in** — there is no auto-provisioning — and each maps to one
account (which can also be listed in `--accounts` and `--teams`). Register the
redirect URI above as an authorized redirect URI on your Google OAuth client.
The secret is read from the environment, never a flag.

- The header shows **Sign in with Google** / your account / **Sign out**. A
  successful login sets an `HttpOnly`, `SameSite=Lax` session cookie (`Secure` when
  the redirect URI is https). Scripts keep using `--accounts` bearer tokens; both work together.
- Turning sign-in on closes the API to anonymous callers, even with no
  `--accounts` file.
- **What is verified:** the login `state` (single use, 10 minutes), PKCE, the ID
  token's RS256 signature against Google's published keys (algorithm fixed by the
  server), issuer, audience, expiry, the login's nonce, and `email_verified`.
- Cookie-authenticated writes must be same-origin (an `Origin` that names another
  host gets 403).
- Sessions live in memory (12 hours); restarting the server signs everyone out.
  Run it behind TLS — the endpoints refuse non-https provider URLs except on
  loopback (for local development against a mock provider).
- Another OpenID Connect provider works via `--oidc-auth-url`, `--oidc-token-url`,
  `--oidc-jwks-url` and `--oidc-issuer`, provided it signs ID tokens with RS256.

The protocol code is Rusty-Mill's `rusty_oauth`; the HTTPS transport is
`rusty_http` + `rusty_tls`. This path was exercised end to end against a local
mock provider, **not against Google itself** — try it with a test OAuth client
before relying on it.

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

### 3.2a Fit expected goals

```bash
cargo run --release -p replay-scoring --bin xg-fit
```

Turns every shot in the corpus into a labelled example (goal = 1), holds out every
fifth match, prints the held-out Brier score (base rate vs the built-in prior vs the
fit) and a reliability table, then writes `xg_model.json` next to the manifest. Until
that file exists the app scores shots with a hand-set prior (`xg-prior-v1`) and the
Moments tab says so. Needs the corpus replays on disk (see `assets/corpus/README.md`).

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

### 3.6 Fetching one player's full match history (case studies)

To go beyond whatever replays you happen to have and pull one named
player's **complete** ranked-doubles history from ballchasing:

```sh
BC_TOKEN=<token> python assets/corpus/fetch_player_history.py "PlayerName"
```

Writes a chronological, ballchasing-dated manifest to
`assets/case-studies/<slug>/manifest.json` (kept, unlike the `.replay` files
alongside it) and downloads the replays. Deliberately separate from the
calibration corpus — this is every match one specific account appears in,
which would bias a rank-stratified sample if mixed in. Then:

```sh
cargo build --release -p replay-pacifist --features corpus-validate --bin case_study
./target/release/case_study assets/case-studies/<slug>/manifest.json "PlayerName"
```

prints, per match in date order, that player's scoring composite and
Pacifist score side by side, plus a per-bucket mean summary — the direct
"did either number move with this player's real rank change" check. See
[`docs/case-study-player-progression.md`](case-study-player-progression.md)
for what this found (short version: not yet, and it's an open question why).

## 4. One-off CLI usage (no server)

```sh
cargo run --release -p replay-analyzer -- assets/replays/42f2.replay --json out.json
cargo run --release -p replay-scoring -- assets/replays/42f2.replay --html report.html
cargo run --release -p replay-skills -- assets/replays/42f2.replay
cargo run --release -p replay-viewer -- assets/replays/42f2.replay --html 42f2.html
cargo run --release -p replay-pacifist -- assets/replays/42f2.replay   # Pacifist score + verdict to stderr
```
