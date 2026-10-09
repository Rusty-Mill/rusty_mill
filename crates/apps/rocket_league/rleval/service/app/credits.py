"""Credit ledger logic (spec §8).

Append-only ledger; **balance = sum of non-expired deltas**. The spend lifecycle
is a soft-hold at enqueue → confirm on success → auto-refund on failure, and is
**idempotent by replay_id** so a retry never double-charges or double-refunds.

Expiry bookkeeping: a hold consumes the soonest-expiring grant first (monthly
before top-up, per §8) and inherits that grant's expiry; a refund inherits the
hold's expiry. That keeps a grant and the spends/refunds against it expiring
*together*, so an expired monthly grant leaves no negative drift and no rollover.
"""

from __future__ import annotations

from datetime import datetime
from datetime import timezone

from sqlmodel import Session, select

from .models import CreditLedger, utcnow


def _active(e: CreditLedger, now: datetime) -> bool:
    if e.expires_at is None:
        return True
    # SQLite drops tzinfo on round-trip; treat a naive stored expiry as UTC so it
    # compares cleanly against the tz-aware `now`.
    exp = e.expires_at
    if exp.tzinfo is None:
        exp = exp.replace(tzinfo=timezone.utc)
    return exp > now


def _ledger(session: Session, account_id: int) -> list[CreditLedger]:
    return list(
        session.exec(
            select(CreditLedger).where(CreditLedger.account_id == account_id)
        ).all()
    )


def balance(session: Session, account_id: int, now: datetime | None = None) -> int:
    now = now or utcnow()
    return sum(e.delta for e in _ledger(session, account_id) if _active(e, now))


def grant(
    session: Session,
    account_id: int,
    delta: int,
    reason: str,
    *,
    expires_at: datetime | None = None,
    replay_id: str | None = None,
    status: str = "settled",
) -> CreditLedger:
    entry = CreditLedger(
        account_id=account_id,
        delta=delta,
        reason=reason,
        expires_at=expires_at,
        replay_id=replay_id,
        status=status,
    )
    session.add(entry)
    session.flush()
    return entry


def _entry(
    session: Session, account_id: int, replay_id: str, reason: str
) -> CreditLedger | None:
    return session.exec(
        select(CreditLedger).where(
            CreditLedger.account_id == account_id,
            CreditLedger.replay_id == replay_id,
            CreditLedger.reason == reason,
        )
    ).first()


def _hold_expiry(session: Session, account_id: int, now: datetime) -> datetime | None:
    """Expiry of the soonest-expiring active grant — the bucket a hold consumes."""
    expiries = [
        e.expires_at
        for e in _ledger(session, account_id)
        if e.delta > 0 and _active(e, now) and e.expires_at is not None
    ]
    return min(expiries, default=None)


def hold(
    session: Session, account_id: int, replay_id: str, now: datetime | None = None
) -> CreditLedger | None:
    """Soft-hold one credit for a replay. Idempotent; ``None`` if no balance."""
    now = now or utcnow()
    existing = _entry(session, account_id, replay_id, "spend")
    if existing is not None:
        return existing
    if balance(session, account_id, now) <= 0:
        return None
    return grant(
        session,
        account_id,
        -1,
        "spend",
        expires_at=_hold_expiry(session, account_id, now),
        replay_id=replay_id,
        status="pending",
    )


def confirm(session: Session, account_id: int, replay_id: str) -> None:
    """Settle the soft-hold once scoring succeeds (no balance change)."""
    spend = _entry(session, account_id, replay_id, "spend")
    if spend is not None and spend.status == "pending":
        spend.status = "settled"
        session.add(spend)
        session.flush()


def refund(
    session: Session, account_id: int, replay_id: str
) -> CreditLedger | None:
    """Auto-refund the spend on failure (net-zero). Idempotent by replay_id."""
    spend = _entry(session, account_id, replay_id, "spend")
    if spend is None:
        return None
    already = _entry(session, account_id, replay_id, "refund")
    if already is not None:
        return already
    spend.status = "settled"
    session.add(spend)
    # Inherit the hold's expiry so the spend/refund pair drops off with its grant.
    return grant(
        session,
        account_id,
        +1,
        "refund",
        expires_at=spend.expires_at,
        replay_id=replay_id,
    )
