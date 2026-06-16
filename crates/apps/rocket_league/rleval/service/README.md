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
and maps each per-player Rust `Report` to a stored `Report` row. Four more
seams are pluggable the same way: the `PdfRenderer` (§4.10), the at-rest `Cipher`
(§12), the `JobQueue` (§4) that schedules scoring, and the `BlobStore` (§5,
filesystem or S3).

## What's implemented

- **Persistence** (§5): `Account`, `CreditLedger`, `Replay`, `Report`,
  `LeaderboardEntry` SQLModel tables; SQLite by default (point `RLS_DATABASE_URL`
  at Postgres for prod).
- **Idempotency** (§9): `replay_id = sha256(blob)`; a re-upload returns the
  existing replay/report and is **not** re-charged. Raw blobs are content-addressed
  on disk for re-scoring without re-upload.
- **Storage backend** (§5): the `BlobStore` port (`blobs.py`) is `fs` by default
  (files at `blob_dir`) or `s3` (`RLS_BLOB_BACKEND=s3`, incl. MinIO/localstack via
  `RLS_S3_ENDPOINT_URL`) so API and workers don't share a local disk. The backend
  moves opaque `(key, bytes)`; cipher/gzip/dedupe sit above it, identical on either.
- **Encryption-at-rest** (§12): every stored artifact (raw blob, report HTML/PDF,
  canonical cache) is written through a pluggable cipher. With `RLS_ENCRYPTION_KEY`
  set it's stdlib AEAD (HMAC-SHA256 CTR + encrypt-then-MAC); unset, a no-op
  (byte-identical to before). Keys stay the plaintext hash, so dedupe is unaffected
  and the worker — which only ever sees decrypted bytes — is oblivious. Swap a
  KMS/AES-GCM adapter behind the same `seal`/`unseal` port in production.
- **Credit ledger** (§8): balance = sum of non-expired deltas; a **soft-hold** at
  enqueue → **confirm** on success → **auto-refund** on failure, idempotent by
  `replay_id`; holds inherit their grant's expiry (no rollover / no drift).
- **Endpoints** (§7): `POST /v1/replays`, `GET /v1/replays/{id}`,
  `GET /v1/reports/{id}`, `GET /v1/reports/{id}/pdf`, `GET /v1/leaderboard`,
  `GET /v1/account/credits`, `POST /v1/account/lock-profile`,
  `POST /internal/webhooks/purchase`, `POST /internal/rescore/{id}`, plus
  `GET /healthz` and `GET /metrics`. Upload guards: `owns_book`, a positive credit
  balance, and a per-account **rate limit** (§7).
- **Observability**: a stdlib metrics registry (`metrics.py`) exposed at
  `GET /metrics` in Prometheus text format — upload outcomes, scoring results,
  **per-stage scoring timings** (parse+score / persist / leaderboard), and HTTP
  RED (request count + latency by route, via a pure-ASGI middleware). No
  `prometheus_client` dep; the exposition parses with a real Prometheus client.
- **PDF report** (§4.10): the worker's PDF-ready report HTML (captured at score
  time) is rendered to PDF via a `PdfRenderer` port (weasyprint adapter) and
  **cached per replay**; served owner-gated at `GET /v1/reports/{id}/pdf`. Rendering
  is optional (the `pdf` extra) — tests use a fake renderer, so the base install
  doesn't need weasyprint's native libs.
- **Re-score** (§8/§9): `POST /internal/rescore/{id}` re-runs the scoring core and
  **swaps in the fresh reports** without a re-upload — refreshing the leaderboard
  and dropping the stale PDF. With canonical caching enabled (`RLS_CACHE_CANONICAL`,
  off by default), re-score runs **from the cached canonical match, skipping the
  parse** (`replay-scoring --dump-canonical` / `--from-canonical`, byte-identical
  to a fresh parse); otherwise it re-parses.
- **Leaderboard** (§5/§8): materialized **best composite per account per season**;
  only **locked-profile** reports with `confidence == "ok"` are eligible, recomputed
  on each successful score. Public read at `GET /v1/leaderboard?season=…&limit=…`.
