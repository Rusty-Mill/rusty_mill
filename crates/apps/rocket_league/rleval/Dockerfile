# syntax=docker/dockerfile:1
#
# One image, two roles: the FastAPI API and the Celery scoring worker. Both need
# the Rust `replay-scoring` binary, so stage 1 builds it and stage 2 bundles it.
# compose runs this image as `api` (default CMD) and `worker` (overridden CMD).
#
# Build from the **monorepo root**: `replay-scoring` is one crate in the root
# workspace, so cargo needs the workspace manifest, the lockfile and every
# member's manifest. compose passes that context; by hand:
#
#     docker build -f crates/apps/rocket_league/rleval/Dockerfile -t rleval .
#
# `Dockerfile.dockerignore` keeps `target/`, `.git` and Python debris out.

# --- stage 1: build the Rust scoring worker ---
FROM rust:1-slim AS worker
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p replay-scoring --bin replay-scoring \
    && cp target/release/replay-scoring /usr/local/bin/replay-scoring

# --- stage 2: the Python service ---
FROM python:3.11-slim AS service
WORKDIR /srv
COPY crates/apps/rocket_league/rleval/service/pyproject.toml crates/apps/rocket_league/rleval/service/alembic.ini ./
COPY crates/apps/rocket_league/rleval/service/app ./app
COPY crates/apps/rocket_league/rleval/service/migrations ./migrations
RUN pip install --no-cache-dir -e ".[queue,migrations,postgres]"
COPY --from=worker /usr/local/bin/replay-scoring /usr/local/bin/replay-scoring
ENV RLS_WORKER_BIN=/usr/local/bin/replay-scoring \
    RLS_QUEUE_BACKEND=celery \
    RLS_CELERY_BROKER_URL=redis://redis:6379/0
EXPOSE 8000
# Default role: the API. The worker / migrate services override this in compose.
CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "8000"]
