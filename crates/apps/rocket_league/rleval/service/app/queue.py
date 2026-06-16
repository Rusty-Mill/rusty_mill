"""Job-queue port: decouple *scheduling* scoring from *how* it runs (§4).

`upload` enqueues scoring rather than running it; the default
`BackgroundTaskQueue` runs it in-process via FastAPI's `BackgroundTasks`
(dev/single-node — the response returns `202` and scoring runs after it). A
production deployment swaps a broker-backed adapter (Celery/RQ) behind the same
one-method interface without touching the endpoint or the domain flow.

The job payload is **just the `replay_id`** — the worker reloads the replay from
the DB and the blob from the store — so it serializes trivially across a broker,
and the `Scorer` is constructed worker-side rather than shipped. A Celery adapter
is therefore a few lines::

    class CeleryJobQueue:
        def enqueue_scoring(self, replay_id: str) -> None:
            score_task.delay(replay_id)          # broker handoff

    @celery_app.task
    def score_task(replay_id: str) -> None:
        process(replay_id, build_scorer())       # worker-side scorer

Wire it by setting `app.state.queue_factory` to a callable returning the adapter
(see `main.get_queue`).
"""

from __future__ import annotations

from typing import Protocol

from fastapi import BackgroundTasks

from .scoring import Scorer
from .service import process


class JobQueue(Protocol):
    def enqueue_scoring(self, replay_id: str) -> None: ...


class BackgroundTaskQueue:
    """In-process adapter: scoring runs on this node after the response is sent.

    Holds the per-request `BackgroundTasks` and the app's `Scorer` (safe to carry
    here precisely because it never crosses a process boundary).
    """

    def __init__(self, background: BackgroundTasks, scorer: Scorer) -> None:
        self._background = background
        self._scorer = scorer

    def enqueue_scoring(self, replay_id: str) -> None:
        self._background.add_task(process, replay_id, self._scorer)
