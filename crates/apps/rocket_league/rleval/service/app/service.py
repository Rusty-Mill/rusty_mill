"""Domain flow: ingest a replay, then score it off-thread (spec §4/§9).

Pure-ish orchestration over the persistence + credit + scoring pieces. The HTTP
layer (`main.py`) calls [`ingest`] synchronously and schedules [`process`] as a
background task; both share the same engine, so the worker path is exercised in
tests too.
"""

from __future__ import annotations

import hashlib

from sqlmodel import Session

from . import blobs, credits, leaderboard
from .config import settings
from .db import session_scope
from .models import Account, Replay, Report
from .scoring import Scorer


def replay_id_for(blob: bytes) -> str:
    """Idempotency key: ``sha256(blob)`` (§9)."""
    return hashlib.sha256(blob).hexdigest()


def ingest(session: Session, account: Account, blob: bytes, playlist: str) -> Replay:
    """Store the blob, create the queued replay, and soft-hold one credit.

    Caller must enforce the upload guards (`owns_book`, balance > 0) and dedupe
    first — see `main.upload`.
    """
    rid = replay_id_for(blob)
    blobs.store(rid, blob)
    replay = Replay(id=rid, account_id=account.id, playlist=playlist, status="queued")
    session.add(replay)
    credits.hold(session, account.id, rid)
    session.commit()
    session.refresh(replay)
    return replay


def run_scoring(session: Session, replay: Replay, blob: bytes, scorer: Scorer) -> None:
    """Score a queued replay, persist its reports, and settle the credit.

    On any failure the replay is marked `failed` and the credit auto-refunded
    (§8) — the soft-hold never strands a credit.
    """
    replay.status = "scoring"
    session.add(replay)
    session.commit()
    try:
        result = scorer.score(blob, replay.id)
        for ps in result.players:
            session.add(
                Report(
                    replay_id=replay.id,
                    player_id=ps.player_id,
                    composite=ps.composite,
                    first_man=ps.first_man,
                    second_man=ps.second_man,
                    general=ps.general,
                    licence=ps.licence,
                    player_type=ps.player_type,
                    main_leak=ps.main_leak,
                    focus_chapter=ps.focus_chapter,
                    confidence=ps.confidence,
                    score_config_version=result.score_config_version,
                    parser_version=result.parser_version,
                    metrics_json=ps.metrics_json,
                )
            )
        replay.parser_version = result.parser_version
        replay.status = "done"
        session.add(replay)
        credits.confirm(session, replay.account_id, replay.id)
        # Recompute the leaderboard on this eligible write (§8): no-op unless the
        # account's locked profile got an ok-confidence report here.
        account = session.get(Account, replay.account_id)
        if account is not None:
            leaderboard.materialize(session, account, replay.id)
        session.commit()
    except Exception as e:  # noqa: BLE001 — any worker failure must refund.
        session.rollback()
        replay = session.get(Replay, replay.id)
        if replay is not None:
            replay.status = "failed"
            replay.error = str(e)[:500]
            session.add(replay)
            credits.refund(session, replay.account_id, replay.id)
            session.commit()


def process(replay_id: str, scorer: Scorer) -> None:
    """Background entry point: load the blob and score the replay."""
    with session_scope() as session:
        replay = session.get(Replay, replay_id)
        if replay is None:
            return
        blob = blobs.load(replay_id)
        run_scoring(session, replay, blob, scorer)


# Re-exported so `main` can reference the configured worker timeout in one place.
WORKER_TIMEOUT_S = settings.worker_timeout_s
