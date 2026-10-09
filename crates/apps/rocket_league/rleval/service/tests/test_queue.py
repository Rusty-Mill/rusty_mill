"""Celery adapter (§4): the broker-backed JobQueue, exercised in eager mode.

`task_always_eager` runs the real task machinery synchronously in-process — no
Redis — so we test enqueue → `score_task` → `process` end-to-end with a fake
scorer. The live-broker path is verified separately (see the PR / README).
"""

from __future__ import annotations

import pytest

pytest.importorskip("celery")

import app.celery_app as ca  # noqa: E402
from app.models import Replay  # noqa: E402
from app.queue import CeleryJobQueue  # noqa: E402
from app.service import ingest  # noqa: E402


def test_celery_eager_executes_scoring(engine, session, scorer, make_account, monkeypatch):
    # Worker-side scorer is built (not injected), so swap in the fake there.
    monkeypatch.setattr(ca, "build_scorer", lambda: scorer)
    monkeypatch.setattr(ca.celery_app.conf, "task_always_eager", True)

    account = make_account(credits_n=5)
    replay = ingest(session, account, b"celery-eager-blob", "ranked")
    assert replay.status == "queued"

    # Real Celery dispatch (.delay), run synchronously by eager mode.
    CeleryJobQueue().enqueue_scoring(replay.id)

    session.expire_all()
    assert session.get(Replay, replay.id).status == "done"
    assert scorer.calls == 1 and scorer.last_method == "score"
