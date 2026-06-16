"""Leaderboard materialization + reads (spec §5/§7/§8).

Integrity rules (mirroring the reference): only a **locked-profile** report with
``confidence == "ok"`` is eligible, and we keep the **single best composite per
account per season**. The materialized table is recomputed on each eligible write
(here: right after a successful score), so reads are a cheap ordered scan.
"""

from __future__ import annotations

from datetime import datetime

from sqlmodel import Session, select

from .models import Account, LeaderboardEntry, Report, utcnow


def current_season(now: datetime | None = None) -> str:
    """Calendar-quarter season key, e.g. ``2026-S2``."""
    now = now or utcnow()
    quarter = (now.month - 1) // 3 + 1
    return f"{now.year}-S{quarter}"


def materialize(
    session: Session,
    account: Account,
    replay_id: str,
    *,
    now: datetime | None = None,
) -> LeaderboardEntry | None:
    """Upsert the account's best entry for the current season from this replay.

    No-op unless the account has a locked profile and the replay produced an
    ``ok``-confidence report for it. Keeps the higher composite if one already
    exists this season.
    """
    if not account.locked_player_id:
        return None
    report = session.exec(
        select(Report).where(
            Report.replay_id == replay_id,
            Report.player_id == account.locked_player_id,
        )
    ).first()
    if report is None or report.confidence != "ok":
        return None  # not eligible (no locked-profile report, or low confidence)

    season = current_season(now)
    existing = session.exec(
        select(LeaderboardEntry).where(
            LeaderboardEntry.account_id == account.id,
            LeaderboardEntry.season == season,
        )
    ).first()

    if existing is not None and report.composite <= existing.composite:
        return existing  # keep the standing best

    target = existing or LeaderboardEntry(account_id=account.id, season=season)
    target.player_id = report.player_id
    target.report_id = report.id
    target.composite = report.composite
    target.first_man = report.first_man
    target.second_man = report.second_man
    target.general = report.general
    target.uploaded_at = report.created_at
    session.add(target)
    session.flush()
    return target


def top(session: Session, season: str, limit: int = 50) -> list[LeaderboardEntry]:
    return list(
        session.exec(
            select(LeaderboardEntry)
            .where(LeaderboardEntry.season == season)
            .order_by(LeaderboardEntry.composite.desc())
            .limit(limit)
        ).all()
    )
