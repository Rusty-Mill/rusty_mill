"""Celery app + scoring task — the broker-backed worker side of the `JobQueue`.

Run a worker with::

    celery -A app.celery_app:celery_app worker -Q scoring --concurrency=1

The task carries only the `replay_id`: it reloads the replay from the DB and the
blob from the store, and builds its own `Scorer` from settings — the scorer never
crosses the broker. `worker_process_init` initialises the DB engine once per
worker process (the API process does this in its lifespan instead).

`concurrency=1` + `prefetch_multiplier=1` keep one heavy parse+score job per
worker at a time; `acks_late` re-delivers a job if a worker dies mid-score (the
domain flow is idempotent — a re-run replaces the same replay's reports).
"""

from __future__ import annotations

from celery import Celery
from celery.signals import worker_process_init

from .config import settings
from .db import init_db
from .scoring import Scorer, SubprocessScorer
from .service import process

celery_app = Celery("rls", broker=settings.celery_broker_url)
celery_app.conf.update(
    task_default_queue="scoring",
    task_acks_late=True,
    worker_prefetch_multiplier=1,
    broker_connection_retry_on_startup=True,
)


def build_scorer() -> Scorer:
    """The worker-side scorer (kept tiny so tests can monkeypatch it)."""
    return SubprocessScorer(
        settings.worker_bin, settings.worker_timeout_s, settings.cache_canonical
    )


@worker_process_init.connect
def _init_worker(**_kwargs) -> None:
    init_db()


@celery_app.task(name="rls.score")
def score_task(replay_id: str) -> None:
    process(replay_id, build_scorer())
