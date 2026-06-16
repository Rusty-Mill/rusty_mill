"""Idempotency key + the ingest/score domain flow (spec §4/§9)."""

from __future__ import annotations

import hashlib

from sqlmodel import select

from app import credits, service
from app.models import Replay, Report


def test_replay_id_is_sha256_of_blob():
    assert service.replay_id_for(b"abc") == hashlib.sha256(b"abc").hexdigest()


def test_ingest_creates_queued_replay_and_holds_credit(session, make_account):
    account = make_account(credits_n=2)
    replay = service.ingest(session, account, b"replay-bytes", "ranked-2v2")
    assert replay.status == "queued"
    assert replay.id == service.replay_id_for(b"replay-bytes")
    assert session.get(Replay, replay.id) is not None
    assert credits.balance(session, account.id) == 1  # one credit held


def test_run_scoring_success_stores_reports_and_confirms(session, make_account, scorer):
    account = make_account(credits_n=2)
    replay = service.ingest(session, account, b"good", "ranked-2v2")
    service.run_scoring(session, replay, b"good", scorer)

    replay = session.get(Replay, replay.id)
    assert replay.status == "done"
    assert replay.parser_version == "boxcars-fake"
    rows = session.exec(select(Report).where(Report.replay_id == replay.id)).all()
    assert len(rows) == 1 and rows[0].player_id == "Alice"
    assert credits.balance(session, account.id) == 1  # held credit confirmed


def test_run_scoring_failure_marks_failed_and_refunds(
    session, make_account, failing_scorer
):
    account = make_account(credits_n=2)
    replay = service.ingest(session, account, b"bad", "ranked-2v2")
    assert credits.balance(session, account.id) == 1

    service.run_scoring(session, replay, b"bad", failing_scorer)

    replay = session.get(Replay, replay.id)
    assert replay.status == "failed"
    assert replay.error
    assert credits.balance(session, account.id) == 2  # auto-refunded, net-zero
