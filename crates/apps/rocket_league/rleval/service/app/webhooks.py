"""Purchase / billing entitlement (spec §8).

The grant side of the credit story (the ledger in `credits.py` owns spend and
refund). A billing provider (Stripe/PayPal) calls the webhook on purchase and on
each renewal; this grants credits and flips the `owns_book` entitlement.

Idempotent by the provider's event id (`WebhookEvent`), since webhooks redeliver.
Authenticating the caller (shared-secret / signature) is a deferred follow-up —
the route is namespaced `/internal/` and expected to sit behind that.
"""

from __future__ import annotations

from datetime import datetime, timedelta

from sqlmodel import Session, select

from . import credits
from .config import settings
from .models import Account, WebhookEvent, utcnow

# Grant kinds the webhook understands.
MONTHLY = "monthly"
TOPUP = "topup"

_TOPUP_AMOUNT = 10
_TOPUP_TTL_DAYS = 30


def _month_end(now: datetime) -> datetime:
    """Start of next month (UTC) — when a monthly grant expires (no rollover)."""
    start = now.replace(day=1, hour=0, minute=0, second=0, microsecond=0)
    return (
        start.replace(year=start.year + 1, month=1)
        if start.month == 12
        else start.replace(month=start.month + 1)
    )


def _account_for(session: Session, email: str) -> Account:
    account = session.exec(select(Account).where(Account.email == email)).first()
    if account is None:
        account = Account(email=email)
        session.add(account)
        session.flush()
    return account


def apply_purchase(
    session: Session,
    *,
    email: str,
    event_id: str,
    kind: str = MONTHLY,
    grant_book: bool = False,
    now: datetime | None = None,
) -> Account:
    """Apply a purchase/renewal event. Idempotent by ``event_id``.

    ``kind == "monthly"`` grants the monthly allotment expiring at period end;
    ``"topup"`` grants a smaller bundle expiring in 30 days (consumed after
    monthly credits, per the ledger's expiry ordering). ``grant_book`` flips the
    `owns_book` entitlement.
    """
    now = now or utcnow()
    account = _account_for(session, email)

    if session.get(WebhookEvent, event_id) is not None:
        return account  # already processed — no double grant

    if grant_book:
        account.owns_book = True
        session.add(account)

    if kind == MONTHLY:
        credits.grant(
            session,
            account.id,
            settings.monthly_grant,
            "monthly_grant",
            expires_at=_month_end(now),
        )
    elif kind == TOPUP:
        credits.grant(
            session,
            account.id,
            _TOPUP_AMOUNT,
            "topup",
            expires_at=now + timedelta(days=_TOPUP_TTL_DAYS),
        )

    session.add(WebhookEvent(id=event_id))
    session.commit()
    session.refresh(account)
    return account