- **Entitlement webhook** (§8): `POST /internal/webhooks/purchase` flips
  `owns_book` and grants credits — monthly (expires period end, no rollover) or a
  top-up (+30 d) — idempotent by the provider's event id, with an `X-Signature`
  HMAC-SHA256 verified when `RLS_WEBHOOK_SECRET` is set.
- **Off-thread scoring** (§4): upload *enqueues* scoring through a `JobQueue` port
  (`queue.py`). The default `BackgroundTaskQueue` runs it in-process (exercised in
  tests); set `RLS_QUEUE_BACKEND=celery` and the **`CeleryJobQueue`** hands the
  `replay_id` to Redis for dedicated workers (`celery_app.py`) — verified live
  end-to-end (real broker + worker + Rust scorer). `docker compose up` brings up
  the whole stack.

## Deferred follow-ups

Tracked in `docs/backlog.md` (M2): real **auth** (the current `X-Account-Email`
header is a dev stub — authentication is the web layer's job) and **Postgres**
(the DB side of true multi-node — object storage is done; the engine still hard-codes
a SQLite-only connect-arg and uses create-all rather than migrations).

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

### Scaling out: Celery + Redis (§4)

By default scoring runs in-process (FastAPI `BackgroundTasks`). Set
`RLS_QUEUE_BACKEND=celery` and the API instead enqueues to Redis; dedicated
workers score in their own processes. Locally:

```bash
pip install -e ".[queue]"           # celery + redis
redis-server &                      # the broker
export RLS_QUEUE_BACKEND=celery RLS_CELERY_BROKER_URL=redis://localhost:6379/0
celery -A app.celery_app:celery_app worker -Q scoring --concurrency=2 &   # a worker
uvicorn app.main:app                # the API (now enqueues)
```

Or the whole stack (Redis + API + worker, worker image bundles the Rust binary):

```bash
docker compose up --build           # repo root
```

The job payload is just the `replay_id`, so workers reload the replay from the DB
and the blob from the store and build their own scorer — nothing app-specific
crosses the broker.

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
| `RLS_BLOB_BACKEND` | `fs` | artifact store: `fs` or `s3` |
| `RLS_BLOB_DIR` | `./blobs` | artifact dir (`fs` backend) |
| `RLS_S3_BUCKET` | `rls-blobs` | bucket (`s3` backend) |
| `RLS_S3_PREFIX` | _(empty)_ | key prefix (`s3` backend) |
| `RLS_S3_REGION` | `us-east-1` | region (`s3` backend) |
| `RLS_S3_ENDPOINT_URL` | _(empty)_ | custom endpoint (MinIO/localstack) |
| `RLS_WORKER_BIN` | `replay-scoring` | Rust worker binary |
| `RLS_WORKER_TIMEOUT_S` | `300` | worker subprocess timeout |
| `RLS_SCORE_CONFIG_VERSION` | `score-v1` | current scoring version (re-score gate) |
| `RLS_CACHE_CANONICAL` | `0` | cache the canonical blob for no-re-parse re-score |
| `RLS_MONTHLY_GRANT` | `20` | monthly credit grant |
| `RLS_UPLOAD_MAX_BYTES` | `26214400` | upload size cap |
| `RLS_UPLOAD_RATE_LIMIT` | `30` | max new uploads per window/account |
| `RLS_UPLOAD_RATE_WINDOW_S` | `3600` | rate-limit window (s) |
| `RLS_WEBHOOK_SECRET` | _(empty)_ | webhook HMAC secret (empty = no verify) |
| `RLS_ENCRYPTION_KEY` | _(empty)_ | at-rest artifact key (empty = store verbatim) |
| `RLS_QUEUE_BACKEND` | `inprocess` | `inprocess` or `celery` |
| `RLS_CELERY_BROKER_URL` | `redis://localhost:6379/0` | Celery broker (when `celery`) |
