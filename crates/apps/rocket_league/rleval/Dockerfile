# syntax=docker/dockerfile:1
#
# One image, two roles: the FastAPI API and the Celery scoring worker. Both need
# the Rust `replay-scoring` binary, so stage 1 builds it and stage 2 bundles it.
# compose runs this image as `api` (default CMD) and `worker` (overridden CMD).

# --- stage 1: build the Rust scoring worker ---
FROM rust:1-slim AS worker
WORKDIR /src
COPY . .
RUN cargo build --release --bin replay-scoring

# --- stage 2: the Python service ---
FROM python:3.11-slim AS service
WORKDIR /srv
COPY service/pyproject.toml ./
COPY service/app ./app
RUN pip install --no-cache-dir -e ".[queue]"
COPY --from=worker /src/target/release/replay-scoring /usr/local/bin/replay-scoring
ENV RLS_WORKER_BIN=/usr/local/bin/replay-scoring \
    RLS_QUEUE_BACKEND=celery \
    RLS_CELERY_BROKER_URL=redis://redis:6379/0
EXPOSE 8000
# Default role: the API. The worker service overrides this command in compose.
CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "8000"]
