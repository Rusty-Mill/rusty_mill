"""Leaderboard materialization + reads (spec §5/§7/§8).

Integrity rules (mirroring the reference): only a **locked-profile** report with
``confidence == "ok"`` is eligible, and we keep the **single best composite per
account per season**. The materialized table is recomputed on each eligible write
(here: right after a successful score), so reads are a cheap ordered scan.
"""

from __future__ import annotations

from datetime import datetime, timezone

from sqlalchemy import func
from sqlmodel import Session, select

from .config import settings
from .models import Account, LeaderboardEntry, Report, utcnow


def current_season(now: datetime | None = None) -> str:
    """Calendar-quarter season key, e.g. ``2026-S2``."""
    now = now or utcnow()
    quarter = (now.month - 1) // 3 + 1
    return f"{now.year}-S{quarter}"


def season_bounds(season: str) -> tuple[datetime, datetime]:
    """``(start, end)`` UTC for a ``YYYY-Sn`` key; ``end`` is the next quarter's
    start (exclusive)."""
    year_s, q_s = season.split("-S")
    year, quarter = int(year_s), int(q_s)
    start_month = (quarter - 1) * 3 + 1
    start = datetime(year, start_month, 1, tzinfo=timezone.utc)
    end_month, end_year = start_month + 3, year
    if end_month > 12:
        end_month, end_year = end_month - 12, year + 1
    return start, datetime(end_year, end_month, 1, tzinfo=timezone.utc)


def is_closed(season: str, now: datetime | None = None) -> bool:
    """A season is closed once its quarter has elapsed; closed seasons never take
    new entries (materialize only ever writes the *current* season)."""
    return (now or utcnow()) >= season_bounds(season)[1]


def _maybe_grant_founding(session: Session, account: Account) -> None:
    """Assign the next Founding-N ordinal on an account's first qualification."""
    if account.founding_number is not None:
        return
    n_founders = session.exec(
        select(func.count(Account.id)).where(Account.founding_number.is_not(None))
    ).one()
    if n_founders < settings.founding_n:
        account.founding_number = n_founders + 1
        session.add(account)
        session.flush()


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

    _maybe_grant_founding(session, account)  # qualifying once is enough (§13)

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


def recent(
    session: Session, limit: int = 50, season: str | None = None
) -> list[LeaderboardEntry]:
    """Most recently-set personal bests (newest first), optionally per season."""
    q = select(LeaderboardEntry)
    if season is not None:
        q = q.where(LeaderboardEntry.season == season)
    q = q.order_by(LeaderboardEntry.uploaded_at.desc()).limit(limit)
    return list(session.exec(q).all())


def seasons(session: Session) -> list[str]:
    """Distinct seasons that have entries, newest first."""
    rows = session.exec(select(LeaderboardEntry.season).distinct()).all()
    return sorted(set(rows), reverse=True)
