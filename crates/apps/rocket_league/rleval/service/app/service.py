"""Domain flow: ingest a replay, then score it off-thread (spec §4/§9).

Pure-ish orchestration over the persistence + credit + scoring pieces. The HTTP
layer (`main.py`) calls [`ingest`] synchronously and schedules [`process`] as a
background task; both share the same engine, so the worker path is exercised in
tests too.
"""

from __future__ import annotations

import hashlib

from sqlmodel import Session, select

from . import blobs, credits, leaderboard, metrics
from .config import settings
from .db import session_scope
from .models import Account, LeaderboardEntry, Replay, Report
from .scoring import ScoreResult, Scorer


def _store_reports(session: Session, replay: Replay, result: ScoreResult) -> None:
    """Persist a [`ScoreResult`]'s per-player reports for a replay."""
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
        with metrics.SCORING_STAGE.time(stage="score"):
            result = scorer.score(blob, replay.id)
        with metrics.SCORING_STAGE.time(stage="persist"):
            if result.report_html:
                blobs.store_html(replay.id, result.report_html)  # for PDF rendering
            if result.canonical_blob:
                blobs.store_canonical(replay.id, result.canonical_blob)  # no-re-parse re-score
            _store_reports(session, replay, result)
            replay.parser_version = result.parser_version
            replay.status = "done"
            session.add(replay)
            credits.confirm(session, replay.account_id, replay.id)
        with metrics.SCORING_STAGE.time(stage="leaderboard"):
            # Recompute the leaderboard on this eligible write (§8): no-op unless the
            # account's locked profile got an ok-confidence report here.
            account = session.get(Account, replay.account_id)
            if account is not None:
                leaderboard.materialize(session, account, replay.id)
            session.commit()
        metrics.SCORED.inc(result="done")
    except Exception as e:  # noqa: BLE001 — any worker failure must refund.
        session.rollback()
        replay = session.get(Replay, replay.id)
        if replay is not None:
            replay.status = "failed"
            replay.error = str(e)[:500]
            session.add(replay)
            credits.refund(session, replay.account_id, replay.id)
            session.commit()
        metrics.SCORED.inc(result="failed")


def rescore(session: Session, replay: Replay, scorer: Scorer) -> ScoreResult:
    """Re-run scoring on a `done` replay and replace its stored reports (§8/§9).

    Re-runs the worker at its current `score_config_version` (deploy a newer
    worker → newer version) and swaps in the fresh reports, dropping the cached
    PDF and re-materializing the leaderboard. (The §9 "skip re-parse via a cached
    canonical blob" optimization is a separate follow-up; this re-parses.)
    """
    # Re-score from the cached canonical when available (no re-parse, §9);
    # otherwise re-parse the raw replay.
    if blobs.has_canonical(replay.id):
        result = scorer.score_from_canonical(blobs.load_canonical(replay.id), replay.id)
    else:
        result = scorer.score(blobs.load(replay.id), replay.id)
    if result.report_html:
        blobs.store_html(replay.id, result.report_html)

    old = list(session.exec(select(Report).where(Report.replay_id == replay.id)).all())
    old_ids = [r.id for r in old]
    # No FK cascade in SQLite: drop leaderboard entries pointing at replaced
    # reports before deleting them, then re-materialize from the new report.
    if old_ids:
        for entry in session.exec(
            select(LeaderboardEntry).where(LeaderboardEntry.report_id.in_(old_ids))
        ).all():
            session.delete(entry)
    for r in old:
        session.delete(r)
    session.flush()

    _store_reports(session, replay, result)
    replay.parser_version = result.parser_version
    session.add(replay)
    session.flush()

    account = session.get(Account, replay.account_id)
    if account is not None:
        leaderboard.materialize(session, account, replay.id)
    blobs.delete_pdf(replay.id)  # stale: the report changed
    session.commit()
    return result


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
