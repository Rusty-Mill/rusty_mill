# Replay-Scoring Service (M2)

The web / DB / monetization layer (`replay-scoring-service-spec.md` §3–§9) that
wraps the Rust parse + feature + scoring worker. The Rust binary stays the source
of truth for the **scoring rubric**; this FastAPI + SQLModel service owns
**persistence, idempotency, the credit ledger, and the HTTP surface**.

It is a separate Python codebase from the Rust workspace (it shells out to the
`replay-scoring` binary), so it lives under `service/` and is tested
independently.

## Architecture (ports & adapters)

```
HTTP (FastAPI)  ──>  domain flow (service.py)  ──>  Scorer port (scoring.py)
      │                     │                              │
   accounts/credits    persistence (models.py,        SubprocessScorer ──> `replay-scoring` (Rust)
   guards (§7)         db.py) + blobs (§9/§12)         FakeScorer (tests)
                       credit ledger (credits.py, §8)
```

Scoring is an injected capability behind the `Scorer` protocol, so the whole
service is testable without the Rust toolchain or real `.replay` files. The
production adapter (`SubprocessScorer`) runs `replay-scoring <file> --json out`
and maps each per-player Rust `Report` to a stored `Report` row.

## What's implemented

- **Persistence** (§5): `Account`, `CreditLedger`, `Replay`, `Report`,
  `LeaderboardEntry` SQLModel tables; SQLite by default (point `RLS_DATABASE_URL`
  at Postgres for prod).
- **Idempotency** (§9): `replay_id = sha256(blob)`; a re-upload returns the
  existing replay/report and is **not** re-charged. Raw blobs are content-addressed
  on disk for re-scoring without re-upload.
- **Credit ledger** (§8): balance = sum of non-expired deltas; a **soft-hold** at
  enqueue → **confirm** on success → **auto-refund** on failure, idempotent by
  `replay_id`; holds inherit their grant's expiry (no rollover / no drift).
- **Endpoints** (§7): `POST /v1/replays`, `GET /v1/replays/{id}`,
  `GET /v1/reports/{id}`, `GET /v1/account/credits`, `POST /v1/account/lock-profile`,
  plus `GET /healthz`. Upload guards: `owns_book` + a positive credit balance.
- **Off-thread scoring**: upload enqueues a background task; the worker path shares
  the engine, so it's exercised in tests.

## Deferred follow-ups

Tracked in `docs/backlog.md` (M2): PDF rendering (`GET /v1/reports/{id}/pdf` via
the existing report HTML), public **leaderboard** + season materialization,
purchase **webhook** (`POST /internal/webhooks/purchase`), admin **re-score**
(`POST /internal/rescore/{id}` — the core is already pure, needs the cached
canonical blob), a real **job queue** (Celery/RQ vs. the in-process background
task), per-account **rate-limiting**, **encryption-at-rest**, and real **auth**
(the current `X-Account-Email` header is a dev stub — authentication is the web
layer's job).

## Run

```bash
cd service
pip install -e ".[dev]"

# Point the service at a built worker (or put `replay-scoring` on PATH):
export RLS_WORKER_BIN="$(git rev-parse --show-toplevel)/target/release/replay-scoring"
cargo build -p replay-scoring --release   # from the repo root

uvicorn app.main:app --reload
```

Then, e.g.:

```bash
curl -F file=@assets/replays/42f2.replay -F playlist=ranked-2v2 \
  -H 'X-Account-Email: me@example.com' http://127.0.0.1:8000/v1/replays
```

(An account needs `owns_book` + credits — granted by the purchase webhook in
production; in dev, seed the `account` / `credit_ledger` rows directly.)

## Test

```bash
cd service && pytest -q
```

The suite (`tests/`) covers the credit-ledger lifecycle, the ingest/score domain
flow, and the HTTP surface end-to-end with a fake scorer — no Rust build required.
CI runs it (`.github/workflows/ci.yml`, the `service` job).

## Configuration (env)

| Var | Default | Meaning |
|---|---|---|
| `RLS_DATABASE_URL` | `sqlite:///./rls.db` | SQLAlchemy URL |
| `RLS_BLOB_DIR` | `./blobs` | raw-replay store |
| `RLS_WORKER_BIN` | `replay-scoring` | Rust worker binary |
| `RLS_WORKER_TIMEOUT_S` | `300` | worker subprocess timeout |
| `RLS_SCORE_CONFIG_VERSION` | `score-v1` | current scoring version (re-score gate) |
| `RLS_MONTHLY_GRANT` | `20` | monthly credit grant |
| `RLS_UPLOAD_MAX_BYTES` | `26214400` | upload size cap |
