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
  `GET /v1/reports/{id}`, `GET /v1/reports/{id}/pdf`, `GET /v1/leaderboard`,
  `GET /v1/account/credits`, `POST /v1/account/lock-profile`,
  `POST /internal/webhooks/purchase`, `POST /internal/rescore/{id}`, plus
  `GET /healthz`. Upload guards: `owns_book` + a positive credit balance.
- **PDF report** (§4.10): the worker's PDF-ready report HTML (captured at score
  time) is rendered to PDF via a `PdfRenderer` port (weasyprint adapter) and
  **cached per replay**; served owner-gated at `GET /v1/reports/{id}/pdf`. Rendering
  is optional (the `pdf` extra) — tests use a fake renderer, so the base install
  doesn't need weasyprint's native libs.
- **Re-score** (§8/§9): `POST /internal/rescore/{id}` re-runs the (now-newer)
  scoring core and **swaps in the fresh reports** without a re-upload — refreshing
  the leaderboard and dropping the stale PDF. (Currently re-parses; the no-re-parse
  canonical-blob cache is the remaining §9 optimization.)
- **Leaderboard** (§5/§8): materialized **best composite per account per season**;
  only **locked-profile** reports with `confidence == "ok"` are eligible, recomputed
  on each successful score. Public read at `GET /v1/leaderboard?season=…&limit=…`.
- **Entitlement webhook** (§8): `POST /internal/webhooks/purchase` flips
  `owns_book` and grants credits — monthly (expires period end, no rollover) or a
  top-up (+30 d) — idempotent by the provider's event id.
- **Off-thread scoring**: upload enqueues a background task; the worker path shares
  the engine, so it's exercised in tests.

## Deferred follow-ups

Tracked in `docs/backlog.md` (M2): a **no-re-parse re-score** (cache the
canonical-match blob so re-score skips parsing — the worker is serde-ready), a
real **job queue** (Celery/RQ vs. the in-process background task), per-account
**rate-limiting**,
webhook **signature verification**, **encryption-at-rest**, and real **auth** (the
current `X-Account-Email` header is a dev stub — authentication is the web layer's
job).

## Run

```bash
cd service
pip install -e ".[dev]"

# Point the service at a built worker (or put `replay-scoring` on PATH):
export RLS_WORKER_BIN="$(git rev-parse --show-toplevel)/target/release/replay-scoring"
cargo build -p replay-scoring --release   # from the repo root

uvicorn app.main:app --reload
```

Then provision an account (the webhook flips `owns_book` + grants credits), and
upload:

```bash
curl -X POST http://127.0.0.1:8000/internal/webhooks/purchase \
  -H 'content-type: application/json' \
  -d '{"email":"me@example.com","event_id":"dev-1","kind":"monthly","grant_book":true}'

curl -F file=@assets/replays/42f2.replay -F playlist=ranked-2v2 \
  -H 'X-Account-Email: me@example.com' http://127.0.0.1:8000/v1/replays
```

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
